# Codegen performance baseline

*Reader: anyone investigating why generation or the test suite is slow,
or checking whether 30UpdatePlan.md's fixes actually worked. Not a
benchmark to quote — a regression detector.*

> 30UpdatePlan.md M1, updated at M5 and M8. These are single-machine numbers gathered in one
> sandbox with a warm `cargo`/`rustc` cache and a cold JVM (no prior
> `java` invocation in this session before the sweep). Absolute
> seconds will differ on any other machine — different CPU, different
> JVM startup cost, different disk. **What travels is the ratio
> between targets on the same machine, in the same run** — that ratio
> is what this document exists to track over the arc's M1→M5→M9
> readings, the same way 29UpdatePlan.md's three cold-start
> transcripts tracked front-door friction as a diffable delta rather
> than a one-off number. Treat every number below as "true on this
> machine, on this day" and the ratios as the durable signal.

## Environment

- 4 logical CPUs (`nproc`), 15GB RAM, Linux 6.18.5 x86_64.
- `rustc 1.94.1`, release profile (`cargo build --release -p ciac`)
  for the generation sweep — debug profile for the test-binary timings
  below, matching how `cargo test` actually runs today (no
  `[profile.test]` exists yet; that's 30UpdatePlan.md M7's question).
- JVM available on `PATH`; `google-java-format` jar vendored at
  `crates/ciac-backend-java/vendor/google-java-format-1.19.2-all-deps.jar`.
  No JVM warm-up was done before measuring — every Java number below
  includes a cold JVM startup, which is the realistic case for a CLI
  invocation (a `ciac build` is not a long-lived process with a warm
  JIT).
- No other CPU-heavy process was running concurrently during either
  the generation sweep or the individual test-binary timings (each was
  run alone, not under the full `cargo test --workspace` sweep, so
  these numbers isolate each binary's own cost rather than measuring
  contention).

## M1 — Baseline: per-target generation cost

29 `examples/*.ciac` × 5 targets = up to 145 builds (a handful of
combinations are refused by `check_support` and excluded automatically
by the script; the corpus below reflects every combination that
actually built).

| Target | Builds | Total (s) | Mean (s) | Min (s) | Max (s) | Ratio to fastest |
|---|---|---|---|---|---|---|
| rust | 29 | 0.387 | 0.013 | 0.007 | 0.022 | **1.00x** |
| typescript | 29 | 0.479 | 0.017 | 0.010 | 0.053 | 1.24x |
| python | 29 | 2.499 | 0.086 | 0.007 | 0.491 | 6.45x |
| go | 29 | 3.279 | 0.113 | 0.031 | 1.128 | 8.47x |
| **java** | 29 | **459.283** | **15.837** | 5.142 | 48.351 | **1186.04x** |

Rust is the fastest backend on this machine, not Python or TypeScript
as the plan's spot-check on `order-system.ciac` alone suggested —
across the full 29-example corpus, Rust's mean edges out TypeScript's.
Java is slower than every other target by close to three orders of
magnitude, confirming 30UpdatePlan.md's central claim with the full
corpus rather than one example: this is not a `order-system.ciac`
quirk, it holds across every program in `examples/`.

The largest single Java build (`sim-three-service.ciac`, a
multi-service example emitting 96 files) took 30.8s — visibly worse
than the flagship `order-system.ciac`'s 21.6s, exactly as the plan's
cost model predicts (`t_java(F) ≈ F × 0.51s`; more services and more
records mean more `.java` files, and the JVM-per-file cost scales with
that count, not with program complexity).

Full per-example, per-target breakdown is reproducible via
`scripts/bench-codegen.sh` (see "How to reproduce" below); it is not
duplicated here to keep this document a summary rather than a second
copy of the script's own output.

## Before/after: per-target generation cost (M1 vs. M5)

The same sweep, re-run after M2 (Java batch formatting), M3 (the
batching seam generalized to Go), and M4 (per-backend template
memoization) — this is the arc's own headline result, in one table:

