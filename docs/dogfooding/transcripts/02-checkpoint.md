# Cold-start transcript 02 — checkpoint (post M2–M4)

> 29UpdatePlan.md M5. Same script as
> [transcript 01](01-baseline.md): install → README walkthrough
> end-to-end → guides 01–03 (the guide series didn't exist at M1;
> it does now, so it's in scope). Author-run, same mechanical-friction
> caveat as before. Run for real in this session's sandbox on
> 2026-07-28, against the repo state after M2 (friction fixes),
> M3 (README rewrite), and M4 (guides + veracity harness).

## What changed since transcript 01

### F1 — `curl | sh` install — unchanged (expected)

Still 404s, for the same reason: no release has been cut. This is
explicitly M9's job, not earlier. Re-confirmed, not re-fixed.

### F2 — `cargo install --path crates/ciac` fallback — fixed by M3's rewrite

The README's install block now states the toolchain requirement and
a rough time inline: `# or: cargo install --path crates/ciac (needs a
Rust toolchain; ~2 minutes)`. Measured this run: **2m41s** (vs 1m40s
at M1) — both readings are "warm cargo/registry cache," and the
difference is almost certainly this session's own source churn
forcing more of the workspace to recompile, not a real regression;
noted rather than hidden. The stated "~2 minutes" is close enough to
be useful and not so precise it invites nitpicking over a number that
depends on the reader's own machine.

### F3 — scaffold/check/build messaging — still good

Unchanged, re-confirmed: `ciac new`, `ciac check`, `ciac build` all
instant, all name the next command.

### F4 — `ciac verify` failing on every fresh project — fixed, confirmed live

`ciac verify` on the README's own quickstart example, and on a fresh
`ciac new --template minimal` service in guide 01, both pass clean.
No ruff errors. The M2 fix holds under real re-measurement, not just
the harness's own re-check.

### F5 — silent gap before `ciac dev` reports anything — fixed, confirmed live

```
$ ciac dev examples/single-service/quickstart.ciac --target python --out ./build
generated 34 files in ./build (python backend)
note: run the API with `uv sync && uv run uvicorn app.main:app`, or `docker compose up` for the full stack
note: start workers/jobs with `uv run python -m app.workers`
dev: starting the compose stack...
unable to get image 'postgres:16': failed to connect to the docker API at [...]
dev: docker compose up failed (exit status: 1) — fix and save to retry
dev: watching 1 source file(s) + seeded services (Ctrl-C to stop)
```

The `dev: starting the compose stack...` line (M2) now covers the
gap that was previously silent for 8-20s.

### F6 — `docs/authoring.md` staleness — unchanged (still deferred to M6)

Not yet touched; still on Pillar 3/7's list, as triaged at M1.

### New finding (F7) — the guide series' own forward-references were dead links

Found on re-read, not by the harness (the harness checks command
blocks, not prose links): `docs/guide/01-first-service.md` and
`03-handlers-and-logic.md` linked forward to `05-simulation.md` and
`04-streams-and-workers.md` — files that don't exist until M6.
Exactly the mistake M3's own README rewrite deliberately avoided
(no links to `docs/positioning.md` or the guide series until they
exist) but that discipline hadn't been carried into the guides
themselves when they cross-referenced *each other's future
installments*. **Fixed live during this transcript**: the dead links
are now plain, unlinked mentions ("a later guide in this series...")
instead of broken relative links — re-verified by grep for
`0[4-7]-` across `docs/guide/*.md` and `README.md`: zero matches.

## Wall-clock summary

| Step | M1 | M2 (this transcript) |
|---|---|---|
| `curl \| sh` install | 0.5s (fail) | 0.75s (fail, unchanged) |
| `cargo install` fallback | 1m40s | 2m41s (cache-state variance, not a regression) |
| `ciac new`/`check`/`build` | <10ms | <10ms (unchanged) |
| `ciac verify` (quickstart) | fails (F4) | **passes** |
| `ciac sim` (quickstart, relative `--out`) | not tested at M1 | **passes** (25s, uv sync cost) |
| `ciac dev` (no flags, Docker unreachable) | 8-20s silent | **instant progress line**, then Docker's own report |
| Guide 01 (install→verify) | guide didn't exist | <3s total, clean |

## Checkpoint decision: **go**

Every M1 fix-now item is closed and re-confirmed under fresh
measurement, not just re-run through the harness. The one new
finding (F7, dead forward-links) was small, caught by the same
"re-read with fresh eyes" discipline this checkpoint exists to apply,
and fixed on the spot rather than requiring a new milestone. Nothing
found here suggests the README's narrative shape or the guide
series' voice/structure is wrong — Pillar 2/3's shape holds up under
a second read. Guides 04–07 proceed on the validated shape at M6;
no re-fix queue carries forward beyond F1 (real release, M9's own
job) and F6 (authoring.md superseded by the coherence pass, M6).
