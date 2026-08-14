# Noise floor

*Reader: anyone about to pick a threshold for a performance gate, or
investigating why one fired when nothing regressed.*

`31UpdatePlan.md` M1. Every band this arc's gates use is derived from
the numbers in this document by one fixed rule, stated before any
number below was known:

> **Rule (pre-registered):** a gate fires at `max(3σ, 10%)` of the
> measured noise floor for its own metric, where σ is the standard
> deviation across this document's repetitions **on the noisier of the
> two environments measured**. If `3σ` for a metric exceeds 25%, that
> metric is not gateable by wall clock at all — it must move to
> instruction counting or be dropped to reporting-only.

## What this measures, and what it found

Two independent questions, kept separate because they have different
answers:

1. **Is wall-clock timing of the compiler's own phases stable enough
   to gate on?** Measured locally, 40 repetitions per metric, four
   examples (`ping`, `order-system`, `domain-orders`,
   `sim-three-service`), covering every phase `31UpdatePlan.md`
   Pillar 3 names.
2. **Are callgrind instruction counts stable enough to gate on
   instead?** Measured locally, 10 baseline repetitions plus three
   deliberately perturbed conditions.

The answers point in opposite directions, and that divergence is the
headline finding: **wall clock is not reliable at the operation sizes
this compiler runs at; instruction counting is.** This is not a
surprise this document manufactures to justify a decision already
made — it is the empirical case for the decision `31UpdatePlan.md`
Pillar 2 already made on first-principles grounds ("`ubuntu-latest` is
a shared, virtualised, 4-vCPU machine... instruction counts do not have
this problem"). M1's job was to check that reasoning against real
numbers rather than let it stand as an assertion, and it holds.

## Wall-clock noise, local (40 reps/metric)

Mean, standard deviation, relative standard deviation, min, max, p95,
in microseconds unless noted. Full data reproduced from the
measurement run; environment is stamped in "Environment" below.

### ping

| Metric | mean (µs) | σ | rel σ | min | max | p95 |
|---|---|---|---|---|---|---|
| `syntax::load` | 9.3 | 1.3 | 13.9% | 8.8 | 15.1 | 11.4 |
| `sema::analyze` | 2.0 | 1.0 | 50.4% | 1.8 | 8.5 | 2.0 |
| `model::build_system` | 2.7 | 0.1 | 5.2% | 2.5 | 3.3 | 2.9 |
| `semantic_model::from_ir` | 1.2 | 0.1 | 5.4% | 1.2 | 1.6 | 1.3 |
| `semantic_model::semantic_hash` | 6.5 | 3.7 | 57.5% | 5.6 | 29.4 | 7.5 |
| `generate()` python | 583.8 | 134.9 | 23.1% | 433.8 | 801.3 | 774.9 |
| `generate()` rust | 490.3 | 29.0 | 5.9% | 458.9 | 552.2 | 549.8 |
| `generate()` typescript | 516.7 | 49.3 | 9.5% | 477.9 | 755.8 | 576.9 |
| `generate()` go | 5841.2 | 556.3 | 9.5% | 5338.3 | 7395.1 | 7395.1 |

### order-system

| Metric | mean (µs) | σ | rel σ | min | max | p95 |
|---|---|---|---|---|---|---|
| `syntax::load` | 62.8 | 16.8 | 26.7% | 54.2 | 136.5 | 90.0 |
| `sema::analyze` | 88.8 | 15.8 | 17.8% | 78.5 | 167.2 | 113.9 |
| `model::build_system` | 60.3 | 9.4 | 15.6% | 54.8 | 98.1 | 83.3 |
| `semantic_model::from_ir` | 71.1 | 7.0 | 9.8% | 64.7 | 90.8 | 88.0 |
| `semantic_model::semantic_hash` | 66.9 | 18.1 | 27.0% | 57.4 | 159.3 | 86.8 |
| `generate()` python | 1399.7 | 221.6 | 15.8% | 1193.0 | 1880.1 | 1836.0 |
| `generate()` rust | 1508.8 | 125.2 | 8.3% | 1410.2 | 2167.1 | 1676.0 |
| `generate()` typescript | 1470.8 | 95.7 | 6.5% | 1378.9 | 1837.6 | 1707.5 |
| `generate()` go | 21791.6 | 3425.1 | 15.7% | 19413.5 | 31716.0 | 31716.0 |

### domain-orders

| Metric | mean (µs) | σ | rel σ | min | max | p95 |
|---|---|---|---|---|---|---|
| `syntax::load` | 50.4 | 9.7 | 19.2% | 44.4 | 90.4 | 77.9 |
| `sema::analyze` | 79.0 | 15.1 | 19.2% | 65.7 | 125.6 | 110.8 |
| `model::build_system` | 54.4 | 6.6 | 12.1% | 49.2 | 79.4 | 70.0 |
| `semantic_model::from_ir` | 63.0 | 14.4 | 22.8% | 55.3 | 143.6 | 84.2 |
| `semantic_model::semantic_hash` | 61.3 | 9.0 | 14.7% | 56.4 | 89.5 | 82.4 |
| `generate()` python | 1157.1 | 49.8 | 4.3% | 1098.2 | 1331.4 | 1258.6 |
| `generate()` rust | 1179.7 | 52.4 | 4.4% | 1120.1 | 1356.9 | 1276.9 |
| `generate()` typescript | 1227.9 | 41.1 | 3.3% | 1177.7 | 1334.5 | 1317.2 |
| `generate()` go | 17904.2 | 573.9 | 3.2% | 17123.1 | 18878.2 | 18878.2 |

### sim-three-service

| Metric | mean (µs) | σ | rel σ | min | max | p95 |
|---|---|---|---|---|---|---|
| `syntax::load` | 22.5 | 4.7 | 20.9% | 20.1 | 47.4 | 27.8 |
| `sema::analyze` | 30.6 | 4.5 | 14.6% | 28.0 | 53.7 | 37.8 |
| `model::build_system` | 46.5 | 11.6 | 24.8% | 41.0 | 99.8 | 70.6 |
| `semantic_model::from_ir` | 32.0 | 9.2 | 28.7% | 25.9 | 73.6 | 46.6 |
| `semantic_model::semantic_hash` | 32.0 | 5.8 | 18.2% | 29.7 | 50.8 | 49.7 |
| `generate()` python | 1209.8 | 156.8 | 13.0% | 1091.6 | 1725.7 | 1499.5 |
| `generate()` rust | 1390.6 | 139.9 | 10.1% | 1278.4 | 2116.2 | 1578.2 |
| `generate()` typescript | 1773.3 | 199.7 | 11.3% | 1529.2 | 2245.9 | 2216.2 |
| `generate()` go | 33250.0 | 2763.5 | 8.3% | 29194.2 | 38074.8 | 38074.8 |

### Whole-process `ciac build`, local vs. `ubuntu-latest` (20 reps, `ping`/python)

The phase table above times individual functions in-process; this row
times the real `./target/release/ciac build examples/ping.ciac
--target python --out <dir>` subprocess end to end — process startup,
front end, `generate()`, hashing, regen plan, manifest, and the actual
file writes together — on both environments, so the two are directly
comparable on the same metric for once.

| Environment | mean (µs) | σ | rel σ | min | max | p95 |
|---|---|---|---|---|---|---|
| Local sandbox | 11721.5 | 7977.4 | **68.1%** | 9054 | 46383 | 46383 |
| `ubuntu-latest` (one job, 20 reps) | 3586.1 | 414.8 | **11.6%** | 3241 | 4859 | 4859 |

**The local sandbox is the noisier of the two environments for this
metric** — the opposite of the naive assumption that a shared CI
runner is always noisier than a dedicated sandbox. The most credible
explanation: this measurement ran inside the same long-lived agent
session that was doing other work concurrently (background tasks, tool
calls) during the sweep, while the CI job is a freshly provisioned,
single-purpose VM running nothing else. The Pillar-1 rule exists
precisely so a case like this doesn't get judged by intuition — it
says "the noisier of the two environments," not "assume CI," and here
that means the **local 68.1% figure is what actually sets this
metric's band** (3σ = 204.3%, nowhere close to wall-clock-gateable,
consistent with every other metric in this document). Recorded here
undisguised, including the outlier that drove it (one 46.4ms sample
against a ~9-12ms median), because a noise-floor document exists to
report what was actually observed, not what was expected.

