//! v0.10 M3: the `--json` contract, exercised through the real binary:
//! exactly one JSON document on stdout, diagnostics resolved to
//! file/line/column, human narration confined to stderr.

use std::path::Path;
use std::process::Command;

fn ciac() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ciac"))
}

fn fixture(rel: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
        .canonicalize()
        .expect("fixture exists")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn check_json_reports_a_resolved_diagnostic_on_a_bad_fixture() {
    let output = ciac()
        .args(["check", &fixture("tests/ui/missing-queue.ciac"), "--json"])
        .output()
        .expect("ciac runs");
    assert!(!output.status.success(), "bad fixture must fail");

    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON document");
    assert_eq!(doc["json_version"], 2);
    assert_eq!(doc["command"], "check");
    assert_eq!(doc["success"], false);

    let diags = doc["diagnostics"].as_array().expect("diagnostics array");
    assert!(!diags.is_empty());
    let first = &diags[0];
    assert_eq!(first["code"], "CIAC0005");
    assert_eq!(first["severity"], "error");
    let label = &first["labels"][0];
    assert!(
        label["file"]
            .as_str()
            .expect("file path")
            .ends_with("missing-queue.ciac"),
        "{label}"
    );
    assert_eq!(label["line"], 4, "1-based line of the `Queue` step");
    assert!(label["column"].as_u64().expect("column") >= 1);
}

#[test]
fn check_json_on_a_valid_program_is_a_clean_success_envelope() {
    let output = ciac()
        .args(["check", &fixture("examples/single-service/ping.ciac"), "--json"])
        .output()
        .expect("ciac runs");
    assert!(output.status.success());

    let doc: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("stdout is exactly one JSON document");
    assert_eq!(doc["success"], true);
    assert_eq!(doc["diagnostics"].as_array().expect("array").len(), 0);
}
