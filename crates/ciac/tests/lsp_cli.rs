//! v0.12 M2: `ciac lsp`, exercised as an editor would — raw JSON-RPC
//! with Content-Length framing over the spawned binary's stdio. The
//! load-bearing assertions: didOpen publishes the same CIAC0005 the
//! CLI reports for the fixture, at the right (0-based) line; hover
//! and completion answer from the static vocabulary and the file's
//! harvested declarations.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

fn fixture(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
        .canonicalize()
        .expect("fixture exists")
}

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Server {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ciac"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("ciac lsp starts");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Server {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, message: Value) {
        let body = serde_json::to_string(&message).expect("serializes");
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).expect("write");
        self.stdin.flush().expect("flush");
    }

    fn recv(&mut self) -> Value {
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).expect("header line");
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(len) = line.strip_prefix("Content-Length: ") {
                content_length = len.parse().expect("length parses");
            }
        }
        let mut body = vec![0u8; content_length];
        self.stdout.read_exact(&mut body).expect("body");
        serde_json::from_slice(&body).expect("body is JSON")
    }

    /// Reads messages until the response with the given id arrives,
    /// ignoring interleaved notifications.
    fn response(&mut self, id: u64) -> Value {
        loop {
            let msg = self.recv();
            if msg["id"] == json!(id) {
                return msg;
            }
        }
    }

    /// Reads messages until a notification with the given method arrives.
    fn notification(&mut self, method: &str) -> Value {
        loop {
            let msg = self.recv();
            if msg["method"] == json!(method) {
                return msg;
            }
        }
    }
}

#[test]
fn lsp_round_trip_diagnostics_hover_and_completion() {
    let path = fixture("tests/ui/missing-queue.ciac");
    let uri = format!("file://{}", path.display());
    let text = std::fs::read_to_string(&path).expect("fixture readable");

    let mut server = Server::start();

    // initialize / initialized
    server.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let init = server.response(1);
    let caps = &init["result"]["capabilities"];
    assert!(caps["hoverProvider"].as_bool().unwrap_or(false), "{caps}");
    assert!(caps["completionProvider"].is_object(), "{caps}");
    assert!(
        caps["codeActionProvider"].as_bool().unwrap_or(false),
        "{caps}"
    );
    server.send(json!({
        "jsonrpc": "2.0", "method": "initialized", "params": {}
    }));

    // didOpen -> publishDiagnostics with the CLI's CIAC0005, 0-based.
    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "ciac", "version": 1, "text": text
        }}
    }));
    let published = server.notification("textDocument/publishDiagnostics");
    assert_eq!(published["params"]["uri"], json!(uri));
    let diags = published["params"]["diagnostics"]
        .as_array()
        .expect("diagnostics array");
    assert!(!diags.is_empty());
    let first = &diags[0];
    assert_eq!(first["code"], "CIAC0005");
    assert_eq!(first["severity"], 1, "LSP error severity");
    assert_eq!(
        first["range"]["start"]["line"], 3,
        "the `Queue` step sits on 1-based line 4 = 0-based line 3"
    );

    // Hover a keyword (`pipeline`, line 4 col 1 -> 0-based 3:2).
    server.send(json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 3, "character": 2 }
        }
    }));
    let hover = server.response(2);
    let value = hover["result"]["contents"]["value"]
        .as_str()
        .expect("markdown hover");
    assert!(value.contains("pipeline"), "{value}");

    // Hover a builtin step (`Queue` at 0-based 3:21).
    server.send(json!({
        "jsonrpc": "2.0", "id": 3, "method": "textDocument/hover",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 3, "character": 21 }
        }
    }));
    let hover = server.response(3);
    let value = hover["result"]["contents"]["value"]
        .as_str()
        .expect("markdown hover");
    assert!(value.contains("queue"), "{value}");

    // Completion: static vocabulary plus the file's own declarations
    // (the fixture parses despite the sema error, so `A` is harvested).
    server.send(json!({
        "jsonrpc": "2.0", "id": 4, "method": "textDocument/completion",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 0 }
        }
    }));
    let completion = server.response(4);
    let items = completion["result"].as_array().expect("completion array");
    let labels: Vec<&str> = items.iter().filter_map(|i| i["label"].as_str()).collect();
    for expected in ["service", "db", "Kafka", "Return", "A"] {
        assert!(
            labels.contains(&expected),
            "missing `{expected}`: {labels:?}"
        );
    }

    // shutdown / exit — the server must terminate cleanly.
    server.send(json!({
        "jsonrpc": "2.0", "id": 5, "method": "shutdown", "params": null
    }));
    server.response(5);
    server.send(json!({ "jsonrpc": "2.0", "method": "exit", "params": null }));
    let status = server.child.wait().expect("server exits");
    assert!(status.success(), "clean exit after shutdown");
}

