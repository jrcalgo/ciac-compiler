# CIaC v0.6 — Living Projects: Regeneration, Scheduled Jobs, Realtime Channels

## Where v0.5.1 leaves us

The compiler holds one invariant: **whatever `ciac build` accepts, the
generated system actually does.** Records, streams, pipelines, `match`,
named capability instances, the ontology runtime (S3/SMTP/OpenSearch/
external HTTP), per-service deployables, and typed `call` clients all
generate working, validated code in both backends. `scheduler` and
`realtime` parse and check but gate at build (CIAC0011), and the
regeneration story is all-or-nothing: `ciac build` refuses a non-empty
output directory unless `--force` blindly overwrites it
(`crates/ciac/src/commands.rs:75-91`).

That last point is the existential gap. A compiler whose output can only
be generated once is a scaffolder; the moment a user edits a handler
stub, regeneration destroys their work or is abandoned. Every goal of
the project — whole systems from `.ciac`, fully implemented composable
code, compiler-guaranteed function — depends on the generated project
staying *compilable again tomorrow*.

**v0.6 theme: the generated project becomes a living artifact.** Three
pillars, ordered by importance:

1. **Regeneration** — `ciac build` can rebuild into a modified project,
   preserving user code, detecting drift, and failing loudly instead of
   destroying work.
2. **Scheduled jobs** — the `scheduler` capability gets its promised
   language construct and un-gates.
3. **Realtime channels** — the `realtime` capability gets its promised
   language construct and un-gates.

After v0.6 there are **zero gated constructs except Kafka**: the
language surface and the generated runtime are congruent again.

## Goal alignment

| Project goal | v0.6 contribution |
|--------------|-------------------|
| 1. `.ciac` formulates entire systems | jobs + channels close the last check-only holes; the whole v0.4 ontology is now usable end-to-end |
| 2. Compile into a host language | both backends gain scheduler/realtime runtimes; emission stays byte-deterministic |
| 3. Fully implemented, composable, DRY | ownership manifest formalizes the compiler-owned / user-owned seam that v0.7's expression language and v0.8's blueprints build on |
| 4. Fully functioning systems, compiler-guaranteed | `ciac verify` makes the guarantee *re-checkable after human edits*, not just at first generation |

---

## M1 — Ownership manifest + safe regeneration (`ciac-codegen`, `ciac` CLI)

### Design

Every generated file gets a **role**, declared by the backend at
`add_file` time:

- **`Owned`** — compiler-owned wiring (routes, workers, config, state,
  compose, schemas, clients, README). Regeneration may rewrite it, but
  only when the on-disk copy is untouched since the last build.
- **`Seeded`** — generated once, then owned by the user (handler stubs
  in `app/services/` / `src/services/`, `.env` if we ever emit one).
  Regeneration never rewrites it.

`GeneratedProject` changes (`crates/ciac-codegen/src/project.rs`):

```rust
pub enum FileRole { Owned, Seeded }
pub fn add_file(..)            // unchanged, defaults to Owned
pub fn add_seeded_file(..)     // handler stubs move to this
```

Backends only change which call they use for `service.py.j2` /
`service.rs.j2` outputs (one-line change per emission site in
`emit_service`).

**Manifest**: `ciac build` writes `.ciac/manifest.json` into the output
root (BTreeMap-serialized, deterministic):

```json
{
  "compiler_version": "0.6.0",
  "source_hash": "sha256 of the .ciac source",
  "target": "python",
  "files": { "app/api/upload.py": { "role": "owned", "hash": "sha256…" }, … }
}
```

Hashes are of the content *as generated* (the merge base). The manifest
itself is `Owned`.

### Regeneration algorithm (replaces the `--force` wall)

For each file the new build produces, compare three states — base
(manifest hash), disk, and new:

| Case | Action |
|------|--------|
| not on disk | write (covers new declarations and deleted-by-user recovery) |
| `Owned`, disk == base | rewrite with new content |
| `Owned`, disk == new | no-op (already current) |
| `Owned`, disk != base | **conflict**: leave the file, write `<file>.ciac-new`, report `CIAC0033` (error) naming both paths |
| `Seeded`, exists | never rewrite; if the *stub the compiler would generate* changed (e.g. a handler gained a binding parameter), write `<file>.ciac-new` and report `CIAC0034` (warning: "handler seed changed — reconcile manually") |

Files present in the old manifest but absent from the new build (a
declaration was removed): delete if `Owned` and disk == base, else
`CIAC0035` (warning: orphaned file left in place).

