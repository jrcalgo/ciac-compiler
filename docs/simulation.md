# Simulation (v0.17)

`ciac sim` runs a portable, versioned scenario against a generated
project's **real code** — real routes, real handlers, real worker/job
entry points — with in-memory fakes standing in for the database,
broker, cache, object store, email, search, and external HTTP. No
Docker, no real network, no wall-clock sleep: a virtual clock and a
deterministic scheduler drive time and ordering instead.

## Claim boundary

Read this before trusting a green run:

```text
verify        = generated-project static truth, no Docker
verify --sim  = verify, plus bounded in-process behavioral truth, no Docker/network
verify --system = real provider/wire truth, CLI/CI only
```

Simulation proves that **the exercised generated logic and topology
behave as scripted, against these fakes.** It does **not** prove SQL
dialect fidelity, broker delivery durability, cryptography, or real
network/TLS behavior. `verify --system` against real provider
containers remains the outer truth for those — simulation is the fast
inner loop that runs before it, not a replacement for it.

## Status: Python (full), Rust and TypeScript (narrow) (v0.17 M11, TypeScript v0.23 M9)

| Surface | Python | Rust | TypeScript |
| --- | --- | --- | --- |
| `ciac sim` | done, every capability faked | done, only `db.insert` + broker publish/consume + cron jobs faked — refused with the specific reason for anything else | same narrow slice as Rust |
| `verify --sim` | done | same | same |
| MCP `verify_sim` | done | same | same |

Rust's ports/adapters seam, fake adapters, and a generated per-program
simulation runner (v0.17 M11) exist now, but they cover a deliberately
narrow slice: `crates/ciac-sim/src/world.rs`'s `SimWorld`
(`FakeDatabase`/`FakeQueue`, wired to `ciac-sim`'s own real
`FailureEngine`) fakes exactly what `sim-vertical-slice.ciac` needs —
`db.insert` and broker publish/consume, `error` failure actions only,
no independent per-`(subject, group)` broker cursors. `ciac sim
--target rust` runs a scenario against `src/bin/sim_runner.rs` (present
in the generated project whenever `db`/`queue` is declared) and refuses
cleanly — naming the specific unsupported verb(s) or capability, not a
generic "unsupported" — for any program using `db.get`/`update`/
`delete`/`query`/`count`/`delete_where`, cache, object store, email,
search, external HTTP, or `auth`. See [backends.md](backends.md) for
the lazy-init work (broker client, OAuth2 JWKS) that made constructing
`AppState` infrastructure-free in the first place, a precondition for
`AppState::simulation` existing at all.

TypeScript's own gated bet (v0.23 M9) reaches the exact same scope,
via a hand-written restatement instead of vendored Rust source: `src/
world.ts`'s `SimWorld` class (`FakeDatabase`/`FakeQueue`/`FailureEngine`,
occupying the same position Python's own `sim/pyrunner/world.py`
restatement does, since TypeScript can no more `include_str!` Rust
source than Python can) fakes the identical `db.insert` + broker
publish/consume pair, gated on the identical `db`/`queue` declaration
check, refused with the identical per-verb/per-capability reasons
`unsupportedSimCapabilities` computes over the same shared HIR scanner
Rust's own `unsupported_sim_capabilities` uses. One real, disclosed
target-specific wrinkle: TypeScript's `transaction {}` blocks are
*really* atomic in production (unlike Rust's own disclosed non-atomic
gap), but degrade to the same non-atomic, unwrapped-statement behavior
Rust already has *only* under simulation — there is no live database
for a real `BEGIN`/`COMMIT` to run against a `SimWorld`, and every
db-verb inside a transaction this checkpoint's own gate allows is
`db.insert`, already world-guarded per statement.

Single-service projects only, every target: `ciac sim` refuses cleanly
(not a crash, not a silent partial run) when it finds more than one
project descriptor (`pyproject.toml`/`Cargo.toml`/`package.json`) under
`--out`. Multi-service simulation — one driver process per service,
coordinated through one shared virtual clock — is real future work, not
attempted here for any target. `--record`/`--replay` remain
Python-only: neither generated-runner target (Rust's, TypeScript's) has
plan/replay-tape support (a plain scenario interpreter, not the bounded
child protocol below).

## The bounded child protocol

`ciac sim` embeds its own Python runner (`sim/pyrunner/*.py`, baked
into the `ciac` binary at compile time via `include_str!`) and writes
it to a scratch directory outside the generated project on every
invocation. For each `--scenario`, it invokes the runner once:

```text
uv run python auto_driver.py plan.json scenario.json \
    --source-hash <hash> --plan-hash <hash> \
    [--record out.json | --replay in.json]
```

One process, one scenario, one JSON reply on stdout, then exit — not a
persistent session or a streaming step-by-step protocol. The runner
auto-discovers workers, jobs, and APIs from the generated project's own
naming convention (`app.workers.<snake_name>`, `app.api.<snake_name>`)
and the scenario's own declared `request.api` names — no per-fixture
registration code to hand-write. An API route is auto-wired only in
the common case (no extra parameters, or a single `session`
parameter); anything else is refused with a clear error naming the
route and its extra parameters, not skipped or guessed.

### Rust's protocol is different, not the same runner ported

