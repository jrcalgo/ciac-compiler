//! v0.18 M3: `ciac diff --semantic`'s CLI surface, exercised through the
//! real binary. Mirrors `baseline_cli.rs`'s spawn pattern — no-op,
//! breaking-change detection with `--deny-breaking`, `--json` shape,
//! `--against` mode, `--format markdown`, and the mutual-exclusion
//! checks (`--against`+`--baseline`, `--semantic`+`--target`/`--out`).

use std::process::Command;

fn ciac() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ciac"))
}

const SRC_V1: &str = r#"
service DiffTest;
use { db Postgres; }

record Video {
    id: Uuid;
    title: String;
}
table Videos: Video;

api GetVideo: Video { method: POST; path: "/videos"; }
pipeline GetVideo: Return;
"#;

const SRC_V2_BREAKING: &str = r#"
service DiffTest;
use { db Postgres; }

record Video {
    id: Uuid;
}
table Videos: Video;

api GetVideo: Video { method: POST; path: "/videos"; }
pipeline GetVideo: Return;
"#;

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ciac-diff-semantic-cli-test-{}-{name}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn make_baseline(entry: &std::path::Path, out: &std::path::Path) {
    let created = ciac()
        .args([
            "baseline",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(created.status.success(), "{created:?}");
}

#[test]
fn semantic_diff_against_unchanged_baseline_is_a_clean_no_op() {
    let tmp = TempDir::new("noop");
    let entry = tmp.0.join("main.ciac");
    let baseline = tmp.0.join("main.semantic.json");
    std::fs::write(&entry, SRC_V1).expect("write entry");
    make_baseline(&entry, &baseline);

    let result = ciac()
        .args([
            "diff",
            entry.to_str().unwrap(),
            "--semantic",
            "--baseline",
            baseline.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("ciac runs");
    assert!(result.status.success(), "{result:?}");
    let doc: serde_json::Value = serde_json::from_slice(&result.stdout).expect("valid json");
    assert_eq!(doc["semantic"]["summary"]["breaking"], 0);
    assert_eq!(doc["semantic"]["summary"]["additive"], 0);
    assert_eq!(doc["semantic"]["summary"]["internal"], 0);
    assert_eq!(doc["semantic"]["changes"].as_array().unwrap().len(), 0);
}

#[test]
fn semantic_diff_deny_breaking_fails_on_a_removed_field() {
    let tmp = TempDir::new("deny-breaking");
    let entry = tmp.0.join("main.ciac");
    let baseline = tmp.0.join("main.semantic.json");
    std::fs::write(&entry, SRC_V1).expect("write entry");
    make_baseline(&entry, &baseline);

    std::fs::write(&entry, SRC_V2_BREAKING).expect("write breaking change");
    let result = ciac()
        .args([
            "diff",
            entry.to_str().unwrap(),
            "--semantic",
            "--baseline",
            baseline.to_str().unwrap(),
            "--deny-breaking",
            "--json",
        ])
        .output()
        .expect("ciac runs");
    assert!(
        !result.status.success(),
        "a removed field must fail --deny-breaking"
    );
    let doc: serde_json::Value = serde_json::from_slice(&result.stdout).expect("valid json");
    assert!(doc["semantic"]["summary"]["breaking"].as_u64().unwrap() >= 1);
    assert_eq!(doc["semantic"]["policy"]["deny_breaking"], true);
    assert_eq!(doc["semantic"]["policy"]["passed"], false);

    // Without --deny-breaking, the same breaking change is reported but
    // the invocation itself still succeeds (a policy failure is visible
    // in `policy.passed`, never silently folded into `success`).
    let visible_only = ciac()
        .args([
            "diff",
            entry.to_str().unwrap(),
            "--semantic",
            "--baseline",
            baseline.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("ciac runs");
    assert!(visible_only.status.success(), "{visible_only:?}");
    let doc: serde_json::Value = serde_json::from_slice(&visible_only.stdout).expect("valid json");
    assert!(doc["semantic"]["summary"]["breaking"].as_u64().unwrap() >= 1);
    assert_eq!(doc["semantic"]["policy"]["deny_breaking"], false);
}

#[test]
fn semantic_diff_against_a_second_file_and_markdown_format() {
    let tmp = TempDir::new("against-markdown");
    let before = tmp.0.join("before.ciac");
    let after = tmp.0.join("after.ciac");
    std::fs::write(&before, SRC_V1).expect("write before");
    std::fs::write(&after, SRC_V2_BREAKING).expect("write after");

    let result = ciac()
        .args([
            "diff",
            after.to_str().unwrap(),
            "--semantic",
            "--against",
            before.to_str().unwrap(),
            "--format",
            "markdown",
        ])
        .output()
        .expect("ciac runs");
    assert!(result.status.success(), "{result:?}");
    let text = String::from_utf8(result.stdout).expect("utf8 output");
    assert!(text.contains('|'), "markdown format should render a table");
}

#[test]
fn semantic_and_regeneration_diff_modes_are_mutually_exclusive() {
    let tmp = TempDir::new("mutex-out-target");
    let entry = tmp.0.join("main.ciac");
    std::fs::write(&entry, SRC_V1).expect("write entry");

    let result = ciac()
        .args([
            "diff",
            entry.to_str().unwrap(),
            "--semantic",
            "--target",
            "python",
            "--out",
            tmp.0.join("out").to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(
        !result.status.success(),
        "--semantic with --target/--out must be refused"
    );
}

#[test]
fn against_and_baseline_are_mutually_exclusive() {
    let tmp = TempDir::new("mutex-against-baseline");
    let entry = tmp.0.join("main.ciac");
    let other = tmp.0.join("other.ciac");
    std::fs::write(&entry, SRC_V1).expect("write entry");
    std::fs::write(&other, SRC_V1).expect("write other");

    let result = ciac()
        .args([
            "diff",
            entry.to_str().unwrap(),
            "--semantic",
            "--against",
            other.to_str().unwrap(),
            "--baseline",
            other.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(
        !result.status.success(),
        "--against and --baseline together must be refused by clap"
    );
}

#[test]
fn plain_regeneration_diff_is_unaffected_by_the_semantic_mode() {
    let tmp = TempDir::new("regen-unaffected");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("out");
    std::fs::write(&entry, SRC_V1).expect("write entry");

    let build = ciac()
        .args([
            "build",
            entry.to_str().unwrap(),
            "-t",
            "python",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(build.status.success(), "{build:?}");

    let result = ciac()
        .args([
            "diff",
            entry.to_str().unwrap(),
            "-t",
            "python",
            "-o",
            out.to_str().unwrap(),
            "--json",
        ])
        .output()
        .expect("ciac runs");
    assert!(result.status.success(), "{result:?}");
    let doc: serde_json::Value = serde_json::from_slice(&result.stdout).expect("valid json");
    assert!(doc.get("semantic").is_none() || doc["semantic"].is_null());
    // "orphan" (a migration file the manifest doesn't track for
    // removal) is a pre-existing benign steady state, same as
    // "unchanged" — this assertion only checks that nothing needing
    // attention (update/conflict/seeded-drift) appeared, i.e. that nothing
    // about the semantic-diff mode's addition leaked into this path.
    let entries = doc["entries"].as_array().expect("regen entries present");
    assert!(entries
        .iter()
        .all(|e| matches!(e["status"].as_str(), Some("unchanged") | Some("orphan"))));
}