No silent merging in v0.6 — conflict *detection* with explicit sidecar
files is the honest, deterministic first step; textual three-way merge
is deliberately out of scope (revisit in v0.7 with real-world corpus).

### CLI surface (`crates/ciac/src/commands.rs`)

- `ciac build … --out DIR` — empty dir or manifest present ⇒ proceed
  with the algorithm above; non-empty dir *without* a manifest ⇒ current
  refusal (pre-0.6 output: suggest `--adopt` or a clean dir).
- `--adopt` — treat every on-disk file as user-modified: write only new
  files + `.ciac-new` sidecars, then a manifest. Migration path for
  v0.5-generated projects.
- `--force` — retains today's semantics (blank-slate overwrite) but now
  prints what it clobbers.
- `ciac diff FILE --target T --out DIR` — dry-run: renders in memory,
  prints per-file status (`unchanged` / `update` / `conflict` /
  `seeded-drift` / `new` / `orphan`) and unified diffs with `--patch`.
- Exit codes: conflicts are build failures (the compiler refuses to lie
  about the state of the system).

### New error codes (append-only registry, `ciac-diagnostics`)

- `CIAC0033` regeneration conflict: compiler-owned file was modified
- `CIAC0034` seeded file drifted from its regenerated seed
- `CIAC0035` orphaned generated file (declaration removed)
- `CIAC0036` output directory has no manifest (with `--adopt` hint)

### Tests

- New `tests/tests/regen.rs`: golden scenarios in tempdirs — clean
  rebuild is a no-op (byte-stable manifest); edit owned file → CIAC0033
  + sidecar; edit seeded handler + change its bindings in source →
  CIAC0034 sidecar carries the new signature; remove an api → orphan
  handling; `--adopt` on a v0.5-shaped tree.
- Determinism suite extended: manifest bytes identical across runs.

---

## M2 — Scheduled jobs: the `scheduler` construct (syntax → sema → both backends)

### Language

```ciac
use { scheduler jobs Cron; }

job Cleanup {
    schedule: "0 3 * * *";      // required, validated cron expression
}

pipeline Cleanup: PruneExpired -> publish Audited;
```

Grammar addition (`docs/language.md`, `ciac-syntax`):
`job-decl = "job" IDENT attr-block ;` with closed attributes
`schedule` (required, 5-field cron string, validated at check time —
new `CIAC0037` invalid cron) and `catch_up: bool` (default `false`).
Allowed at top level and inside service blocks.

### IR + sema

- `NodeKind::Job`, `Component::Job { name, config: JobConfig { schedule, catch_up } }`.
- A job is a pipeline owner like a worker: payload is untyped (`None`) —
  a timer has no message. `Auth` in a job pipeline is `CIAC0008`;
  `Return` is `CIAC0009`; publishes and handlers are legal, so a job can
  feed the whole streaming topology.
- Requires the `scheduler` capability (`CIAC0005` via the existing
  `default_capability` machinery); `DependsOn` edge job → scheduler
  instance. Reachability: a job with no pipeline is `CIAC0007`.
- Scheduler capability nodes stop being useless: un-gate
  `Component::Scheduler` in both backends' `supports()` and update
  `tests/tests/gating.rs` (realtime moves to its own probe until M3;
  after M3 the gating test keeps only Kafka).

### Codegen

Model (`ciac-codegen/src/model.rs`): `JobCtx { name, snake, schedule,
catch_up, steps, handlers, db_sessions, session_with, extra_imports,
call_imports }` — reuses `steps_of`/`sessions_of`/`extras_of`
wholesale; jobs are workers minus subject/payload.

- **Python**: `app/jobs/<snake>.py` from `job.py.j2` — an asyncio loop
  computing the next fire time with `croniter` (new dep when jobs
  exist), invoking the same `emit_steps` macro body as workers, with
  retry/logging parity. `app/jobs/__main__.py` gathers all jobs;
  workers `__main__` also spawns them so `docker compose` needs no new
  container kind (jobs ride the existing `-workers` container; a
  service with only jobs still emits it).
- **Rust**: `src/jobs/<snake>.rs` using `tokio` sleep-until +
  `croner` (pure-Rust cron parsing; fallback: `cron` crate) wired into
  the existing workers binary.
- Compose: nothing new — jobs run in the workers process. The
  `scheduler` capability instance contributes no container (cron is
  in-process); its value is declarative intent + gating.

### Verification

