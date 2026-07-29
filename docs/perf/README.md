# Codegen performance

*Reader: a contributor investigating why generation or the test suite
is slow, or checking whether a change moved the numbers.*

- [codegen-baseline.md](codegen-baseline.md) — the measured per-target
  generation cost, the M1→M5→M9 delta table, and how to reproduce both
  with `scripts/bench-codegen.sh`.
- `tests/tests/perf_budget.rs` is the actual gate (`30UpdatePlan.md`
  M8): a loose, relative budget asserting no backend's generation cost
  exceeds a generously-headroomed multiple of the median backend on
  the same run. It exists to catch a return to the JVM-per-file
  catastrophe `30UpdatePlan.md` fixed, not to police ordinary
  variance — see that file's own doc comment for the exact multiplier
  and the measurement that picked it.
- CI prints the full sweep every run as information, not a gate (the
  "Codegen speed (informational)" step in `.github/workflows/ci.yml`'s
  `test` job) — `perf_budget.rs` is what's allowed to fail the build.

If you're changing a backend's codegen path (a new template, a
different formatter invocation, anything touching `generate()`), run
`scripts/bench-codegen.sh --targets <yours>` before and after — it's
faster than waiting for CI and gives you the exact numbers this
document's own table is built from.
