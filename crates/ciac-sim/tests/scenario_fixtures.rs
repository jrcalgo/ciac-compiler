//! The M5-checkpoint fixture-file proof for `Scenario` (v0.17). Lives as
//! an integration test, not inside `src/scenario.rs`, specifically
//! because `scenario.rs` is vendored verbatim (`include_str!`) into every
//! generated Rust project that needs `SimWorld` (v0.17 M11) — a test
//! reaching for `sim/*.ciac-sim.json` via `CARGO_MANIFEST_DIR` only
//! resolves inside this crate's own checkout, and would fail in every
//! vendored copy if it lived in the vendored file itself.

use ciac_sim::Scenario;

#[test]
fn m5_checkpoint_scenarios_are_valid_instances_of_this_schema() {
    // The two scenario files 17UpdatePlan.md's M5 milestone checks in
    // (`sim/vertical-slice.ciac-sim.json`, `sim/virtual-week.ciac-sim.json`)
    // are real JSON documents, not just prose examples -- this test is the
    // schema-side half of the M5 checkpoint's proof: they parse and
    // structurally validate against the schema this crate owns. The
    // Python-side half (a real generated project executing the equivalent
    // effect sequence) lives in `sim/pyrunner/`, outside this crate.
    for name in ["vertical-slice", "virtual-week"] {
        let path = format!(
            "{}/../../sim/{name}.ciac-sim.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let json = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
        let scenario = Scenario::parse(&json)
            .unwrap_or_else(|e| panic!("{name}.ciac-sim.json failed to parse: {e}"));
        scenario
            .validate()
            .unwrap_or_else(|e| panic!("{name}.ciac-sim.json failed to validate: {e}"));
        assert!(!scenario.steps.is_empty());
    }
}
