//! v0.13 M5: `ciac mcp`, exercised as an MCP client would — newline-
//! delimited JSON-RPC over the spawned binary's stdio (the MCP stdio
//! transport, distinct from `ciac lsp`'s Content-Length framing).
//! Round-trips `initialize`, `tools/list`, and `tools/call` for
//! `describe`, `check`, and `explain`, then closes stdin and checks
//! the server exits cleanly.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
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
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
}

impl Server {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ciac"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("ciac mcp starts");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Server {
            child,
            stdin: Some(stdin),
            stdout,
        }
    }

    fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().expect("stdin still open");
        writeln!(stdin, "{}", serde_json::to_string(&message).unwrap()).expect("write");
        stdin.flush().expect("flush");
    }

    fn recv(&mut self) -> Value {
        let mut line = String::new();
        self.stdout.read_line(&mut line).expect("response line");
        serde_json::from_str(line.trim_end()).expect("line is JSON")
    }
}

#[test]
fn mcp_round_trip_initialize_tools_list_and_tool_calls() {
    let mut server = Server::start();

    server.send(json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "protocolVersion": "2024-11-05", "capabilities": {} }
    }));
    let init = server.recv();
    assert_eq!(init["id"], json!(1));
    assert!(init["result"]["protocolVersion"].is_string(), "{init}");
    assert_eq!(init["result"]["serverInfo"]["name"], "ciac");

    server.send(json!({
        "jsonrpc": "2.0", "method": "notifications/initialized", "params": {}
    }));

    server.send(json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }));
    let listed = server.recv();
    let tools = listed["result"]["tools"].as_array().expect("tools array");
    let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    for expected in [
        "check", "build", "diff", "verify", "graph", "explain", "describe", "fix",
    ] {
        assert!(
            names.contains(&expected),
            "missing tool `{expected}`: {names:?}"
        );
    }

    // `describe`: a versioned document naming the provider registry.
    server.send(json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "describe", "arguments": {} }
    }));
    let described = server.recv();
    assert_eq!(described["result"]["isError"], json!(false));
    let text = described["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    let doc: Value = serde_json::from_str(text).expect("describe text is JSON");
    assert!(doc["describe_version"].is_u64(), "{doc}");
    assert!(
        doc["providers"]
            .as_array()
            .expect("providers array")
            .iter()
            .any(|p| p["name"] == "Kafka"),
        "{doc}"
    );

    // `check`: the same envelope `ciac check --json` prints, on a
    // program known to compile clean.
    let ping = fixture("examples/ping.ciac");
    server.send(json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": { "name": "check", "arguments": { "file": ping.to_str().unwrap() } }
    }));
    let checked = server.recv();
    assert_eq!(checked["result"]["isError"], json!(false));
    let text = checked["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    let envelope: Value = serde_json::from_str(text).expect("check text is JSON");
    assert_eq!(envelope["command"], "check");
    assert_eq!(envelope["success"], json!(true), "{envelope}");

    // `explain`: the same text `ciac explain CIAC0006` prints.
    server.send(json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": { "name": "explain", "arguments": { "code": "CIAC0006" } }
    }));
    let explained = server.recv();
    let text = explained["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    assert!(text.starts_with("CIAC0006:"), "{text}");
    assert!(text.contains("cycle"), "{text}");

    // `fix` (v0.15 M7): dry-run by default (preview only, no write),
    // then applied and re-checked -- on a scratch copy, never the
    // checked-in fixture.
    let scratch_dir = std::env::temp_dir().join(format!("ciac-mcp-fix-{}", std::process::id()));
    std::fs::create_dir_all(&scratch_dir).expect("scratch dir");
    let scratch_file = scratch_dir.join("missing-scheduler.ciac");
    std::fs::copy(
        fixture("tests/ui/missing-scheduler-with-use-block.ciac"),
        &scratch_file,
    )
    .expect("copy fixture to scratch");

    server.send(json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": { "name": "fix", "arguments": {
            "file": scratch_file.to_str().unwrap(), "code": "CIAC0005",
        } }
    }));
    let previewed = server.recv();
    assert_eq!(previewed["result"]["isError"], json!(false));
    let text = previewed["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    let doc: Value = serde_json::from_str(text).expect("fix preview text is JSON");
    assert_eq!(doc["applied"], json!(false), "{doc}");
    assert!(
        doc["patched_source"]
            .as_str()
            .expect("patched_source")
            .contains("scheduler Cron;"),
        "{doc}"
    );
    // Dry-run must not have touched the file on disk.
    assert!(!std::fs::read_to_string(&scratch_file)
        .unwrap()
        .contains("scheduler Cron;"));

    server.send(json!({
        "jsonrpc": "2.0", "id": 8, "method": "tools/call",
        "params": { "name": "fix", "arguments": {
            "file": scratch_file.to_str().unwrap(), "code": "CIAC0005", "apply": true,
        } }
    }));
    let applied = server.recv();
    assert_eq!(applied["result"]["isError"], json!(false));
    let text = applied["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    let doc: Value = serde_json::from_str(text).expect("fix apply text is JSON");
    assert_eq!(doc["applied"], json!(true), "{doc}");
    assert_eq!(doc["recheck"]["success"], json!(true), "{doc}");
    assert!(std::fs::read_to_string(&scratch_file)
        .unwrap()
        .contains("scheduler Cron;"));
    std::fs::remove_dir_all(&scratch_dir).ok();

    // An unknown tool comes back as a tool-level error, not a
    // JSON-RPC protocol error.
    server.send(json!({
        "jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": { "name": "not-a-tool", "arguments": {} }
    }));
    let unknown = server.recv();
    assert_eq!(unknown["result"]["isError"], json!(true), "{unknown}");

    // Closing stdin ends the read loop; the server must exit cleanly.
    drop(server.stdin.take());
    let status = server.child.wait().expect("server exits");
    assert!(status.success(), "clean exit after stdin closes");
}
