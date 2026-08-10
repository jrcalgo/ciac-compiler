//! `31UpdatePlan.md` M6: the uniform-regression gate.
//!
//! `tests/tests/perf_budget.rs` compares backends to each other on the
//! same run, which makes it blind by construction to a regression that
//! slows every backend equally — exactly the shape of a shared-path
//! regression in `ciac-codegen`'s own template environment or context
//! model. This gate closes that hole: callgrind instruction counts for
//! a small, fixed corpus (`ping`, `order-system` — the corpus
//! `31UpdatePlan.md` M5's checkpoint fixed), compared against the
//! counts committed in `docs/perf/baseline.json`.
//!
//! **`#[ignore]`d by design** and never part of `cargo test --workspace`
//! — callgrind's own 10-50x slowdown means this cannot run in the
//! default suite without violating this arc's own second contract (the
//! instrument must not slow the suite it measures). Run explicitly:
//!
//! ```text
//! cargo build --release -p ciac
//! cargo test -p ciac-integration-tests --test perf_baseline -- --ignored
//! ```
//!
//! Ships **reporting-only** (`31UpdatePlan.md` M5's checkpoint,
//! outcome (b)): CI's own `perf-gate` job runs this and prints the
//! result, but the job does not fail the build. Promotion to blocking
//! is a named, mechanically-checkable follow-up
//! (`docs/perf/README.md`'s "Promotion status" section, `31UpdatePlan.md`
//! M9) — see that section for why a real 20-merge soak cannot happen
//! inside a single continuous session, and what a human checks later
//! to know when it has.

use ciac_integration_tests::bench::{
    check_instruction_budget, measure_instruction_counts, Baseline, INSTRUCTION_COUNT_BAND_PCT,
};
use std::path::Path;

const GATED_EXAMPLES: &[&str] = &["ping", "order-system"];

#[test]
#[ignore = "shells out to valgrind; run explicitly, see this file's own doc comment"]
fn no_shared_path_regresses_instruction_count() {
    let baseline = load_baseline();
    let current = measure_instruction_counts(GATED_EXAMPLES);
    assert_eq!(
        current.len(),
        GATED_EXAMPLES.len(),
        "every gated example must measure successfully -- see stderr above for which one failed"
    );
    if let Err(report) = check_instruction_budget(
        &current,
        &baseline.instruction_counts,
        INSTRUCTION_COUNT_BAND_PCT,
    ) {
        panic!("{report}");
    }
}

fn load_baseline() -> Baseline {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/perf/baseline.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
}
