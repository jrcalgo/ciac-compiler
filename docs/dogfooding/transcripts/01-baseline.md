# Cold-start transcript 01 — baseline (v0.26-era front door)

> 29UpdatePlan.md M1. Scripted, author-run (not a real outside
> human — see `DOGFOODING.md`, landing at M9, for that distinction).
> This transcript measures *mechanical* friction only: broken
> commands, missing prerequisites, misleading or missing output,
> slow steps. Every command below was actually executed against the
> real `ciac` binary in this session's sandbox on 2026-07-28; output
> is verbatim except where noted `[...]` for length. No fixes were
> made while gathering this transcript — measurement first, per M1's
> own rule, so the M2/M5/M9 deltas mean something.
>
> Caveat on "clean container": this sandbox already has the Rust
> toolchain, a warm `cargo`/`uv` cache, and a checkout of the repo —
> it is not a bare machine. Where that materially affects a timing
> (e.g. `cargo install` reusing a warm registry cache), it's called
> out. The `curl | sh` step, `ciac new`, `ciac check`, `ciac build`,
> and `ciac dev`'s messaging are unaffected by that caveat — they
> measure `ciac`'s own behavior, not the host's warmth.

## Script followed

1. `curl -fsSL .../install.sh | sh` — README's documented first line.
2. Fallback: `cargo install --path crates/ciac` — README's documented
   second line, since (1) fails (see finding F1).
3. `ciac new my-app && cd my-app` — README's quick start.
4. `ciac check main.ciac`
5. `ciac build main.ciac --target python --out ./build`
6. `ciac dev main.ciac --target python --out ./build` — README's
   quick start's last line, exactly as documented (no flags).
7. The scaffolded README's own suggested next step,
   `ciac verify main.ciac --target python --out ./build` — not in
   the top-level README's quick start, but *is* what `ciac new`
   itself tells a reader to do next, so it's in scope as "the
   closest thing to the guide-01 path that exists today."
8. `docs/authoring.md` read cold, as the nearest existing substitute
   for a guide-01 ("first service") walkthrough.

## Findings

### F1 — `curl | sh` install fails outright (fix-now)

```
$ curl -fsSL https://raw.githubusercontent.com/jrcalgo/ciac/main/install.sh -o install.sh
curl: (22) The requested URL returned error: 404
```

