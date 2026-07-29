//! v0.27 M7: `editors/vscode/snippets/ciac.json` is `contributes.snippets`'
//! payload, checked in (VS Code loads it from disk, not from the CLI) —
//! this test holds it byte-for-byte derived from `ciac describe`'s own
//! `snippets` table, which itself renders from `vocab::SNIPPETS`. Two
//! snippet sources that can drift apart is the exact disease `vocab.rs`
//! exists to cure (v0.13 M5's own rationale, extended here) — this test
//! is what makes that true of the VS Code file too, not just the LSP.

use std::process::Command;

#[test]
fn vscode_snippets_file_matches_vocab_table() {
    let output = Command::new(env!("CARGO_BIN_EXE_ciac"))
        .arg("describe")
        .output()
        .expect("ciac runs");
    assert!(output.status.success());
    let describe: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("ciac describe emits valid json");
    let snippets = describe["snippets"].as_array().expect("snippets array");

    let mut expected = serde_json::Map::new();
    for s in snippets {
        let prefix = s["prefix"]
            .as_str()
            .expect("prefix is a string")
            .to_string();
        expected.insert(
            prefix,
            serde_json::json!({
                "prefix": s["prefix"],
                "description": s["description"],
                "body": s["body"],
            }),
        );
    }
    let expected = serde_json::Value::Object(expected);

    let checked_in: serde_json::Value =
        serde_json::from_str(include_str!("../../../editors/vscode/snippets/ciac.json"))
            .expect("editors/vscode/snippets/ciac.json is valid json");

    assert_eq!(
        checked_in, expected,
        "editors/vscode/snippets/ciac.json is stale against vocab::SNIPPETS -- regenerate it \
         from `ciac describe`'s own `snippets` field (see the M7 Shipped note in \
         29UpdatePlan.md for the exact transform)"
    );
}
