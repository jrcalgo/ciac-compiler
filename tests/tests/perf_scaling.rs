//! `31UpdatePlan.md` M7: the asymptotic guard.
//!
//! `tests/tests/perf_baseline.rs` (M6) catches a shared path getting
//! uniformly slower. It is structurally blind to a different failure:
//! someone adds an O(n^2) lookup that is invisible on every example in
//! the fixed corpus because none of them are large enough to make the
//! constant factor visible. This gate closes that hole by measuring
//! the same corpus shape at three sizes (N, 2N, 4N) in a single run on
//! a single machine, and asserting the cost never grows faster than a
//! documented ceiling per doubling.
//!
//! Runner-independent by construction -- unlike M6's callgrind gate,
//! this needs no noise-floor calibration at all, since it only ever
//! compares two measurements taken back to back in the same process.
//! `#[ignore]`d anyway, for consistency with M6 and because measuring
//! three corpus sizes with enough repetitions to be stable still takes
//! longer than this arc's own second contract allows inside
//! `cargo test --workspace`. Run explicitly:
//!
//! Run with `--release`: the ceiling constants were calibrated against
//! release-profile numbers (matching every other wall-clock measurement
//! in this arc, e.g. `ciac-bench` itself), and a debug build's uneven
//! per-operation overhead (bounds checks, no inlining) is not
//! guaranteed to preserve the same growth ratios.
//!
//! ```text
//! cargo test --release -p ciac-integration-tests --test perf_scaling -- --ignored --nocapture
//! ```
//!
//! Ships **reporting-only** (`31UpdatePlan.md` M5's checkpoint,
//! outcome (b), extended to this gate at M7 for the same reason: no
//! real soak history exists inside a single continuous session).
//! Promotion is `docs/perf/README.md`'s job, per M9.

use ciac_integration_tests::bench::{
    check_growth_ceiling, measure_scaling, GENERATE_GROWTH_CEILING, SEMA_ANALYZE_GROWTH_CEILING,
};

const REPS: u32 = 20;

#[test]
#[ignore = "takes tens of seconds to measure three corpus sizes; run explicitly, see this file's own doc comment"]
fn front_end_and_generation_growth_stay_under_ceiling() {
    let n1 = measure_scaling(100, REPS);
    let n2 = measure_scaling(200, REPS);
    let n4 = measure_scaling(400, REPS);

    let mut failures = Vec::new();

    for (from, to, doubling) in [(&n1, &n2, "N->2N"), (&n2, &n4, "2N->4N")] {
        if let Err(e) = check_growth_ceiling(
            from.analyze.mean_us,
            to.analyze.mean_us,
            SEMA_ANALYZE_GROWTH_CEILING,
            &format!("sema::analyze [{doubling}]"),
        ) {
            failures.push(e);
        }
        if let Err(e) = check_growth_ceiling(
            from.generate.mean_us,
            to.generate.mean_us,
            GENERATE_GROWTH_CEILING,
            &format!("generate() [python, {doubling}]"),
        ) {
            failures.push(e);
        }
    }

    println!(
        "N={:<4} analyze={:>10.1}us generate={:>10.1}us",
        n1.n, n1.analyze.mean_us, n1.generate.mean_us
    );
    println!(
        "N={:<4} analyze={:>10.1}us generate={:>10.1}us",
        n2.n, n2.analyze.mean_us, n2.generate.mean_us
    );
    println!(
        "N={:<4} analyze={:>10.1}us generate={:>10.1}us",
        n4.n, n4.analyze.mean_us, n4.generate.mean_us
    );

    if !failures.is_empty() {
        panic!("asymptotic guard tripped:\n{}", failures.join("\n"));
    }
}