The CI run also confirms, independently, that a shared runner alone
isn't disqualifying: `ubuntu-latest`'s own 11.6% for the identical
operation is *tighter* than several of the local per-phase figures
above (e.g. `sema::analyze` at 50.4% on the same machine class this
document's phase table used). Noise is a property of what's being
measured and what else is running, not simply a property of which
machine it runs on.

## Applying the rule

`3σ` computed per metric, **worst case across the four examples above**
(the gate-relevant reduction — the actual gated corpus at M6/M7 is
smaller, `ping` and `order-system`; see that section below for the
narrower, decision-relevant view):

| Metric | worst rel σ | 3σ | Wall-clock verdict |
|---|---|---|---|
| `model::build_system` | 24.8% | 74.4% | **not gateable** |
| `generate()` go | 15.7% | 47.1% | **not gateable** |
| `generate()` python | 23.1% | 69.3% | **not gateable** |
| `generate()` rust | 10.1% | 30.3% | **not gateable** |
| `generate()` typescript | 11.3% | 33.9% | **not gateable** |
| `sema::analyze` | 50.4% | 151.2% | **not gateable** |
| `semantic_model::semantic_hash` | 57.5% | 172.5% | **not gateable** |
| `semantic_model::from_ir` | 28.7% | 86.1% | **not gateable** |
| `syntax::load` | 26.7% | 80.1% | **not gateable** |

**Every metric measured fails the 25% ceiling when judged across all
four examples.** Narrowed to exactly the two examples M6/M7 actually
gate (`ping`, `order-system` — see open question 2), a handful of
metrics are borderline wall-clock-gateable at wide bands
(`model::build_system` on `ping`: 3σ = 15.6%; `semantic_model::from_ir`
on `ping`: 3σ = 16.2%; `generate()` rust on `order-system`: 3σ = 24.9%,
right at the edge), but the operative finding is the same either way:
**most of what this arc wants to gate cannot be gated reliably by wall
clock at these operation sizes, full stop** — not because the
measurement was sloppy, but because sub-millisecond operations sit
close enough to timer and scheduler jitter that the signal-to-noise
ratio is genuinely poor, independent of how many reps are taken.

This is not a defect in the compiler or the measurement. It is the
reason `31UpdatePlan.md` Pillar 2 exists, now with real numbers behind
it rather than an argument from first principles alone. **Consequence
for M6/M7:** the uniform-regression gate and the asymptotic guard both
gate primarily on callgrind instruction counts (below); wall-clock
figures stay in `baseline.json` as reporting-only context, never as
the pass/fail condition, for every metric in this arc.

## Callgrind determinism

10 baseline repetitions (`ping`, python target, identical conditions),
plus three perturbed conditions (10 conditions × 3 reps each), all
local, `--cache-sim=no` (instruction count only, no cache simulation —
cache behavior is itself a source of run-to-run variance this arc does
not need for a pass/fail count).

| Condition | Reps | Instructions (min–max) |
|---|---|---|
| Baseline | 10 | 12,972,079 – 12,972,181 |
| Long `$PWD` (200-char dir) | 3 | 12,972,039 – 12,972,320 |
| Inflated env block (+4000 bytes) | 3 | 12,972,672 – 12,972,886 |
| ASLR disabled (`setarch -R`) | 3 | 12,972,079 – 12,972,181 |

Baseline drift (max − min, relative to baseline mean): **0.00079%.**
Overall drift across every condition combined, relative to baseline
mean: **0.00653%.**

**Open question 1, answered: 0.00653% is far under the pre-registered
2% threshold.** Callgrind gating proceeds as designed at M6 — no
fallback to same-runner merge-base comparison is needed. The one
condition that moved the needle at all (inflated environment block,
+4000 bytes shifting the count by ~0.006%) is consistent with
`execve`'s own environment-copy cost scaling with block size, not with
any instability in the measured program's own logic — reassuring
rather than concerning, since it means the small drift that does exist
has a mechanistic explanation rather than being unexplained jitter.

**Confirmed independently on real `ubuntu-latest` hardware.** The same
5-rep callgrind sample, taken by the `perf-noise-floor` CI job on a
completely different machine (different CPU microarchitecture, glibc
build, kernel): 12,954,681 / 12,954,676 / 12,954,676 / 12,954,953 /
12,954,757 instructions — drift **0.00214%**, same order of magnitude
as the local baseline's 0.00079%. The absolute count differs from the
local baseline by **−0.134%** (12,954,749 mean vs. 12,972,120 mean),
which is exactly the kind of small, expected, machine-specific offset
`docs/perf/codegen-baseline.md`'s own standing disclosure already
anticipates — different glibc/kernel syscall paths taking marginally
different instruction counts for the same logical work — and is
irrelevant to gating, since M6's gate compares a build against its own
prior build on the same machine class, never against a number from a
different one.

## Provisional bands for M6/M7

Derived mechanically from the table above, to be confirmed (not
re-derived from scratch) at the M5 checkpoint once `ubuntu-latest` data
is folded in:

- **Callgrind instruction counts (`ping`, `order-system`):** band set
  at **1%** of the measured instruction count for each gated example.
  This is roughly 150× wider than the largest observed drift
  (0.00653%), which mirrors the existing `perf_budget.rs`'s own stated
  philosophy — "comfortably above today's steady state so ordinary
  variance never trips it" — applied to a metric two orders of
  magnitude more stable than wall clock, not copied from a wall-clock
  band that doesn't apply here.
- **Wall-clock `generate()` figures:** recorded in `baseline.json` for
  every backend and example, reporting-only, no pass/fail band. A
  reader investigating a regression uses these as corroborating
  evidence, not as the thing that failed.
- **Asymptotic guard (M7):** ratio-based and runner-independent by
  construction, so it needs no band from this document at all — but
  the corpus M7 picks for its N/2N/4N sweep should favor operation
  sizes large enough that wall-clock noise doesn't swamp the ratio
  signal. `sema::analyze` on `ping` (2.0µs mean, 50.4% rel σ) is far
  too small an operation to trust a ratio computed from it; the
  synthetic corpus's smallest N should target at least
  low-hundreds-of-microseconds per measured call, consistent with
  where this document's own `order-system`/`domain-orders` figures
  already sit.

## How this was measured

Local sweep: a phase-level timer instrumenting the same entry points
`31UpdatePlan.md` Pillar 3 names
(`ciac_syntax::load`, `ciac_sema::analyze`,
`ciac_codegen::model::build_system`,
`ciac_codegen::semantic_model::SemanticModel::from_ir`/`.semantic_hash()`,
`Backend::generate` per backend), release profile, 2 discarded
warm-up calls then N timed calls per metric, min/max/mean/σ/p95
computed over the N samples. This tool is *not yet checked in* —
promoting it to a committed, reusable harness is M2's own job
(`tests/bin/ciac-bench.rs`); M1's only obligation is trustworthy
numbers, not a permanent instrument.

`ubuntu-latest` sample: one non-blocking CI job
(`perf-noise-floor` in `.github/workflows/ci.yml`), reusing the
already-built `ciac` release binary directly rather than adding new
committed source, so this milestone's diff stays CI-config-only. Per
`31UpdatePlan.md`'s own tension-2 resolution: this is deliberately
**not** a dedicated multi-push polling campaign. A campaign sampling
many independent `ubuntu-latest` scheduling instances would cost tens
of minutes of session time to characterize *between-instance*
variance the plan itself already discounts ("absolute values are
machine-specific and only ratios travel"). Instead: this one job
supplies genuine runner hardware and within-instance noise cheaply,
and the arc's own ordinary one-push-per-milestone cadence supplies a
free, opportunistic sample of between-instance variance by the time M9
closes. `docs/perf/README.md`'s M9 update carries a short postscript
once that opportunistic sample exists.

## Environment

Local: 4 logical CPUs, Intel Xeon @ 2.80GHz, **no SHA-NI**
(`grep sha_ni /proc/cpuinfo` — empty), 15GB RAM, Linux 6.18.5 x86_64,
`rustc 1.94.1`, release profile (`lto = "thin"`), valgrind 3.22.0. The
SHA-NI absence is the same fact behind `docs/perf/codegen-baseline.md`'s
disclosed 39.79%-of-a-build hashing figure — recorded again here
because it is exactly the kind of fact this document exists to make
unmissable rather than merely mentioned once and forgotten.

`ubuntu-latest`: whatever GitHub Actions provisions at the time of the
run this milestone's own commit triggers — captured by the workflow
job itself, not hand-transcribed, and not independently pinned by this
document (the whole point of running the job is to observe whatever
GitHub actually gives us, not to assume it).

## Gate decisions (M5 checkpoint)

`31UpdatePlan.md` M5: a hard stop before M6/M7 write a line of gate
code. For each proposed gate, three numbers — measured noise, the band
derived from it, the smallest regression that band would reliably
catch — and one of the plan's four pre-registered outcomes. Nothing
below re-derives a number already established above; this section
only decides what to build from numbers already in hand.

### A deviation from the pre-registered rule's literal text, stated plainly

The rule as pre-registered in Pillar 1 reads `max(3σ, 10%)`. Applied
literally to callgrind, the `10%` floor term would win outright —
callgrind's own measured noise (0.00079% local, 0.00214% on
`ubuntu-latest`, 0.00653% across every perturbed condition combined)
is so far below 10% that `3σ` never approaches it. A 10%-wide gate on
a metric this stable would only fire on a tenfold regression, which
defeats the purpose of building a dedicated instruction-count gate at
all — the existing `perf_budget.rs` already catches damage that large.

The `10%` floor exists to guard against an *under-sampled* noise
estimate producing false confidence — a real risk for wall-clock
metrics, where this document's own 40-repetition local sample still
carries real sampling uncertainty. That risk does not transfer
unchanged to callgrind: its determinism was independently confirmed on
two unrelated machines (this sandbox and a real `ubuntu-latest` VM,
different CPU microarchitecture, glibc build, and kernel), which is a
stronger cross-check than any single environment's repetition count
alone could provide. Given that, the floor is re-expressed for
instruction-count metrics as a fixed multiplier over observed drift
(150×) rather than a flat 10% — the same "comfortably above ordinary
variance, comfortably below a real regression" philosophy
`perf_budget.rs`'s own `BUDGET_MULTIPLIER` doc comment already states,
scaled to callgrind's actual noise floor instead of copying a band
calibrated for wall clock. This is the deviation, stated once here so
it is never silently rediscovered three commits later: **the
pre-registered rule's floor term applies to wall-clock metrics as
written; for instruction-count metrics it is replaced by this
150× multiplier, and that replacement — not the literal `10%` — is
what M6 implements.**

### Gate 1 — the uniform-regression gate (M6, callgrind)

| | |
|---|---|
| Measured noise | 0.00079% (local baseline) / 0.00214% (`ubuntu-latest`) / 0.00653% (worst case, all perturbed conditions) |
| Derived band | 1% of measured instruction count, per gated example |
| Smallest regression reliably caught | ≥1% instruction-count growth on `ping` or `order-system` — by design, not the tightest theoretically detectable value (which the noise floor alone would put closer to 0.05–0.1%), for the same wide-headroom reason `perf_budget.rs` chose 1000× over a tighter multiplier |
| Outcome | **(a) proceeds** |

A 1% band sitting ~150× above the worst observed drift is not a
timid choice — every inventory item this arc exists to eventually gate
(lazy template loading, hash deduplication, `build_system`
deduplication) is expected to move generation cost by double-digit
percentages when it lands, per the phase-level numbers already in
`docs/perf/baseline.json`. A 1% band catches all of them with room to
spare and essentially zero false-positive risk given the measured
drift.

### Gate 2 — the asymptotic guard (M7, ratio-based)

| | |
|---|---|
| Measured noise | N/A directly — runner-independent by construction (a ratio computed within one run), but see reasoning below |
| Derived band | Ceiling set above today's measured ~3.7×/doubling for `sema::analyze` (this session's own synthetic-corpus finding, to be reproduced by M7's own generator, shared with `ciac-bench` per open question 4) |
| Smallest regression reliably caught | Any exponent increase distinguishable from measurement noise on the *mean* of repeated calls — see below |
| Outcome | **(a) proceeds** |

This gate does not sidestep wall-clock noise by ignoring it — it
survives the noise this document found by a mechanism worth stating
explicitly: `measure()` (`tests/src/bench.rs`) reports the **mean**
over `reps` repetitions, and a mean's own uncertainty shrinks with
`1/√reps`, not with the raw per-sample relative standard deviation.
`sema::analyze` on `ping` measured 50.4% relative standard deviation
per sample at 40 repetitions — but the *mean's* relative standard
error is closer to `50.4% / √40 ≈ 8%`. An 8% wobble in a mean estimate
does not threaten detecting a multi-fold change in a growth exponent.
This is also why the "Provisional bands" section above already steers
M7's own corpus toward operations of at least low-hundreds-of-
microseconds: it is not about avoiding noise outright, it is about
keeping the per-sample noise low enough that averaging over a
practical repetition count converges quickly instead of needing
thousands of reps to be trustworthy.

### Gate 3 — `perf_budget.rs` (existing, kept)

Not a decision this checkpoint makes — it already exists, already
gates, and already works on any runner because it compares backends to
each other on the same run rather than to an absolute number. M9
revisits its multiplier once M6's own instruction-count data is
available to inform that revisit; this checkpoint changes nothing
about it now.

### Overall outcome

**Pre-registered outcome (b): the callgrind gate proceeds, and every
wall-clock portion of this arc's measurements — the full phase table,
every `generate()` figure, the whole-process build timings — stays
reporting-only, permanently, never a pass/fail condition.** M1's own
finding forced this rather than leaving it a judgment call: every
metric measured failed the 25% wall-clock-gateable ceiling across the
full four-example corpus, and even narrowed to the two examples M6/M7
actually gate, only a handful of metrics sit at the ragged edge of
gateable. The asymptotic guard (M7) is the one gate that operates in
wall-clock space at all, and it does so by ratio-of-means, not by
raw-sample thresholding — a different enough mechanism that M1's
"wall clock is not wall-clock-gateable" finding does not disqualify
it.

This is a hard serialization point: M6 and M7 both proceed as designed
below, and neither starts before this section is committed.
