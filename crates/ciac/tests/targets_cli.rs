//! v0.22 M4: `docs/targets.json` is `ciac targets --json`'s output,
//! checked in for docs-build/agent consumers who'd rather read a file
//! than run the binary. This test holds it byte-identical to what the
//! current registry actually derives — the checked-in copy can never
//! silently go stale.

use std::process::Command;

#[test]
fn checked_in_targets_json_matches_the_derived_one() {
    let output = Command::new(env!("CARGO_BIN_EXE_ciac"))
        .args(["targets", "--json"])
        .output()
        .expect("ciac runs");
    assert!(output.status.success());
    let derived = String::from_utf8(output.stdout).expect("utf-8 output");
    let checked_in = include_str!("../../../docs/targets.json");
    assert_eq!(
        checked_in, derived,
        "docs/targets.json is stale; regenerate it with `cargo run -p ciac -- targets --json > docs/targets.json`"
    );
}
