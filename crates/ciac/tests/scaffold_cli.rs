//! v0.12 M1: `ciac new`, exercised through the real binary. The load-
//! bearing property is "a scaffold always compiles": every template is
//! scaffolded into a fresh directory and the result must pass
//! `ciac check` — the same gate the embedded source examples already
//! pass in the golden suite, re-proven here on the scaffolded copy.

use std::path::PathBuf;
use std::process::Command;

fn ciac() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ciac"))
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ciac-new-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn every_template_scaffolds_and_the_result_passes_check() {
    for template in ["crud", "multi-service", "kafka", "minimal"] {
        let dir = temp_dir(template);

        let output = ciac()
            .args(["new", dir.to_str().unwrap(), "--template", template])
            .output()
            .expect("ciac runs");
        assert!(
            output.status.success(),
            "`ciac new --template {template}` failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let main = dir.join("main.ciac");
        assert!(main.is_file(), "{template}: main.ciac was written");
        assert!(
            dir.join("README.md").is_file(),
            "{template}: README.md was written"
        );

        let check = ciac()
            .args(["check", main.to_str().unwrap()])
            .output()
            .expect("ciac runs");
        assert!(
            check.status.success(),
            "scaffolded `{template}` template must pass `ciac check`: {}",
            String::from_utf8_lossy(&check.stderr)
        );

        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}

#[test]
fn new_refuses_a_non_empty_directory() {
    let dir = temp_dir("non-empty");
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(dir.join("keep.txt"), "precious").expect("seed file");

    let output = ciac()
        .args(["new", dir.to_str().unwrap()])
        .output()
        .expect("ciac runs");
    assert!(!output.status.success(), "must refuse a non-empty dir");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not empty"),
        "error names the problem: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !dir.join("main.ciac").exists(),
        "nothing was written into the refused directory"
    );

    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn new_accepts_an_existing_but_empty_directory() {
    let dir = temp_dir("empty-existing");
    std::fs::create_dir_all(&dir).expect("mkdir");

    let output = ciac()
        .args(["new", dir.to_str().unwrap(), "--template", "minimal"])
        .output()
        .expect("ciac runs");
    assert!(
        output.status.success(),
        "an existing empty dir is a valid target: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(dir.join("main.ciac").is_file());

    std::fs::remove_dir_all(&dir).expect("cleanup");
}

#[test]
fn new_rejects_an_unknown_template_at_the_cli_boundary() {
    let dir = temp_dir("unknown-template");

    let output = ciac()
        .args(["new", dir.to_str().unwrap(), "--template", "quantum"])
        .output()
        .expect("ciac runs");
    assert!(!output.status.success(), "clap rejects unknown templates");
    assert!(!dir.exists(), "nothing was created");
}