/// v0.18 M7: `textDocument/prepareRename` and `textDocument/rename` over
/// the same whole-program resolver `ciac rename` uses. Reads from disk
/// (no VFS, same as diagnostics), so the fixture is a real temp file
/// rather than a checked-in one.
#[test]
fn lsp_rename_round_trip() {
    const SRC: &str = "service Billing;\nuse { db Postgres; }\nrecord Video {\n    id: Uuid;\n    title: String;\n}\ntable Videos: Video;\napi Create: Video { method: POST; path: \"/videos\"; }\nhandler CreateHandler(v: Video) -> Video {\n    db.insert(Videos, v);\n    return v;\n}\npipeline Create: CreateHandler -> Return;\n";

    let dir = std::env::temp_dir().join(format!("ciac-lsp-rename-cli-test-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let path = dir.join("main.ciac");
    std::fs::write(&path, SRC).expect("write fixture");
    let uri = format!("file://{}", path.display());

    let mut server = Server::start();
    server.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    let init = server.response(1);
    let caps = &init["result"]["capabilities"];
    assert_eq!(
        caps["renameProvider"]["prepareProvider"],
        json!(true),
        "{caps}"
    );
    server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));

    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "ciac", "version": 1, "text": SRC
        }}
    }));
    server.notification("textDocument/publishDiagnostics");

    // `record Video {` -- "Video" starts at 0-based line 2, character 7.
    server.send(json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/prepareRename",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 7 }
        }
    }));
    let prepared = server.response(2);
    assert_eq!(
        prepared["result"]["placeholder"],
        json!("Video"),
        "{prepared}"
    );
    assert_eq!(
        prepared["result"]["range"]["start"],
        json!({"line": 2, "character": 7})
    );
    assert_eq!(
        prepared["result"]["range"]["end"],
        json!({"line": 2, "character": 12})
    );

    server.send(json!({
        "jsonrpc": "2.0", "id": 3, "method": "textDocument/rename",
        "params": {
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 7 },
            "newName": "Clip"
        }
    }));
    let renamed = server.response(3);
    let edits = renamed["result"]["changes"][&uri]
        .as_array()
        .unwrap_or_else(|| panic!("workspace edit for this document: {renamed}"));
    assert_eq!(edits.len(), 5, "{edits:?}");
    assert!(edits.iter().all(|e| e["newText"] == json!("Clip")));

    // Every edit applies cleanly against the original text -- confirms
    // the ranges the server computed are actually correct, not just
    // present. Applied right-to-left within each line so earlier
    // column offsets on the same line stay valid.
    let mut lines: Vec<String> = SRC.lines().map(str::to_owned).collect();
    let mut sorted_edits: Vec<&Value> = edits.iter().collect();
    sorted_edits.sort_by_key(|e| {
        (
            e["range"]["start"]["line"].as_i64().unwrap(),
            -(e["range"]["start"]["character"].as_i64().unwrap()),
        )
    });
    for e in sorted_edits {
        let line = e["range"]["start"]["line"].as_i64().unwrap() as usize;
        let start = e["range"]["start"]["character"].as_i64().unwrap() as usize;
        let end = e["range"]["end"]["character"].as_i64().unwrap() as usize;
        lines[line].replace_range(start..end, e["newText"].as_str().unwrap());
    }
    let patched = lines.join("\n") + "\n";
    assert!(patched.contains("record Clip {"), "{patched}");
    assert!(patched.contains("table Videos: Clip;"), "{patched}");
    assert!(
        patched.contains("api Create: Clip { method: POST; path: \"/videos\"; }"),
        "{patched}"
    );
    assert!(
        patched.contains("handler CreateHandler(v: Clip) -> Clip {"),
        "{patched}"
    );

    server.send(json!({
        "jsonrpc": "2.0", "id": 5, "method": "shutdown", "params": null
    }));
    server.response(5);
    server.send(json!({ "jsonrpc": "2.0", "method": "exit", "params": null }));
    let status = server.child.wait().expect("server exits");
    assert!(status.success(), "clean exit after shutdown");

    std::fs::remove_dir_all(&dir).ok();
}

/// v0.15 M7: a diagnostic's fix rides the LSP `data` field from
/// `publishDiagnostics` to `codeAction` -- the same edits `--json`/MCP
/// expose, resolved into a `WorkspaceEdit` a client applies directly.
#[test]
fn lsp_code_action_offers_a_missing_capability_fix() {
    let path = fixture("tests/ui/missing-scheduler-with-use-block.ciac");
    let uri = format!("file://{}", path.display());
    let text = std::fs::read_to_string(&path).expect("fixture readable");

    let mut server = Server::start();
    server.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "capabilities": {} }
    }));
    server.response(1);
    server.send(json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }));

    server.send(json!({
        "jsonrpc": "2.0", "method": "textDocument/didOpen",
        "params": { "textDocument": {
            "uri": uri, "languageId": "ciac", "version": 1, "text": text
        }}
    }));
    let published = server.notification("textDocument/publishDiagnostics");
    let diags = published["params"]["diagnostics"]
        .as_array()
        .expect("diagnostics array");
    let diag = diags
        .iter()
        .find(|d| d["code"] == "CIAC0005")
        .expect("CIAC0005 reported");
    assert!(diag["data"].is_array(), "fix data attached: {diag}");

    server.send(json!({
        "jsonrpc": "2.0", "id": 2, "method": "textDocument/codeAction",
        "params": {
            "textDocument": { "uri": uri },
            "range": diag["range"],
            "context": { "diagnostics": [diag] },
        }
    }));
    let response = server.response(2);
    let actions = response["result"].as_array().expect("code action array");
    assert!(!actions.is_empty(), "{response}");
    let action = &actions[0];
    assert_eq!(action["kind"], "quickfix");
    let edits = action["edit"]["changes"][&uri]
        .as_array()
        .expect("workspace edit for this document");
    assert!(
        edits.iter().any(|e| e["newText"]
            .as_str()
            .unwrap_or_default()
            .contains("scheduler Cron;")),
        "{edits:?}"
    );

    server.send(json!({
        "jsonrpc": "2.0", "id": 5, "method": "shutdown", "params": null
    }));
    server.response(5);
    server.send(json!({ "jsonrpc": "2.0", "method": "exit", "params": null }));
    let status = server.child.wait().expect("server exits");
    assert!(status.success(), "clean exit after shutdown");
}
