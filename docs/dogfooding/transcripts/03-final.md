# Cold-start transcript 03 — final (release candidate, v0.27.0)

> 29UpdatePlan.md M9. Same script family as
> [transcript 01](01-baseline.md) and [transcript 02](02-checkpoint.md):
> install → README walkthrough end-to-end → guides 01–05 (the guide
> series is now complete; 05 is simulation, this arc's "hook" phase in
> `DOGFOODING.md`). Author-run, same mechanical-friction caveat as
> before — this measures what a script can find, not what a real
> stranger would find; that gap is exactly why `DOGFOODING.md` exists.
> Run for real in this session's sandbox on 2026-07-28 against the
> repo at the M9 release-candidate state (version 0.27.0, `cargo
> install --path crates/ciac --force`, real binary on `PATH`).

## What changed since transcript 02

### F1 — `curl | sh` install — unchanged (expected)

Still 404s: no release has been cut yet. Same as M1/M2. M9 is the
version bump; the actual tag/release is a separate, explicitly-gated
step this session surfaces to the user rather than doing unprompted
(see `29UpdatePlan.md`'s own M9 Shipped note).

### F2 — `cargo install --path crates/ciac` fallback — re-measured

**1m33.6s** this run (real 5.1MLoC-ish workspace release build,
warm registry cache, cold `target/release` for this arc's own
changes). Comparable to M2's 2m41s; both readings are "warm
cargo/registry cache, cold incremental build," and the difference is
churn-dependent, not a trend. Still within the README's stated
"~2 minutes."

### F3–F6 — re-confirmed, no regressions

`ciac new`/`check`/`build` instant; `ciac verify` on both the
README's quickstart and guide 01's minimal template pass clean;
`ciac dev`'s instant progress line still covers the compose-stack
gap. All re-verified live this run via `scripts/check-guides.sh
README.md docs/guide/01-first-service.md docs/guide/05-simulation.md`:
**10 blocks run, 3 skipped (disclosed), 0 failed**, 8.3s total.

### New finding (F8) — spurious orphan-migration warning on the README's own two-step walkthrough, found and fixed live

Running the README's exact documented sequence —
`ciac build examples/single-service/quickstart.ciac --target python --out ./build`
immediately followed by
`ciac sim examples/single-service/quickstart.ciac --target python --out ./build
--scenario sim/quickstart.ciac-sim.json` (both `--out ./build`, per
the README's own text) — printed, before this transcript's fix:

```
warning[CIAC0035]: generated file app/migrations/0001_migration.sql is no longer produced and was left in place
warning[CIAC0035]: generated file tests/system/app/migrations/0001_migration.sql is no longer produced and was left in place
[PASS] 29-m3-quickstart
```

Reproduced against a completely clean output directory (not a
transcript artifact) with just two calls to `ciac build` back to
back, zero source changes between them — the warning fired on the
*second* invocation every time, and would keep firing on every future
build for the life of any project with a migration. Root cause: a
migration file is written once (on the schema-diff that creates it)
and correctly never re-emitted afterward — that's how migrations are
supposed to work — but the regeneration-orphan check couldn't tell
"a migration file whose permanent steady state is 'not regenerated
this time'" apart from "a stale seeded scaffold the user should
investigate," so it warned about the first case forever.

This is exactly the class of finding this transcript exists to catch
before a real human sees it: mechanical, reproducible on the
headline demo, and — because it re-fires on every build once a
project has any migration — arguably worse than a one-time surprise,
since a new user would see it *repeatedly* and have no way to make it
go away short of deleting a file they were explicitly told not to
touch. Fixed live this milestone: `FileRole` gained a `Migration`
variant (`crates/ciac-codegen/src/project.rs`) distinct from
`Seeded`, threaded through `regen.rs`'s classification and
`is_warning()` and `commands.rs`'s CLI-facing report, so a migration's
`OrphanLeft` state stays silent while a genuinely-stale `Seeded`
scaffold still warns (both directions covered by new tests in
`tests/tests/regen.rs`: `orphaned_migration_file_does_not_warn`,
`orphaned_seeded_scaffold_still_warns`). Re-verified after the fix:
the same two-command sequence, and a bare second `ciac build` with no
source changes at all, both produce zero warnings.

## Wall-clock summary

| Step | M1 | M2 | M9 (this transcript) |
|---|---|---|---|
| `curl \| sh` install | 0.5s (fail) | 0.75s (fail) | 0.9s (fail, unchanged — real release still not cut) |
| `cargo install` fallback | 1m40s | 2m41s | 1m33.6s (cache-state variance across all three, not a trend) |
| `ciac new`/`check`/`build` | <10ms | <10ms | <10ms (unchanged) |
| `ciac verify` (quickstart) | fails (F4) | passes | passes |
| Guide 01 (install→verify) | guide didn't exist | <3s total | build+verify 1.76s (2 tests) |
| Guide 05 (simulation) | guide didn't exist | guide didn't exist | new+sim, harness-timed within the 8.3s combined run |
| README build→sim→verify (quickstart) | not run this way | not run this way | 0.008s / 1.7s / 1.4s — **zero warnings** (F8 fixed) |
| Full veracity harness (README + guide 01 + guide 05) | harness didn't exist | harness existed, guides 01–03 only | **10 run, 3 skipped, 0 failed, 8.3s** |

## Delta table: 01 → 02 → 03

| Finding | 01 (baseline) | 02 (checkpoint) | 03 (final) |
|---|---|---|---|
| F1: `curl \| sh` install 404s | found | unchanged (M9's own job) | unchanged (M9's own job — real tag/release gated on user sign-off) |
| F2: `cargo install` fallback undocumented | found | **fixed** (inline note) | holds |
| F3: scaffold/check/build messaging | found (good already) | holds | holds |
| F4: `ciac verify` failing on fresh projects | found | **fixed** | holds |
| F5: silent gap before `ciac dev` reports anything | found | **fixed** | holds |
| F6: `docs/authoring.md` staleness | found | deferred to M6 | **fixed** (M6 coherence pass; M8 rewrote it again for the widened editor feature set) |
| F7: guide series' dead forward-links | n/a (guides didn't exist at M1) | found + **fixed live** | holds (re-checked: `grep` for `0[4-7]-` across `docs/guide/*.md` and `README.md`, zero matches) |
| F8: spurious orphan-migration warning on every rebuild | not found (quickstart's inline `transaction` was M9-new; this exact build→sim sequence wasn't run at M1/M2 in a from-scratch directory) | not found | found + **fixed live** |

Every finding from M1 through M9 is closed. Two new findings (F7 at
M5, F8 at M9) were caught by the same discipline that makes these
transcripts worth running at all — re-reading and re-executing with
fresh eyes at each checkpoint — and both were fixed on the spot rather
than carried forward. Nothing in this transcript suggests a
structural problem with the README's shape, the guide series' voice,
or the simulation walkthrough's design; what a script can find, it
found, and this arc closed it. The remaining gap is categorical, not
incremental: only a real stranger's conceptual confusion — not
another author re-read — can tell us what's left. That is exactly
`DOGFOODING.md`'s job, not this transcript's.
