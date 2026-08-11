# Codegen and compilation performance

*Reader: a contributor investigating why generation, validation, or the
test suite is slow, checking whether a change moved the numbers, or
deciding whether a reporting-only gate is ready to promote.*

## Instrument index

- [codegen-baseline.md](codegen-baseline.md) — the measured per-target
  generation cost, `30UpdatePlan.md`'s own M1→M5→M9 delta table, and
  how to reproduce it with `scripts/bench-codegen.sh`. Its own
  hand-maintained "Readings so far" table is retired as of
  `31UpdatePlan.md` M9 — see that document's own note on why, and
  `baseline.json` below for what replaced it.
- [noise-floor.md](noise-floor.md) — `31UpdatePlan.md` M1: wall-clock
  and callgrind noise characterization for four examples, the
  pre-registered Pillar-1 gating rule (`max(3σ, 10%)` of measured
  noise, `150×` for instruction-count metrics specifically), and the
  M5 checkpoint's gate decisions with their reasoning.
- [baseline.json](baseline.json) — `31UpdatePlan.md` M4/M6/M7/M8: the
  single machine-readable baseline every instrument below reads or
  writes. Environment-stamped, git-sha-stamped, additive across
  milestones. Never hand-edited — always written by
  `ciac-bench --update-baseline`, with a justification in the commit
  message per this arc's own discipline (below).

| Instrument | What it measures | Ships | Gate? |
|---|---|---|---|
| `tests/tests/perf_budget.rs` | one backend's generation cost vs. the median backend, same run | `30UpdatePlan.md` M8 | **blocking**, part of `cargo test --workspace` |
| `tests/tests/perf_baseline.rs` (`perf-gate` CI job) | callgrind instruction count, fixed corpus, vs. committed baseline | `31UpdatePlan.md` M6 | reporting-only — see "Promotion status" |
| `tests/tests/perf_scaling.rs` | `sema::analyze`/`generate()` growth ratio at N/2N/4N, single run | `31UpdatePlan.md` M7 | reporting-only — see "Promotion status" |
| `perf-noise-floor` CI job | wall-clock + callgrind sweep on real `ubuntu-latest` hardware | `31UpdatePlan.md` M1 | informational only, never a promotion candidate |
| `tests/bin/ciac-bench.rs` | phase-level timing, instruction counts, scaling, verify, slow test binaries, sim | `31UpdatePlan.md` M2/M4/M6/M7/M8 | not a gate — the measuring tool the gates and `baseline.json` are built on |
| `scripts/bench-codegen.sh` | per-target generation cost, warm rebuild, deploy/client coverage, output size | `30UpdatePlan.md` / `31UpdatePlan.md` M3 | not a gate — human-runnable investigation tool |
| `scripts/bench-verify.sh` | per-`ValidateStep` timing, one target at a time | `31UpdatePlan.md` M3/M8 | not a gate |
| `scripts/bench-callgrind.sh` | instruction counts and DHAT allocation totals, standalone | `31UpdatePlan.md` M6/M7 | not a gate |

