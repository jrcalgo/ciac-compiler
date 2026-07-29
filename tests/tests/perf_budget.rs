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
