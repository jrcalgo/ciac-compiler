//! v0.12 M3: `registry:` blueprint imports, proven against a real
//! HTTP server (`python3 -m http.server` over a git-repo-shaped
//! directory, the same technique as the OAuth2 JWKS live proof) —
//! then proven *offline*: the server is stopped and a fresh load must
//! resolve entirely from the on-disk cache.
//!
//! Everything env-dependent (CIAC_REGISTRY, XDG_CACHE_HOME) lives in
//! this one serial test so no parallel test observes the mutation.

use ciac_diagnostics::{Diagnostics, SourceMap};
use ciac_syntax::ast::Item;
use std::io::Read;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ciac-registry-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
}

/// Picks a port by binding to :0 and releasing it — a small window
/// for reuse races, acceptable for a test that owns the whole run.
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_until_serving(port: u16) {
    for _ in 0..100 {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("http.server never came up on port {port}");
}

fn load(
    entry: &Path,
) -> (
    Result<ciac_syntax::ast::Program, std::io::Error>,
    Diagnostics,
) {
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::load(entry, &mut sources, &mut diags);
    (program, diags)
}

#[test]
fn registry_import_fetches_caches_and_answers_offline() {
    // A git-repo-shaped registry: {owner}/{repo}/{ref}/{path}.
    let root = temp_dir("root");
    write(
        &root.join("acme/blueprints/v1/notes/record.ciac"),
        "record Note {\n    id: Uuid;\n    body: String;\n}\n",
    );

    let port = free_port();
    let server = Server(
        Command::new("python3")
            .args([
                "-m",
                "http.server",
                &port.to_string(),
                "--bind",
                "127.0.0.1",
                "--directory",
            ])
            .arg(&root)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("python3 http.server starts"),
    );
    wait_until_serving(port);

    let cache = temp_dir("cache");
    // Safe under edition 2021; confined to this single test binary's
    // single env-touching test.
    std::env::set_var("CIAC_REGISTRY", format!("http://127.0.0.1:{port}"));
    std::env::set_var("XDG_CACHE_HOME", &cache);

    let project = temp_dir("project");
    let entry = project.join("main.ciac");
    write(
        &entry,
        "service Notes;\n\
         import \"registry:acme/blueprints/notes/record.ciac@v1\";\n\
         stream Saved: Note;\n",
    );

    // 1. Online: the import resolves over real HTTP.
    let (program, diags) = load(&entry);
    let program = program.expect("resolves over HTTP");
    assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
    assert!(
        program
            .items
            .iter()
            .any(|item| matches!(item, Item::Record(r) if r.name.text == "Note")),
        "the fetched record should be spliced in"
    );

    // 2. The fetch landed in the cache.
    let cached: Vec<PathBuf> = walk(&cache.join("ciac/registry"));
    assert_eq!(cached.len(), 1, "exactly one cached blueprint: {cached:?}");
    let mut cached_src = String::new();
    std::fs::File::open(&cached[0])
        .unwrap()
        .read_to_string(&mut cached_src)
        .unwrap();
    assert!(cached_src.contains("record Note"), "{cached_src}");

    // 3. Offline: stop the server; a fresh load must succeed purely
    //    from the cache.
    drop(server);
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "server must actually be down for the offline proof"
    );
    let (program, diags) = load(&entry);
    let program = program.expect("resolves from cache with the server gone");
    assert!(diags.is_empty(), "unexpected: {:?}", diags.codes());
    assert!(program
        .items
        .iter()
        .any(|item| matches!(item, Item::Record(r) if r.name.text == "Note")));

    // 4. An unknown path (never cached) fails with a clear error
    //    naming the spec — and, offline, the transport failure.
    let missing = project.join("missing.ciac");
    write(
        &missing,
        "service X;\nimport \"registry:acme/blueprints/nope.ciac@v1\";\n",
    );
    let (result, _) = load(&missing);
    let err = result.expect_err("unknown blueprint cannot resolve");
    let msg = err.to_string();
    assert!(
        msg.contains("registry:acme/blueprints/nope.ciac@v1"),
        "{msg}"
    );

    // 5. A malformed spec (no @ref) is rejected before any I/O.
    let unpinned = project.join("unpinned.ciac");
    write(
        &unpinned,
        "service X;\nimport \"registry:acme/blueprints/notes/record.ciac\";\n",
    );
    let (result, _) = load(&unpinned);
    let err = result.expect_err("unpinned spec is invalid");
    assert!(err.to_string().contains("@<ref>"), "{}", err);

    std::env::remove_var("CIAC_REGISTRY");
    std::env::remove_var("XDG_CACHE_HOME");
    for dir in [root, cache, project] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}