CI prints the informational sweeps every run (`perf-noise-floor`,
`scripts/bench-codegen.sh`'s own step in the `test` job) without
failing the build on them — only `perf_budget.rs` is currently allowed
to fail CI on a performance regression.

## Promotion status

`31UpdatePlan.md` M5 pre-registered the soak rule every reporting-only
gate below is held to, verbatim:

> Soak is defined in merges, not days: a gate promotes to blocking
> after 20 merges to `main` with no spurious fire. If it fires
> spuriously once, the count resets and the band is re-derived from
> M1's data. If it fires spuriously twice, the gate is removed rather
> than widened.

**Neither `perf_baseline.rs`/`perf-gate` nor `perf_scaling.rs` has
promoted.** `31UpdatePlan.md` was executed in one continuous session,
which cannot manufacture a real 20-merge history without fitting the
soak count to convenience — exactly the failure mode the rule exists
to prevent (see that plan's own "tension 1" reasoning). Both gates
ship reporting-only, as pre-registered outcome (b) from the M5
checkpoint (`noise-floor.md`'s own "Gate decisions" section), and stay
reporting-only until a human — not a continued single session — has
watched them across real merges.

| Gate | Shipped at | Merges observed toward promotion |
|---|---|---|
| `perf_baseline.rs` / `perf-gate` | `3aeac4c` | 0 (see note below) |
| `perf_scaling.rs` | `76be418` | 0 |

**Note on `perf_baseline.rs`'s one real CI failure:** commit `7c86a0a`
deliberately introduced a shared-path regression to prove the gate
fires in a real CI run (`31UpdatePlan.md` M6's own end-to-end
demonstration requirement), confirmed firing correctly
(run `31364165785`, job `93378885978`: `ping +7.49%`,
`order-system +15.09%`), then reverted at `c9aa060`. This is an
intentional, disclosed proof exercise, not a spurious fire — it does
not count against the soak counter, and is recorded here so a future
reader auditing CI history for "spurious fires so far" knows to
exclude it rather than double-count it as strike one.

**The mechanical check for promotion**, to run by hand once real merge
history exists:

1. Count merge commits to `main` on or after each gate's own "Shipped
   at" SHA above.
2. For each, check whether the corresponding CI job
   (`perf-gate` for `perf_baseline.rs`; there is no scheduled CI job
   for `perf_scaling.rs` yet — it would need one added at promotion
   time, since it currently only runs on demand) reported a failure
   that was not itself caused by a real regression in that merge's own
   diff.
3. If the count reaches 20 with zero such failures, promote: change
   `continue-on-error: true` to `false` on the `perf-gate` job's
   own gate step in `.github/workflows/ci.yml` for
   `perf_baseline.rs`, and remove the `#[ignore]` attribute (adding a
   scheduled or on-demand CI job first) for `perf_scaling.rs`.
4. If any failure in that window traces to noise rather than a real
   regression, that is a spurious fire under the rule above: reset the
   count, re-derive the band from fresh noise-floor data
   (`noise-floor.md`'s own methodology), and restart the 20-merge
   count from the re-derivation commit.
5. Two spurious fires on the same gate: remove it, per the rule's own
   "never widened silently" clause, and record why in this file.

**`32UpdatePlan.md` M9 applied this check, honestly, per its own
pre-registration.** That milestone's text stated in advance: *"this arc
is a single continuous session, so neither [gate] will have [accumulated
real merge history], and the 'Promotion status' table gets the real
count rather than a convenient one."* Running the mechanical check above
against this arc's own commits confirms exactly that — zero merge
commits to `main` observed by either gate across `32UpdatePlan.md`'s
own run, for the identical structural reason `31UpdatePlan.md` M9 gave
the first time this table was written. The table above is left
unchanged because it was already accurate; neither gate promotes.

## `32UpdatePlan.md` M5 checkpoint

Cumulative M2+M3+M4 measured against the frozen `baseline-v0.29.0.json`
(git `76be418`), per that milestone's own pre-registration: no new
code, full Pillar-3 suite, decision recorded here with the numbers
inline rather than by reference.

**`ciac-bench --compare=docs/perf/baseline-v0.29.0.json`** (per-phase
metrics, every example): every row improved, none regressed. Selected
rows, `order-system`:

| Metric | Old (µs) | New (µs) | Change |
|---|---|---|---|
| `semantic_model::semantic_hash` | 64.5 | 17.5 | -72.8% |
| `regen::plan_regeneration [warm]` | 888.2 | 309.4 | -65.2% |
| `manifest::build_manifest` | 362.4 | 75.3 | -79.2% |
| `generate() [python]` | 1291.6 | 1124.1 | -13.0% |
| `generate() [go]` | 18894.9 | 15180.8 | -19.7% |
| `generate() [java]` | 1342038.1 | 956293.6 | -28.7% |

The full table (5 examples × ~15 metrics, all negative) is in
`docs/perf/baseline.json`'s own git history at this milestone's commit.

**Fields `compare_baselines` doesn't cover** (Contract 2 requires the
full report, not only the delta table), diffed by hand against
`baseline-v0.29.0.json`:

| Field | Old | New | Change |
|---|---|---|---|
| `instruction_counts` / ping | 12,982,474 | 12,058,278 | -7.12% |
| `instruction_counts` / order-system | 25,416,100 | 21,643,893 | -14.84% |
| `verify` / python (sum) | 3.872s | 3.857s | -0.40% |
| `verify` / rust (sum) | 116.555s | 98.773s | -15.26% |
| `verify` / typescript (sum) | 18.420s | 13.113s | -28.81% |
| `verify` / go (sum) | 9.032s | 4.262s | -52.82% |
| `verify` / java (sum) | 319.746s | 317.079s | -0.83% |
| `slow_test_binaries` / determinism | 74.221s | 80.572s | **+8.56%** |
| `slow_test_binaries` / conformance | 65.029s | 56.041s | -13.82% |
| `slow_test_binaries` / golden | 35.613s | 30.595s | -14.09% |
| `slow_test_binaries` / openapi | 35.456s | 29.158s | -17.76% |
| `sim` / python (sum) | 3.028s | 2.689s | -11.17% |
| `sim` / rust (sum) | 77.995s | 64.340s | -17.51% |
| `sim` / typescript (sum) | 13.898s | 10.352s | -25.51% |
| `sim` / go (sum) | 5.533s | 3.633s | -34.34% |
| `sim` / java (sum) | 32.977s | 23.909s | -27.50% |

`determinism`'s +8.56% is the only regression-shaped number anywhere
in this checkpoint, and it is a measurement artifact, not a code
effect: `ciac-bench`'s single-shot reading (80.572s) landed immediately
after the callgrind and verify phases in the same process, both
CPU/IO-heavy. Three isolated re-runs of `cargo test -p
ciac-integration-tests --test determinism` right after, on an otherwise
quiet system, measured 68.77s / 56.27s / 56.60s — below the *old*
baseline (74.221s), consistent with every other slow-test-binary
improving. Noise, not a regression; Contract 2's "no metric may
regress" holds.

**`cargo test --workspace --release`: one spurious gate fire, resolved
by isolation.** The full run (16m11.869s, itself inflated by running
two concurrent release builds by mistake) hit one real failure:
`perf_budget.rs`'s `no_backend_exceeds_the_budget`, reporting java at
1197.7x the median (31.519s vs. a 26.317s budget) — far outside that
gate's own documented steady state (its own doc comment records
131-230x from JVM-spawn variance alone, no code change, in
`31UpdatePlan.md` M9's own revisit). That run was executing under heavy
self-inflicted contention: two simultaneous `cargo build --release`
runs plus a `valgrind` callgrind measurement, all competing for the
same CPU. Re-run twice in complete isolation immediately after: **pass,
26.88s and 27.05s**, java back in its normal band. Filed as contention,
not a regression, per this file's own Pillar 7 discipline (a gate that
fires spuriously is investigated before anything is loosened) — one
fire, explained, not two, so no gate change is warranted.

**`perf_baseline.rs`** (`--ignored`): pass. **`perf_scaling.rs`**
(`--ignored`): pass, growth ratios 2.11-2.77x per doubling (well under
M7's own future 2.5x guard, not yet gated). **`scripts/bench-codegen.sh`,
`scripts/bench-verify.sh`** (all 5 targets), **`scripts/bench-callgrind.sh`**:
all run clean. **`scripts/sim-corpus-x5.sh`**: not re-run at this exact
commit — M5 introduces no code change, and it last ran 50/50 green
immediately after M4's own commit, on the identical tree this
checkpoint measures.

**Decision: outcome (a) — M6, M7 and M8 proceed exactly as
pre-registered.** M2 (15.14% vs. an 8% floor), M3 (mixed — see
`32UpdatePlan.md`'s own "Contract 3 tension" note — but the mechanism
verified correct and `sim-three-service` cleared 8.84x), and M4
(51.9-70.8% vs. a 40% floor) all met or exceeded their thresholds, and
this checkpoint's own full-metric sweep shows gains everywhere with no
disclosed trade. Nothing here argues for narrowing M6 (outcome b) or
cutting M7's item 11 (outcome c) — those remain live options to revisit
at M6's own start if its `build_system` dedupe turns out to be more
surgery than the checkpoint's cumulative gains justify, per that
milestone's own text, but nothing at M5 pre-empts that judgment.

## Discipline this arc adopted (`31UpdatePlan.md` Pillar 7)

Recorded here so later work inherits it as a standing convention:

- Any change touching a hot path names the metric it expects to move,
  and the direction, **before** the change.
- `--update-baseline` requires a justification in the commit message.
  A baseline that moves without a stated reason is a regression that
  was normalised.
- A gate that fires spuriously twice is fixed or removed — never
  widened silently. Widening is how gates die quietly rather than
  loudly.

## Reproducing the numbers

```sh
# phase-level timing, instruction counts (needs --with-callgrind),
# asymptotic guard (needs --with-scaling), validation/slow-binary/sim
# timing (needs --with-verify/--with-slow-tests/--with-sim; real
# minutes each)
cargo run --release -p ciac-integration-tests --bin ciac-bench

# the actual gates
cargo test -p ciac-integration-tests --test perf_budget
cargo test -p ciac-integration-tests --test perf_baseline -- --ignored
cargo test --release -p ciac-integration-tests --test perf_scaling -- --ignored

# standalone investigation tools
scripts/bench-codegen.sh
scripts/bench-verify.sh --target python
scripts/bench-callgrind.sh --metric instructions
scripts/bench-callgrind.sh --metric allocations
```

If you're changing a backend's codegen path, `ciac-codegen`'s own
context model, or anything on the front end (`ciac-syntax`,
`ciac-sema`), run the relevant instrument above before and after —
it's faster than waiting for CI and gives you the exact numbers this
document's own table is built from.