Rust can't embed a scratch-directory runner the way Python does — the
simulation needs concrete types from the program being simulated, so
the runner is *generated code*, not something written out at CLI-
invocation time. `ciac build`/`verify --target rust` emits
`src/bin/sim_runner.rs` directly into the generated project whenever it
declares `db` or `queue` (i.e., whenever `SimWorld` has anything to
fake). `ciac sim --target rust` builds it once (`cargo build --bin
sim_runner`) and then runs it once per `--scenario`:

```text
cargo run --bin sim_runner -- scenario.json
```

Narrower than Python's protocol in two ways worth naming: no
`plan.json`/`--source-hash`/`--plan-hash` arguments (the runner doesn't
resolve names against a `SimPlan` the way Python's auto-discovery
does — it's generated with the program's own api/worker/job names
already baked into `match`/`if` arms), and no `--record`/`--replay`.
Both are real, disclosed gaps, not silently narrower behavior.

### TypeScript's protocol mirrors Rust's own, line-for-line

Same reasoning as Rust's, same shape: `ciac build`/`verify --target
typescript` emits `src/sim_runner.ts` whenever the program declares
`db` or `queue`, dispatching on the closed `request`/`publish`/
`advance`/`drain`/`expect` step vocabulary exactly like `sim_runner.rs`
does — a generic scenario interpreter generated per program (unlike
Python's hand-written per-scenario drivers), because TypeScript also
needs concrete per-program route/worker/job names baked in at codegen
time, not resolved dynamically. `ciac sim --target typescript` installs
dependencies and compiles it once (`npm ci && npm run build`), then
runs the compiled entry point once per `--scenario`:

```text
node dist/sim_runner.js scenario.json
```

One deliberate implementation difference from Rust's runner, not a
scope difference: `app.inject()` (Fastify's real request-handling path
with no live listener, the same "real code, no Docker" property
`tower::ServiceExt::oneshot` gives Rust) is built with `{ logger:
false }` specifically for the simulation runner — pino's structured
request logs would otherwise share stdout with the runner's own
one-line `SimScenarioOutcome` JSON reply, and pino's writer flushes
asynchronously, so a stray log line could land *after* that final line
and break the "last line on stdout" contract `ciac sim`'s parent
process depends on. Production `buildApp` calls keep full logging;
only the simulation runner passes the override.

## CLI

```sh
ciac sim service.ciac -t python -o build/ --scenario sim/checkout.ciac-sim.json

# Layer simulation on top of the full static verify:
ciac verify service.ciac -t python -o build/ --sim --scenario sim/checkout.ciac-sim.json

# Record a transcript, then check a later run reproduces it:
ciac sim service.ciac -t python -o build/ --scenario sim/checkout.ciac-sim.json --record build/replay.json
ciac sim service.ciac -t python -o build/ --scenario sim/checkout.ciac-sim.json --replay build/replay.json

# Rust: same shape, no --record/--replay; refused per-program if the
# capability-coverage check finds something SimWorld doesn't fake.
ciac sim service.ciac -t rust -o build/ --scenario sim/checkout.ciac-sim.json

# TypeScript: identical shape and identical refusal behavior to Rust's.
ciac sim service.ciac -t typescript -o build/ --scenario sim/checkout.ciac-sim.json
```

`--record`/`--replay` accept exactly one `--scenario` at a time.
Replay compares `source_hash`/`plan_hash` before replaying (a mismatch
is refused, never silently accepted) and then compares the new run's
effect transcript to the recorded one. `Uuid.new()` in generated
handler bodies lowers to real, non-seeded entropy (`uuid.uuid4()`), so
replay equivalence holds over the effect/subject sequence, not
row-level ID values.

`--json` emits one envelope on stdout (the same shape `check`/`build`/
`verify --json` use, with a `sim` field: `plan_hash`, `source_hash`,
and one outcome per scenario).

## Scenarios

A scenario is a versioned JSON document (`ciac_sim::Scenario`), not
target code and not a general-purpose scripting language — the closed
action set is `request` / `publish` / `advance` / `drain` / `expect`.
`given.failures` lets a scenario declare its own failure-injection
rules up front (the same `{"at": {...}, "action": {...}}` shape
Pillar 7's failure engine uses) so a checked-in scenario is fully
self-describing — a runner reads what it needs from the document
itself, never from out-of-band per-fixture Python glue.

## MCP `verify_sim`

`verify_sim` calls the same internal function `verify --sim` does —
it does not shell out to the CLI. Unlike a human at a terminal, an
agent that hangs mid-call has no operator to interrupt it, so the tool
applies fixed server-side bounds a terminal invocation doesn't: at
most 5 scenarios per call, and the whole call is killed once it
exceeds a wall-clock limit. It cannot request `--live`/`--system`/
`--keep` (those boot Docker), and cannot write an arbitrary
`--record`/`--replay` path. The claim boundary above is stated inline
in the tool's own `description` field returned by `tools/list`, not
only here — so an agent sees the limit before ever calling it.

## Explicit non-goals (this version)

No Docker or provider containers in simulation; no cloud emulators; no
database lock/MVCC/deadlock/query-planner modeling; no Kafka
partition/rebalance/retention model; no claim of durable Core NATS
delivery; no complete OpenSearch/Redis/S3/Keycloak emulation; no
production cron persistence; no process OOM/disk/chaos or
probabilistic failure injection; no mixed-target (e.g. Python+Rust)
simulation in one run; no simulation of an external (non-built-in)
backend's output; no `ciac dev --sim`; no interactive simulation REPL.