| Target | M1 mean | M5 mean | Change | M1 ratio to fastest | M5 ratio to fastest |
|---|---|---|---|---|---|
| python | 0.086s | 0.014s | 6.14x faster | 6.45x | 1.00x |
| rust | 0.013s | 0.015s | ~flat (noise) | **1.00x** | 1.05x |
| typescript | 0.017s | 0.017s | ~flat | 1.24x | 1.17x |
| go | 0.113s | 0.032s | 3.53x faster | 8.47x | 2.27x |
| **java** | **15.837s** | **1.277s** | **12.40x faster** | **1186.04x** | **90.00x** |

**Java's drop is the only one with a mechanistic explanation, and it
is the arc's actual subject**: M2 converted ~24-140 JVM spawns per
project (one per generated `.java` file) into exactly one, per
`generate()` call. Python's, Rust's, TypeScript's, and Go's small
moves are **not** attributable to M3 or M4 — `scripts/bench-
codegen.sh` runs `ciac build` as a separate process per (example,
target) pair, and `ciac build` calls `Backend::generate` exactly once
per process, so neither M3's batching seam (Go's own formatter cost
was always ~2ms, immaterial either way) nor M4's per-backend `OnceLock`
template cache (empty at the start of every fresh process) has
anything to amortize against within this sweep. Their small drops most
plausibly reflect less concurrent system load during the M5 run than
the M1 run (which shared the machine with other heavy test-suite
invocations in the same working session) — disclosed here rather than
credited to work that structurally cannot have caused it. **The
targets whose readings actually demonstrate M3 and M4's own effect are
`determinism.rs` and `conformance.rs`** (below), which make hundreds of
`generate()` calls inside one long-lived process — exactly the shape
those two milestones' caches need to pay for themselves.

## M1 — Baseline: the four slow test binaries

Each timed alone (not as part of `cargo test --workspace`), debug
profile, so these numbers are directly comparable to what a developer
sees running `cargo test -p ciac-integration-tests --test <name>`
today:

| Binary | Wall time (`time`, real) | Test-reported time | `#[test]` fns |
|---|---|---|---|
| `determinism.rs` | 15m22.5s | 922.27s | 1 |
| `conformance.rs` | 11m38.3s | 656.79s | 3 |
| `golden.rs` | 8m3.8s | 478.99s | 3 |
| `openapi.rs` | 7m20.9s | 439.80s | 1 |
| **Combined** | **42m25.5s** | **2497.85s** | 8 |

The gap between "wall time" and "test-reported time" in each row is
mostly `cargo`'s own compile step for that binary (each was run as a
fresh `cargo test` invocation, not a warm re-run) — the test-reported
figures are the ones directly comparable to 30UpdatePlan.md's own
citations of these binaries' costs.

`determinism.rs` and `openapi.rs` each hold exactly one `#[test]` fn,
confirmed here by direct observation — libtest has nothing to
parallelize within either binary, so each is a single thread walking
its whole share of the 145-combination corpus.

## Before/after: the four slow test binaries (M1 vs. M5)

Unlike the per-target sweep above, every one of these binaries makes
many `generate()` calls inside one process — this is where M3's and
M4's own mechanisms actually had something to amortize against, on
top of M2's dominant fix:

| Binary | M1 | M5 | Change |
|---|---|---|---|
| `determinism.rs` | 922.27s | 81.20s | 11.36x faster |
| `conformance.rs` | 656.79s | 94.17s | 6.97x faster |
| `golden.rs` | 478.99s | 39.06s | 12.26x faster |
| `openapi.rs` | 439.80s | 38.80s | 11.34x faster |
| **Combined** | **2497.85s** | **253.23s** | **9.87x faster** |

## What this does not measure

- **`ciac verify`'s downstream toolchain invocations** (`uv`, `cargo
  check`, `tsc`, `go build`, `mvn`/Maven wrapper) — real cost, entirely
  outside generation, and dominated by each ecosystem's own compiler/
  linter rather than anything CIaC controls.
- **Docker-dependent paths** (`ciac verify --system`, `ciac dev`'s
  compose-stack startup) — gated behind a running Docker daemon this
  sandbox does not exercise for this measurement.
- **Simulation runtime** (`ciac sim`, `scripts/sim-corpus-x5.sh`) — a
  separate hot path with its own cost profile (virtual-clock
  scheduling, fake I/O), unrelated to `generate()`'s template-rendering
  and formatter costs. `34UpdatePlan.md` reduced its cost via a shared
  cargo target directory (rust steady-state 3.07×); see
  `docs/perf/README.md`'s own `34UpdatePlan.md` checkpoint section for
  the full result, including the parallelization lever that was
  measured and reverted rather than shipped.
- **Anything about correctness.** This document says nothing about
  whether generated code is right — that's the golden/negative/
  equivalence suites' job, untouched by this arc, verified separately
  at every milestone.

## How to reproduce

```sh
# full sweep, all 29 examples x 5 targets
scripts/bench-codegen.sh