Expected and already disclosed (26 M8/M9's own retrospective: zero
git tags exist, `release.yml` has never fired, `install.sh` depends
on a `releases/latest` that doesn't exist). Confirmed again here as
the literal first command a reader following the README today would
run, and it is the literal first thing that fails. Also note: the
raw file fetch itself 404s (not just `releases/latest`), which this
sandbox cannot fully diagnose (its GitHub access is proxied and
scoped — `api.github.com` returns "GitHub access is not enabled for
this session" here) — recorded as-observed, not as a claim about the
public repo's visibility. **Wall clock: 0.5s to fail.**

Triage: **fix-via-rewrite** (M9 cuts the real v0.27.0 release, which
is the actual fix — this transcript's job is only to confirm the
symptom is real and re-measure it at M9). No action needed in M2.

### F2 — `cargo install --path crates/ciac` works, but is a 100-second silent wait with no progress framing (fix-via-rewrite)

```
$ cargo install --path crates/ciac --root <dir>
   Compiling bitflags v2.13.1
   [... 30 crates ...]
    Finished `release` profile [optimized] target(s) in 1m 39s
  Installing <dir>/bin/ciac
   Installed package `ciac v0.26.0 (/home/user/ciac/crates/ciac)` (executable `ciac`)
warning: be sure to add `<dir>/bin` to your PATH [...]
```

**Wall clock: 1m40s** (warm registry cache — a genuinely cold
machine would add the crates.io index fetch and download time on
top). `cargo`'s own compile output is fine on its own terms, but the
README presents this as an equal-weight bullet alongside the
`curl | sh` one-liner ("# or: cargo install --path crates/ciac")
with no framing that it's a >1-minute wait, and no mention that it
requires a Rust toolchain the reader may not have. A reader whose
first command failed (F1) lands here next with zero warning about
either the toolchain prerequisite or the wait.

Triage: **fix-via-rewrite** — Pillar 2's README rewrite is exactly
the place a "if X fails, here's what to expect from the fallback"
framing belongs; not a code fix.

### F3 — `ciac new`, `ciac check`, `ciac build` are fast and their messaging is already good (no finding)

```
$ ciac new my-app
scaffolded the `crud` template into my-app
next: cd my-app && ciac check main.ciac

$ ciac check main.ciac
main.ciac: no errors

$ ciac build main.ciac --target python --out ./build
generated 21 files in ./build (python backend)
note: run the API with `uv sync && uv run uvicorn app.main:app`, or `docker compose up` for the full stack
```

**Wall clock: <10ms each.** All three name the next command. Noted
as a positive baseline, not a finding — nothing to fix, nothing to
regress against at M5/M9.

### F4 — the freshly generated project fails its own documented next step: `ciac verify` (fix-now, highest priority)

The scaffolded `README.md` (written by `ciac new` itself) says:

```
## Next steps
ciac check main.ciac
ciac build main.ciac --target python --out ./build
ciac verify main.ciac --target python --out ./build
```

Running that exact third line, on the exact project the second line
just built:

```
$ ciac verify main.ciac --target python --out ./build
B008 Do not perform function call `Depends` in argument defaults [...]
  --> app/api/note.py:22:29
[... 16 more B008/I001/UP037 errors across app/api/note.py, app/auth.py,
     app/main.py, app/models.py, app/services/note_store.py, app/state.py ...]
Found 18 errors.
[*] 5 fixable with the `--fix` option.
error: `uv run ruff check .` failed in ./build (lints)
```

**Wall clock: 0.4s to fail.** This is not scaffold-specific or
provider-specific — the same 18 errors, at the same lines, reproduce
against the checked-in `examples/crud-notes.ciac` (the exact program
`ciac new --template crud` embeds verbatim, confirmed against
`docs/authoring.md`'s own claim) run through `ciac verify` directly
in this repo. Root cause, confirmed by inspection:
`crates/ciac-backend-python/templates/pyproject.toml.j2` pins
`"ruff>=0.6"` with no upper bound and no lockfile, and its
`[tool.ruff]` block sets only `target-version` — no explicit
`select`/`extend-select`. `uv run ruff --version` in the generated
project resolves **ruff 0.16.0** today. Between whenever the
templates were last hand-verified against ruff's then-current
defaults and ruff 0.16.0, ruff's own default/implied rule set
picked up findings (`B008` bugbear-style, `I001` import sorting,
`UP037` pyupgrade) that the generated code now trips on — in
generated code the reader never touched. A brand-new reader
following the scaffold's own README, verbatim, on a fresh machine
today, hits a wall of lint errors in code they didn't write, with
no indication these are pre-existing (not something they broke).

Triage: **fix-now (M2)**. Two independent fixes worth considering at
M2 (not decided here — measurement only): pin an exact/narrower
`ruff` version so template and lint config move together
deliberately, and/or extend `[tool.ruff]` with an explicit `select`
so new ruff defaults can't silently start failing already-shipped
templates. Either way this is the single highest-priority item in
this transcript: it breaks the documented golden path for every
target's every generated project today, not just `crud`/Python.

### F5 — `ciac dev`, run exactly as the top-level README documents (no `--no-docker`), prints nothing for several seconds when Docker's daemon isn't reachable (fix-now, moderate)

```
$ ciac dev main.ciac --target python --out ./build
[... 8s of total silence on both stdout and stderr ...]
[killed]
```

Given a longer window, the real sequence appears — it wasn't hung,
just slow to report:

```
$ ciac dev main.ciac --target python --out ./build     # 25s window
generated 21 files in ./build (python backend)
note: run the API with `uv sync && uv run uvicorn app.main:app`, or `docker compose up` for the full stack
unable to get image 'redis:7': failed to connect to the docker API at unix:///var/run/docker.sock; check if the path is correct and if the daemon is running: dial unix /var/run/docker.sock: connect: no such file or directory
dev: docker compose up failed (exit status: 1) — fix and save to retry
dev: watching 1 source file(s) + seeded services (Ctrl-C to stop)
```

This sandbox has the `docker` CLI installed but no reachable daemon
— a realistic "clean container" state (CI runners, minimal dev
containers, and this very session all match it), and not a
configuration this transcript should have needed to special-case to
find. The failure message once it arrives is clear and actionable;
the problem is the **silent gap beforehand** — a first-time reader
who just typed the README's last documented command has no signal
anything is happening until Docker's own connection attempt times
out. Contrast with `--no-docker` (not mentioned in the top-level
README's quick start at all, only discoverable via `--help` or
`docs/dev-loop.md`), which reports instantly:

```
$ ciac dev main.ciac --target python --out ./build --no-docker
generated 21 files in ./build (python backend)
note: run the API with `uv sync && uv run uvicorn app.main:app`, or `docker compose up` for the full stack
dev: regenerated (--no-docker: not starting the stack)
dev: watching 1 source file(s) + seeded services (Ctrl-C to stop)
```

Triage: **fix-now (M2)** for a "starting the compose stack..."
progress line printed before the Docker attempt begins (cheap,
mechanical); **fix-via-rewrite** for whether the quick start should
mention `--no-docker` at all, or lead with it in a docker-optional
example — Pillar 2's call, not this milestone's.

### F6 — `docs/authoring.md`, read as "the closest thing to guide-01," is itself stale (fix-via-rewrite)

`docs/authoring.md` opens `# Authoring CIaC (v0.13)` and states
outright that rename, references, and code actions are "deliberately
out of scope" for `ciac lsp` — both false as of v0.18 (rename) and
v0.15 M7 (structured-fix quick-fixes; code actions exist). It is
otherwise a good, focused document — accurate on `ciac new`,
`ciac lsp` hover/completion, and editor setup — just frozen at the
version in its own title while the LSP grew around it. This is
exactly Pillar 3/7's job (a real guide-01 replaces the "closest
thing," and the coherence pass in M6 catches cross-doc staleness
like this one), not a fix-now patch to a doc that's about to be
superseded.

Triage: **fix-via-rewrite** (M4's guide-01, M6's coherence pass).

## Wall-clock summary

| Step | Time | Notes |
|---|---|---|
| `curl \| sh` install | 0.5s | fails (F1) |
| `cargo install --path crates/ciac` | 1m40s | warm cache; fallback path (F2) |
| `ciac new my-app` | <10ms | |
| `ciac check main.ciac` | <10ms | |
| `ciac build ... --target python` | <10ms | |
| `ciac verify ... --target python` | 0.4s | **fails** (F4) |
| `ciac dev ...` (no flags, Docker unreachable) | ~8-20s silent, then reports | (F5) |
| `ciac dev ... --no-docker` | <10ms to report | contrast case for F5 |

## Triage summary

- **fix-now (M2 queue):** F4 (ruff drift breaks `ciac verify` on
  every fresh project — highest priority), F5 (silent gap before
  Docker's own failure surfaces).
- **fix-via-rewrite (Pillars 2/3/6/7):** F1 (real release, cut at
  M9), F2 (README framing of the cargo-install fallback), F6
  (authoring.md superseded by guide-01 + the coherence pass).
- **defer-with-reason:** none this round — every finding had a home
  in an already-planned milestone.

Re-measured at M5 (checkpoint, against the new README + guides
01-03) and M9 (final, against the v0.27.0 release candidate).
