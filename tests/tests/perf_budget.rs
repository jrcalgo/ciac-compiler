//! `30UpdatePlan.md` M8: the ratchet — a budget guard so this arc's
//! own gains can't silently regress back toward the JVM-per-file tax
//! M2 fixed. Relative, not absolute, so it survives a slower CI
//! runner: every backend's total generation cost across the full
//! example corpus is compared against the *median* backend's total on
//! the same run.
//!
//! The multiplier was picked from a live measurement, not guessed:
//! today's in-process steady state is Java at ~200-230x the median
//! backend (one JVM spawn per `generate()` call is a real, unavoidable
//! floor — not a defect this arc leaves behind). A real regression
//! back to one JVM spawn *per file* would push that ratio into the
//! tens of thousands (extrapolating from `docs/perf/codegen-
//! baseline.md`'s own M1 numbers: ~0.51s per file, dozens of files per
//! example, 29 examples). [`BUDGET_MULTIPLIER`] sits with wide
//! headroom on both sides — comfortably above today's steady state so
//! ordinary variance never trips it, comfortably below the
//! catastrophe floor so a real regression reliably does.
//!
//! **`31UpdatePlan.md` M9 revisited this multiplier** (not merely
//! asserted it in passing): this file's own premise changed once M6's
//! callgrind gate and M7's asymptotic guard landed, since this test
//! was originally the *only* gate and had to catch everything. It now
//! answers one narrower question — has one backend become an outlier
//! relative to its peers on the same run — while M6 catches a
//! shared-path regression uniformly, at a much tighter 1% band, and M7
//! catches a growth-shape regression. That narrower job argued for
//! *considering* a smaller multiplier. It was kept at 1000, not
//! lowered, for a reason found live during this revisit: this
//! session's own debug-profile measurement of `measure_all()` (the
//! profile `cargo test --workspace` actually runs this gate under —
//! `--release` measurements are not representative here, since Java's
//! JVM-spawn cost is release/debug-invariant while the cheap backends'
//! own in-process cost is not, which briefly produced a misleadingly
//! tight ~950-1010x ratio before this was caught) swung from
//! java-at-131.7x-median to java-at-187.4x-median to java's own total
//! moving 34.0s → 48.8s across two back-to-back runs with *no code
//! change at all* — a ~43% run-to-run swing from JVM-spawn variance
//! alone. Tightening the multiplier into that noise band would trade
//! "never fires spuriously" for marginal extra sensitivity M6's own
//! instruction-count gate already provides, more precisely, on exactly
//! the shared-path case this test structurally cannot isolate a cause
//! for. Per this arc's own Pillar 7 discipline ("a gate that fires
//! spuriously twice is fixed or removed — never widened silently"),
//! shrinking a coarse safety net into a source of flakiness is the
//! same mistake in the opposite direction. "Revisited" and "changed"
//! are two different facts; this multiplier's value is the same one
//! `30UpdatePlan.md` M8 chose, re-confirmed against real M9 data
//! rather than left standing by default.

use ciac_codegen::{BackendError, GenOptions};
use ciac_integration_tests::{backends, ciac_files, compile_file, examples_dir};
use std::time::{Duration, Instant};

const BUDGET_MULTIPLIER: u32 = 1000;

#[test]
fn no_backend_exceeds_the_budget() {
    let totals = measure_all();
    if let Err(report) = check_budget(&totals, BUDGET_MULTIPLIER) {
        panic!("{report}");
    }
}

/// Generates every example this corpus contains once per backend,
/// summing wall time per backend. Backends that refuse a given
/// example (`BackendError::Unsupported`) simply don't contribute that
/// example to their own total — this measures generation cost, not
/// coverage.
fn measure_all() -> Vec<(&'static str, Duration)> {
    let files = ciac_files(&examples_dir());
    backends()
        .into_iter()
        .map(|backend| {
            let mut total = Duration::ZERO;
            for f in &files {
                let ir = compile_file(f);
                let start = Instant::now();
                match backend.generate(&ir, &GenOptions::default()) {
                    Ok(_) => total += start.elapsed(),
                    Err(BackendError::Unsupported { .. }) => {}
                    Err(e) => panic!("{}: unexpected generation failure: {e}", backend.id()),
                }
            }
            (backend.id(), total)
        })
        .collect()
}

/// Pure comparison logic, kept separate from measurement so it can be
/// exercised with synthetic data (see the `tests` module below)
/// without paying for a full-corpus generation run — this is also the
/// harness-can-fail proof M8 requires: fed a synthetically slowed
/// entry, it fails, by construction.
fn check_budget(totals: &[(&'static str, Duration)], multiplier: u32) -> Result<(), String> {
    let mut sorted: Vec<Duration> = totals.iter().map(|(_, d)| *d).collect();
    sorted.sort();
    let median = sorted[sorted.len() / 2].max(Duration::from_nanos(1));
    let budget = median * multiplier;
    let over_budget = totals.iter().any(|(_, d)| *d > budget);
    if !over_budget {
        return Ok(());
    }
    let mut table = format!(
        "generation cost budget exceeded (median {median:?} \u{d7} {multiplier} = {budget:?} allowed):\n"
    );
    for (name, d) in totals {
        let ratio = d.as_secs_f64() / median.as_secs_f64();
        table.push_str(&format!("  {name:<12} {d:>14?}  {ratio:>10.1}x median\n"));
    }
    Err(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_at_todays_steady_state() {
        // Mirrors the live-measured shape: four cheap backends
        // clustered together, one ~200x the median.
        let totals = vec![
            ("python", Duration::from_millis(6)),
            ("rust", Duration::from_millis(6)),
            ("typescript", Duration::from_millis(6)),
            ("go", Duration::from_millis(20)),
            ("java", Duration::from_millis(1_260)),
        ];
        assert!(check_budget(&totals, BUDGET_MULTIPLIER).is_ok());
    }

    #[test]
    fn fails_when_a_backend_is_artificially_slowed() {
        let totals = vec![
            ("python", Duration::from_millis(6)),
            ("rust", Duration::from_millis(6)),
            ("typescript", Duration::from_millis(6)),
            ("go", Duration::from_millis(20)),
            ("java", Duration::from_secs(30)), // ~5000x median: a regression, not noise
        ];
        let err = check_budget(&totals, BUDGET_MULTIPLIER)
            .expect_err("an artificially slowed backend must trip the budget");
        assert!(
            err.contains("java"),
            "report should name the offending backend: {err}"
        );
    }
}