# a faster, targeted check while iterating on one backend
scripts/bench-codegen.sh --targets java
scripts/bench-codegen.sh --targets java --examples ping,order-system

# the four slow test binaries, timed individually
time cargo test -p ciac-integration-tests --test determinism
time cargo test -p ciac-integration-tests --test conformance
time cargo test -p ciac-integration-tests --test openapi
time cargo test -p ciac-integration-tests --test golden
```

The script builds `ciac` in release mode once, then times a real
`ciac build` per (example, target) pair into a scratch directory it
cleans up itself. It exits non-zero only on an actual build failure —
it is a measuring instrument, not a pass/fail gate. The actual gate is
`tests/tests/perf_budget.rs` (`30UpdatePlan.md` M8, part of `cargo
test --workspace`): a loose, relative budget that fails only if a
backend's generation cost returns to hundreds-of-times worse than the
median backend, not on ordinary variance — see that file's own doc
comment for the exact multiplier and the measurement behind it. CI
runs `scripts/bench-codegen.sh` on every push too, printed as
information in the job summary rather than as a second gate.

## Readings so far (retired at `31UpdatePlan.md` M9)

| Reading | When | Combined slow-binary time | Java mean generation | Ratio to fastest |
|---|---|---|---|---|
| M1 (this document) | pre-optimization | 2497.85s | 15.837s | 1186.04x |
| M5 (checkpoint) | post M2 (Java batch fmt) + M3 (Go seam) + M4 (template memo) | 253.23s | 1.277s | 90.00x |
| M9 (arc close) | no separate pass — M5's readings stood | 253.23s | 1.277s | 90.00x |

This table was `30UpdatePlan.md`'s own headline metric, filled in by
hand as each of that arc's milestones completed. It is frozen above as
a historical record and is **not maintained past this point** —
`31UpdatePlan.md` M9 retires it in favor of
[`docs/perf/baseline.json`](baseline.json), a machine-readable
baseline `ciac-bench --update-baseline` writes and
`ciac-bench --compare=<old-baseline.json>` diffs against automatically,
git-sha-stamped so a reader always knows which commit a given number
came from.

The table's own last paragraph, kept verbatim below, is the specific
reason a hand-maintained table stopped being an acceptable design for
tracking these numbers going forward — and is worth reading as the
rationale for why `31UpdatePlan.md` built the JSON baseline and its
`--compare` machinery rather than asking a future contributor to keep
this table honest by hand indefinitely:

> M9's row says what actually happened rather than what was planned.
> M5 stopped the arc early on the grounds that M2–M4 had already
> beaten the target, and M6/M7 were both conditional on M5 choosing to
> continue; no optimization landed between M5 and M9, so M9 closed the
> arc citing M5's numbers as the M1→M9 delta rather than re-running the
> sweep. The row read "pending" until long after the arc shipped,
> which is its own small lesson about a metric a document promises to
> carry: nothing failed, but the table stopped describing reality the
> moment the last milestone declined to measure and no one updated it.

For current numbers, see `docs/perf/baseline.json` directly, or run
`cargo run --release -p ciac-integration-tests --bin ciac-bench` — see
[`docs/perf/README.md`](README.md) for the full instrument index this
document is now one entry in.

The four slow test binaries this table once tracked got a second,
much later optimization pass at `33UpdatePlan.md` — that arc corrected
a wrong diagnosis `32UpdatePlan.md` M8 left in its own running log
(the shortfall it measured was attributed to core count; it was
actually Amdahl's law on a split axis holding only 4.6% of the
achievable win, unrelated to hardware) and cut the combined total a
further 30.8% (135.30s → 93.62s). See that document's M7 checkpoint,
mirrored in [`docs/perf/README.md`](README.md)'s own
`33UpdatePlan.md` M7 section, for the full numbers.
