# The inner loop: `ciac dev`

`ciac build` + `ciac verify` is a fine loop by hand, but iterating on
a running system means re-running both and restarting the stack every
time. `ciac dev` (v0.13 M4) automates that cycle: watch the program's
source, regenerate on change, restart the compose stack, and re-probe
every service's health — while never taking down a working system
over a compile error.

```sh
ciac dev main.ciac --target python --out ./build
```

## What one cycle does

1. **Compile** the program (the same front end `ciac check` runs).
   A compile error prints diagnostics and **keeps the last good stack
   running** — nothing is torn down, nothing is regenerated, and the
   watch keeps going so the next save can retry.
2. **Regenerate** through the exact path `ciac build` uses — manifest
   and sidecar discipline included, so `ciac dev` can never clobber an
   edit `ciac build` would have protected (see
   [docs/regeneration.md](regeneration.md)).
3. **Restart** the stack: `docker compose up -d --build --wait`.
   Restarts are deliberately whole-stack — compose itself only
   recreates containers whose image or configuration actually changed,
   which keeps the conservative choice cheap in practice. Per-service
   restart heuristics are out of scope.
4. **Probe** every service's `/health` route with a bounded backoff
   and print per-service up/DOWN.

## What's watched

The resolved source set: the entry file plus every file it
transitively `import`s (virtual entries — `std/...` blueprints,
`registry:...` imports — have no file to watch and drop out
naturally), plus the generated project's seeded `services/`
directories (so editing a seeded handler implementation also triggers
a restart-and-reprobe, even though it doesn't trigger a recompile).

## Flags

| Flag | Effect |
|------|--------|
| `--keep` | Leave the compose stack running on exit instead of tearing it down. |
| `--no-docker` | Watch and regenerate only — never touch Docker. For pairing with a hand-run process, or a zero-container program (`db SQLite` with no other containerized capability). |
| `--poll` | Use filesystem polling instead of native change events, for filesystems where inotify/FSEvents misbehave (network mounts, some container bind-mounts, some CI sandboxes). |

## The `--poll` baseline edge

A polling watcher establishes its baseline on its *first* tick after
registration (a 500ms interval). An edit landing inside that window is
absorbed into the baseline rather than detected as a change — the
*next* save picks it up correctly. This is inherent to polling, not a
bug to chase further; if a save right after starting `ciac dev
--poll` seems to do nothing, save again.

Set `CIAC_DEV_TRACE=1` to log every raw filesystem event to stderr —
the first thing to check when a save isn't triggering a rebuild.

## Relationship to `ciac verify`

`ciac dev` is for iterating with a live stack in front of you;
`ciac verify` is the truth signal for "does this still work" (CI,
pre-commit, or your own gut check before calling something done) —
see [docs/deployment.md](deployment.md) for `--system`/`--live`. They
regenerate through the same path but serve different moments: `dev`
optimizes for feedback while you're changing things, `verify`
optimizes for a trustworthy yes/no once you've stopped.