Example `examples/scheduled-cleanup.ciac`; live probe: run the
generated Python workers process with a `* * * * *` schedule and assert
the handler fires within 61s and a publish lands on a subscribed NATS…
in CI-less environments, assert the loop computes correct next-fire
times via the generated unit test (`tests/test_jobs.py` emitted
alongside smoke tests). Negative fixtures: bad cron (`CIAC0037`), job
without scheduler capability (`CIAC0005`), `Auth` in job pipeline
(`CIAC0008`).

---

## M3 — Realtime channels: the `realtime` construct (syntax → sema → both backends)

### Language

```ciac
use { realtime live WebSocket; }   // or SSE

channel Progress on Transcoded;    // exposes stream Transcoded at /channels/progress
```

`channel-decl = "channel" IDENT "on" IDENT decl-tail ;` with optional
attribute `path` (default `/channels/<kebab>`). The stream must exist
(`CIAC0017`) and the channel requires the `realtime` capability
(`CIAC0005`). Duplicate channel paths are `CIAC0003`.

### IR + sema

`NodeKind::Channel`, `Component::Channel { name, path }`;
`AsyncMessage` edge stream → channel (a channel is a consumer, so
reachability marks streams with only a channel as consumed);
`DependsOn` channel → realtime instance. Payload typing: the channel
serializes the stream's record; untyped streams flow raw JSON.

### Codegen

- **Python** (`channel.py.j2` → `app/api/channel_<snake>.py`):
  WebSocket provider — FastAPI `@router.websocket(path)`: per-connection
  NATS subscription on the stream subject, forwarding validated
  payloads (`Video.model_validate_json(...).model_dump(mode="json")`)
  to the socket; SSE provider — `StreamingResponse` with
  `text/event-stream`. Lazy NATS connect keeps import-safety and smoke
  tests green with no broker.
- **Rust** (`channel.rs.j2` → `src/routes/channel_<snake>.rs`): axum
  `ws` upgrade (or SSE via `axum::response::sse`), async-nats
  subscriber task per connection, typed decode → JSON text frames.
- Smoke tests: assert the channel path is present in the OpenAPI/router
  table and the module imports; live WS echo is covered by the M5 probe.

Un-gate `Component::Realtime`; the gating test now asserts *only*
Kafka gates and asserts jobs/channels are supported.

---

## M4 — `ciac verify`: the guarantee becomes a command (`ciac` CLI)

`ciac verify FILE --target T --out DIR [--live]`

1. Regenerates in memory and runs the M1 diff — reports drift
   (conflicts/seeded-drift/orphans) without writing.
2. Runs the generated project's own validation: `uv sync + ruff +
   pytest` (Python) / `cargo check` warnings-as-errors (Rust), per
   nested project in multi-service systems.
3. `--live`: boots the app(s) (`uvicorn`/`cargo run`) and probes
   `/health` per service, honoring per-service ports.

This is the compiler-guarantee (goal 4) as an operational loop: after
any human edit, one command answers "is this still the system the
`.ciac` file promises?" CI adds a `ciac verify` job over every example.

---

## M5 — Hardening, docs, release

- Snapshot refresh (deliberate review; new jobs/channels files, no
  byte changes to untouched examples).
- New examples: `scheduled-cleanup.ciac`, `realtime-progress.ciac`, and
  extend `multi-service-media.ciac` with a `channel Progress on
  Transcoded;` in a new fifth service to exercise channels in the
  system compose.
- Live milestone proof (as in v0.5.1): boot the realtime example,
  connect a WS client, publish to the stream via generated `publish`,
  assert the frame arrives typed; run a 1-minute job live.
- Docs: language.md gains `job`/`channel` sections + updated status
  table (only Kafka remains check-only); new `docs/regeneration.md`
  (manifest format, roles, conflict workflow, `--adopt` migration);
  README regeneration quickstart.
- Version 0.6.0 across the workspace (all internal dep versions
  together, as in 0.5.1). Milestone-per-commit discipline, full
  verification matrix (now 10 examples × 2 backends × regen no-op).

## Risks / cuts

- Three-way *merge* is explicitly deferred; v0.6 ships conflict
  detection + sidecars only. Cut line if schedule slips: M4 `--live`
  flag (steps 1–2 are the core).
- Cron parsing must be identical across hosts — pin croniter/croner
  behavior with a shared test vector table in `tests/`.
- WS fan-out per connection is deliberately naive (one NATS
  subscription per socket); pooling is a v0.8 performance item, not a
  correctness item.
