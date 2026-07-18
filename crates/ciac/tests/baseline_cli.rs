//! v0.18 M1: `ciac baseline`'s lifecycle, exercised through the real
//! binary — create/no-op/update/accept-breaking and future-schema
//! refusal. Mirrors `json_cli.rs`'s spawn pattern.

use std::process::Command;

fn ciac() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ciac"))
}

const SRC_V1: &str = r#"
service BaselineTest;
use { db Postgres; }

record Video {
    id: Uuid;
    title: String;
}
table Videos: Video;
"#;

const SRC_V2: &str = r#"
service BaselineTest;
use { db Postgres; }

record Video {
    id: Uuid;
    title: String;
    summary: String;
}
table Videos: Video;
"#;

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ciac-baseline-cli-test-{}-{name}",
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

#[test]
fn baseline_lifecycle_is_deterministic() {
    let tmp = TempDir::new("lifecycle");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("main.semantic.json");
    std::fs::write(&entry, SRC_V1).expect("write entry");

    // First creation succeeds.
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
    let bytes_after_create = std::fs::read(&out).expect("baseline written");

    // Identical recreation is a true no-op: byte-identical, not merely
    // logically unchanged.
    let recreated = ciac()
        .args([
            "baseline",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(recreated.status.success(), "{recreated:?}");
    let bytes_after_recreate = std::fs::read(&out).expect("baseline still there");
    assert_eq!(
        bytes_after_create, bytes_after_recreate,
        "an unchanged recreation must not rewrite the file at all"
    );

    // A real architecture change is refused without --update.
    std::fs::write(&entry, SRC_V2).expect("write changed entry");
    let refused = ciac()
        .args([
            "baseline",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(!refused.status.success(), "must refuse without --update");
    assert_eq!(
        std::fs::read(&out).expect("unchanged"),
        bytes_after_create,
        "a refused update must not touch the file"
    );

    // --update alone is refused too (v0.18 M1: no classifier yet, so
    // every change conservatively needs --accept-breaking).
    let update_only = ciac()
        .args([
            "baseline",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--update",
        ])
        .output()
        .expect("ciac runs");
    assert!(
        !update_only.status.success(),
        "must refuse without --accept-breaking"
    );

    // --update --accept-breaking succeeds and actually changes the file.
    let updated = ciac()
        .args([
            "baseline",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--update",
            "--accept-breaking",
        ])
        .output()
        .expect("ciac runs");
    assert!(updated.status.success(), "{updated:?}");
    let bytes_after_update = std::fs::read(&out).expect("baseline updated");
    assert_ne!(bytes_after_update, bytes_after_create);
}

#[test]
fn baseline_accept_breaking_with_reason_appends_changelog() {
    let tmp = TempDir::new("changelog");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("main.semantic.json");
    std::fs::write(&entry, SRC_V1).expect("write entry");
    ciac()
        .args([
            "baseline",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");

    std::fs::write(&entry, SRC_V2).expect("write changed entry");
    let updated = ciac()
        .args([
            "baseline",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--update",
            "--accept-breaking",
            "--reason",
            "add summary field",
        ])
        .output()
        .expect("ciac runs");
    assert!(updated.status.success(), "{updated:?}");

    let changelog = std::fs::read_to_string(tmp.0.join("CHANGELOG.ciac.md"))
        .expect("changelog written next to entry");
    assert!(changelog.contains("add summary field"));
}

#[test]
fn baseline_refuses_a_newer_incompatible_version() {
    let tmp = TempDir::new("future-version");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("main.semantic.json");
    std::fs::write(&entry, SRC_V1).expect("write entry");
    ciac()
        .args([
            "baseline",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");

    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&out).unwrap()).expect("valid json");
    doc["semantic_baseline_version"] = serde_json::json!(999999);
    std::fs::write(&out, serde_json::to_vec_pretty(&doc).unwrap()).expect("write tampered file");

    std::fs::write(&entry, SRC_V2).expect("write changed entry");
    let refused = ciac()
        .args([
            "baseline",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--update",
            "--accept-breaking",
        ])
        .output()
        .expect("ciac runs");
    assert!(
        !refused.status.success(),
        "must refuse a newer baseline format"
    );
}

#[test]
fn baseline_defaults_to_entry_relative_ciac_dir() {
    let tmp = TempDir::new("default-path");
    let entry = tmp.0.join("main.ciac");
    std::fs::write(&entry, SRC_V1).expect("write entry");

    let created = ciac()
        .args(["baseline", entry.to_str().unwrap()])
        .output()
        .expect("ciac runs");
    assert!(created.status.success(), "{created:?}");

    let default_path = tmp
        .0
        .join(".ciac")
        .join("baselines")
        .join("main.semantic.json");
    assert!(
        default_path.exists(),
        "expected default path {}",
        default_path.display()
    );
}
