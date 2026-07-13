# CIaC v0.17 — Simulation: the Strongest Infrastructure-Free Loop (roadmap forecast)

> Forecast document. Assumes v0.16 has landed relations, cascades,
> uniqueness/index metadata, explicit database transactions, and declared
> field validation on both bundled targets. Direction-setting; the
> implementation planning pass freezes the simulation-plan, scenario,
> scheduler, failure, and replay schemas before target adapters begin.
>
> This is not a deployment-maturity release. It adds no Kubernetes,
> Terraform, cloud emulator, image, secret, or production-provisioning
> surface. `ciac verify --system` against real generated services and real
> provider containers remains the outer truth.

## The gap this version closes

The repository has two useful verification layers and a confidence cliff
between them.

At the inner edge, plain `ciac verify` checks regeneration drift and runs
the generated project's target-native static tests. Python emits useful
mock-based tests for inline handler lowering; Rust does not have an
equivalent provider seam. Neither executes the complete architecture.

At the outer edge, `ciac verify --system` boots compose and tests real
HTTP, database, broker, cache, tracing, and identity-provider boundaries.
That is the right integration truth, but Docker startup, image pulls,
network ports, and real sleeping make it too expensive and fragile for
every source edit or agent turn.

The missing middle is visible in the live implementation:

1. **Python mocks calls, not a system.** `AsyncMock` can prove that a
   method was invoked; it cannot preserve relational state, roll back a
   transaction, deliver a message, call another generated service, or
   advance a scheduler.
2. **Rust has concrete infrastructure in `AppState`.** SQLx pools,
   Redis, queue, OAuth/JWKS, S3, search, and HTTP clients do not expose a
   complete fakeable contract. Rust queue connection and OAuth2 JWKS
   fetching are eager enough that some no-infrastructure tests are
   skipped. Python's queue/JWKS clients are already lazy, but its
   module-global caches and FastAPI dependencies are not a complete
   whole-system fake seam.
3. **Time is real.** Worker retries happen in loops, jobs sleep against
   the host clock, `catch_up` is emitted but not operationally testable,
   cache/token expiry uses wall time, and a virtual week of scheduled
   behavior otherwise requires a real week of sleeping.
4. **Whole-system behavior implies processes.** Cross-service calls and
   broker paths are only exercised when several generated processes and
   containers are running.
5. **MCP intentionally stops at static verification.** The existing
   `verify` tool cannot request `--system` or `--live`, correctly
   preventing an agent from unexpectedly booting Docker. It therefore
   has no safe behavioral whole-system tool.
6. **User-authored logic is the largest blind spot.** Inline handlers
   have compiler-generated tests, but classic and `extern` seeded code is
   outside the generated behavioral loop unless a human writes a test.

**v0.17 theme: compile all services for one target into one deterministic
process, install faithful in-memory capability implementations, and run
the real generated and user-authored logic under a virtual clock. A
full-system week must be reproducible in milliseconds without Docker.**

## What a green simulation means

A successful simulation may claim:

- every selected service was assembled into one target-runtime process;
- the exercised generated route, pipeline, inline handler, classic
  handler, and seeded `extern` code actually ran;
- internal calls crossed an in-process framework request boundary,
  including JSON serialization, request validation, auth, route
  selection, and response-envelope decoding;
- the fake database enforced the v0.16 record, reference, cascade,
  uniqueness, index, validation, and transaction model;
- the fake broker preserved its documented per-subject/per-group order
  and deterministic at-least-once redelivery;
- retries, cron, `catch_up`, cache/token expiry, `Timestamp.now()`, and
  generated IDs used one virtual time/entropy source;
- injected failures happened at named semantic effect points;
- the same compatible replay artifact reproduces the same normalized
  transcript.

A successful simulation may not claim:

- SQL dialect, migration runner, lock, isolation, or query-planner
  correctness on Postgres/MySQL/SQLite;
- NATS/Kafka durability, rebalance, partition, retention, or SDK
  behavior;
- Redis eviction, real JWKS discovery/cryptography, Keycloak startup,
  SMTP/S3/OpenSearch/OTLP wire correctness, or network behavior;
- process-crash recovery beyond modeled effect boundaries;
- deterministic behavior for arbitrary target code that bypasses
  generated dependency ports;
- deployability.

Every report states that boundary. `verify --system` is not renamed
legacy, optional truth, or a slower simulator.

## Architecture: a compiler simulation plan, not a Python shortcut

Generating a Python-only harness would repeat the current asymmetry and
infer Rust behavior from another target. v0.17 introduces a
target-neutral plan:

```text
.ciac source
    │
    ▼
NormalizedIr
    │
    │ ciac-sim::plan
    ▼
SimPlan
    ├───────────────┬────────────────┐
    ▼               ▼                ▼
Python adapter      Rust adapter     external backend
one asyncio         one current-     explicit unsupported
process             thread Tokio     unless it implements sim
```

`SimPlan` owns semantics:

- stable project/service/route/handler/table/stream/job/capability IDs;
- normalized v0.16 schemas and transaction regions;
- pipeline and cross-service topology;
- worker retries, logical concurrency, cron, and `catch_up`;
- capability-instance bindings;
- synthesized scenario cases;
- semantic failure points;
- deterministic scheduling keys;
- transcript/replay metadata.

Target adapters own execution:

- loading/linking all generated service code;
- constructing target-native records and requests;
- installing fake capability ports;
- invoking actual routes, workers, jobs, and handlers;
- producing one normalized event transcript.

Simulation is target-specific because user code is target-specific:

```sh
ciac sim main.ciac --target python --out ./build
ciac sim main.ciac --target rust --out ./build-rust
```

There is no implicit “reference Python” target. One simulation run uses
one backend for every service, matching the current project-level target
model. Mixed-host simulation remains outside the release.

“One process” means one generated runner process containing all service
routers and actors. The short-lived `ciac` launcher is naturally a
separate parent.

## Pillar 1 — A generated runtime seam for real and fake providers

### One explicit dependency boundary

Generated code gains narrow ports for the operations CIaC itself emits:

- database begin/commit/rollback/query/mutation;
- cache get/set/delete/expiry;
- broker publish/subscribe/ack/nack;
- authentication and scope checks;
- object storage;
- email;
- search;
- external HTTP;
- internal service dispatch;
- clock, sleep, deterministic IDs, and event observation.

Two constructors are explicit:

```text
production(settings/config) -> provider adapters
simulation(sim world view)   -> in-process adapters
```

Simulation is never selected by an environment variable in the normal
server binary. A failed production connection never falls back to fake
state. A missing fake fails preflight; it never reaches a real provider
URL.

### Python shape

Python keeps `Settings` as environment parsing and introduces a generated
app/state factory:

```python
settings = Settings()
state = await AppState.production(settings)
app = create_app(state)
```

The simulator uses:

```python
world = SimWorld.from_plan(plan, seed=seed, clock=clock)
state = AppState.simulation(world.for_service("Catalog"))
app = create_app(state)
```

Routes, workers, jobs, CRUD stores, clients, and handlers receive a state
view. Module-global cached engines, producers, clients, and monkeypatched
`publish` functions cease to be the primary behavioral seam.

The production `app` symbol remains available for
`uvicorn app.main:app`, assembled through the production factory.

### Rust shape

Rust retains `Config` and `AppState`, but concrete provider fields move
behind generated ports or explicit real/sim adapter enums:

```rust
let state = AppState::production(Config::from_env()).await?;
let state = AppState::simulation(world.for_service(ServiceId::CATALOG));
```

`AppState` remains cheap to clone through `Arc` handles. The simulator
uses a current-thread Tokio runtime so task progress is controlled at
semantic effect boundaries rather than host thread races.

The current Rust-specific eager infrastructure gaps are closed while
Python's existing lazy behavior is preserved:

- Rust broker connection becomes lazy;
- Rust OAuth2 JWKS loading becomes lazy and cached;
- simulation adapters never inspect queue/JWKS URLs;
- constructing any valid Rust router requires no live infrastructure.

The existing Rust scope-test restriction for queue/OAuth2 services must
disappear.

### Python multi-service package isolation

Live multi-service Python output gives each service a distinct project
package name but hard-codes the same top-level import namespace, `app`, in
every service tree. Loading several such modules in one interpreter is
unsafe.
The generated simulator therefore:

- uses package-relative generated imports;
- loads each service package under a stable service-specific module
  identity;
- preserves normal production execution for each per-service `app`;
- emits migration guidance/sidecars for seeded code that relies on
  eager absolute `from app...` imports;
- rejects unresolved absolute-import collisions instead of changing
  `sys.modules` context between calls.

### User-authored code

Classic and `extern` handlers are imported/linked from the real seeded
files. They are not replaced by simulator stubs.

The supported deterministic contract is:

- user code receives generated capability interfaces;
- calls through them use fake or production adapters according to the
  explicit constructor;
- generated context exposes virtual clock and ID sources;
- a seeded file still identical to its generated TODO body is detected
  and reported as unimplemented when a selected scenario reaches it.

Arbitrary direct SQLAlchemy, SQLx, HTTP, filesystem, subprocess,
threading, host-clock, or random calls remain escape hatches. They are
not silently intercepted. The generated scaffold teaches users to stay
inside the ports when they want deterministic simulation.

## Pillar 2 — `SimPlan` and whole-system assembly

### Versioned plan

A canonical plan contains:

```text
SimPlan
├── plan version, source hash, project identity
├── services
│   ├── routes and CRUD operations
│   ├── handler entry points
│   ├── capability bindings
│   ├── workers and logical lanes
│   └── jobs
├── records and validation rules
├── tables
│   ├── columns, keys, references, cascades
│   ├── unique constraints and indexes
│   └── transaction metadata
├── streams, subjects, groups, channels
├── scheduling keys
├── failure-point vocabulary
└── synthesized verification cases
```

IDs derive from validated declaration order and stable semantic keys,
not hash-map order, generated paths, ports, or provider URLs.

Canonical JSON produces `plan_hash`. Replay compatibility checks plan,
source, target-adapter, and scenario versions before execution.

Provider names remain in reports, but the logical fake does not pretend
to emulate Postgres versus SQLite or NATS versus Kafka. It tests the
provider-neutral CIaC contract.

### Generated runner

Each build emits compiler-owned simulation support:

- root runner;
- `sim/plan.json`;
- one service registration adapter per service;
- fake-capability runtime modules;
- generated portable cases;
- target-native handler test scaffolds.

The runner:

1. creates one `SimWorld`;
2. creates a state view per service;
3. constructs each real generated router with simulation state;
4. registers internal call targets by service/API identity;
5. registers workers, consumers, channels, and jobs as actors;
6. loads a scenario;
7. executes actions and drains eligible work;
8. writes one normalized result through a dedicated result channel;
9. exits at quiescence.

Target/user stdout and stderr are narration only. They cannot corrupt the
versioned result consumed by CLI JSON or MCP.

### Internal calls retain an application boundary

`call Service.Api` does not directly invoke a handler function. It:

- serializes canonical JSON;
- issues an in-process ASGI/Axum request;
- runs request parsing, validation, auth/scope, routing, and response
  construction;
- serializes the response envelope;
- validates/reconstructs it on the caller side.

There is no socket, but serialization and generated framework behavior
remain observable.

### Actors, not infinite loops

Production workers/jobs remain long-running. Simulation registers
single-effect entry points:

- `handle_message_once`;
- `handle_tick_once`;
- channel delivery;
- consumer event handling.

The simulation scheduler owns when those entry points run. A run reaches
quiescence when:

- no scenario action is immediate;
- no message/retry is eligible at current virtual time;
- no requested clock advance remains;
- no generated effect is in flight.

Future recurring cron ticks do not prevent termination, and the runner
never leaps forever to the next schedule.

## Pillar 3 — Portable scenarios

Scenarios are versioned JSON, not target code and not a new
general-purpose `.ciac` sublanguage:

```json
{
  "simulation_version": 1,
  "name": "third-retry-and-nightly-cleanup",
  "start_at": "2030-01-01T00:00:00Z",
  "given": {
    "db": [
      {
        "service": "Orders",
        "table": "Orders",
        "rows": []
      }
    ],
    "external_http": [
      {
        "instance": "payments",
        "responses": [
          {"error": "timeout"},
          {"error": "timeout"},
          {"status": 200, "json": {"accepted": true}}
        ]
      }
    ]
  },
  "steps": [
    {
      "request": {
        "service": "Gateway",
        "api": "CreateOrder",
        "json": {"total": 10},
        "as": {"sub": "user-1", "scopes": ["orders:write"]},
        "save_as": "create"
      }
    },
    {"drain": {}},
    {"advance": {"by": "7d"}},
    {"drain": {}},
    {"expect": {"worker_attempts": {"worker": "Charge", "count": 3}}},
    {"expect": {"job_runs": {"job": "Cleanup", "count": 7}}}
  ]
}
```

The closed action set is:

- `request`;
- `publish`;
- `advance`;
- `drain`;
- `expect`.

Assertions cover responses, rows, cache, objects, the simulated email
mailbox, search,
external calls, messages/deliveries, jobs, channels, traces, and
quiescence.

No arbitrary scripts, loops, conditional scenario language, or
target-specific callbacks enter v0.17.

### Scenario discovery

Without explicit `--scenario`, the runner executes:

1. compiler-synthesized cases;
2. local `sim/*.ciac-sim.json`, sorted by normalized path.

Each case gets a fresh world. Synthesized cases include, where enough
typed data exists:

- one valid request per API;
- unauthorized/authorized variants for scoped routes;
- typed CRUD operations;
- triggerable service calls;
- API-originated publish→worker chains;
- realtime fan-out;
- v0.16 relation/constraint/transaction smoke paths;
- one invalid value per validation rule.

Jobs require an explicit clock advance. A program with no synthesizable
entry point reports zero generated cases and a warning; state
construction alone is not described as behavioral coverage.

### Preflight validation

Before target code runs:

- every named service/API/table/stream/capability resolves;
- seeded rows satisfy types, constraints, references, and ordering;
- principals use declared scopes;
- failure selectors name supported effect points;
- assertions apply to declared capabilities;
- payload/resource/step limits are bounded.

Scenario errors carry JSON pointer plus resolved line/column and may
offer nearest-name fixes.

## Pillar 4 — In-memory relational database

The fake database is a small reference engine built from normalized
v0.16 schema, not a dictionary with permissive behavior.

For each database instance it stores:

- ordered row maps by canonical primary key;
- primary and unique indexes;
- declared non-unique indexes;
- reverse-reference maps;
- relation/link-table state;
- normalized validation rules;
- transaction write sets.

It enforces:

- required fields and field validation;
- primary/unique conflicts;
- reference existence;
- restrict/cascade behavior;
- deterministic relation hydration;
- index maintenance after insert/update/delete/rollback;
- atomic transaction commit/rollback;
- read-your-writes inside a transaction.

The model is deterministic serial transaction execution. It does not
simulate MVCC, lock waits, deadlocks, write skew, provider-specific
null uniqueness, or query cost.

Queries use a declared index where the normalized predicate allows and
record the selected access path. Other predicates perform a deterministic
scan. This proves metadata reaches execution, not real optimizer quality.

For unordered language queries, the fake uses a seed-derived stable
order. The same replay is stable; different seeds may expose accidental
ordering assumptions. Many-valued relation hydration retains v0.16's
deterministic target-ID ordering.

## Pillar 5 — In-memory broker and at-least-once delivery

One logical broker is shared by every simulated service.

Each publish records:

- subject ID and per-subject sequence;
- deterministic message ID;
- canonical payload;
- trace/correlation metadata;
- virtual publish time.

For each `(subject, queue group)`:

- first delivery follows publish sequence;
- a failed/lost-ack message remains ahead of later messages;
- independent groups each receive a copy;
- logical worker lanes are selected deterministically;
- realtime subscribers observe fan-out without consuming worker copies.

At-least-once cases are explicit:

- handler failure and retry;
- injected disconnect;
- acknowledgement lost after effects committed;
- injected duplicate delivery.

A redelivery preserves message ID/payload and increments attempt. After
`max_retries + 1` total attempts, the transcript records exhaustion. No
dead-letter stream is invented.

The fake contract may be more adversarial than current Core NATS
durability. That is useful for finding idempotency bugs, not proof that a
real broker survives a crash.

### Database/broker ordering

The normal worker path is:

1. receive;
2. begin transaction if declared;
3. run effects;
4. commit/roll back;
5. acknowledge or schedule redelivery.

An injected lost acknowledgement after step 4 deliberately creates the
duplicate-after-commit failure v0.19 idempotency must solve.

## Pillar 6 — Virtual time, deterministic scheduling, and replay

### One clock

Virtual time drives:

- `Timestamp.now()`;
- generated retry eligibility;
- cache expiry;
- token expiry;
- cron and `catch_up`;
- scripted external latency/timeouts;
- event timestamps.

Generated UUIDs and tie-breaking entropy use a separate seeded stream.
Production adapters still use host time/entropy.

### Scheduler order

Every effect yields a semantic key containing:

```text
virtual timestamp
phase
service declaration identity
actor identity
stream/message sequence
delivery attempt
local occurrence
```

Eligible events sort by a documented total order. Worker `concurrency`
creates deterministic logical lanes; the host scheduler does not race
them. User code between generated effect calls is atomic from the
simulator's perspective.

### Retries

The live language has `max_retries` but no declared backoff. Simulation
does not invent one:

- first delivery is attempt zero;
- retries are later events at the same virtual instant unless a scenario
  injects delay;
- all attempts appear in the transcript;
- exhaustion is expected or fails the case.

### Cron and `catch_up`

The generated real schedulers and simulator must share one semantic rule:

- `catch_up: true` runs every missed due instant oldest-first;
- `catch_up: false` coalesces missed instants into one latest run;
- catch-up work is bounded;
- recurring future ticks do not auto-advance time.

This also pays the current debt where `CATCH_UP` is generated but unused.
Durable schedule history across process restarts is not part of v0.17.

### Replay

A replay artifact contains:

- replay schema, compiler, target-adapter, source, and plan versions;
- scenario canonical content;
- seed and initial clock;
- failure rules;
- scheduler choices;
- generated IDs;
- ordered external fixtures;
- normalized inputs and transcript hash.

`--replay` refuses source/plan/adapter mismatch. Compatibility is promised
within a replay schema, not indefinitely across semantic compiler
changes.

Failed CLI runs print a copyable target/scenario/seed command and write a
bounded last-failure artifact under ignored `.ciac/sim/` state. MCP
returns replay data without writing an arbitrary caller path.

## Pillar 7 — Deterministic failure injection

Failures select semantic effects, not threads, line numbers, or target
stack frames:

```json
{
  "at": {
    "effect": "broker.ack",
    "subject": "orders.created",
    "occurrence": 1,
    "phase": "after"
  },
  "action": {"kind": "lose"}
}
```

Selectors may include:

- service/actor/handler;
- capability kind and instance;
- operation;
- table/stream/subject/target route;
- message sequence/attempt;
- local/global occurrence;
- `before` or `after`.

Actions are:

- `error`;
- `delay`;
- `timeout`;
- `lose`;
- `duplicate`;
- `disconnect`.

`before` means the effect was not applied. `after` means it was applied
but the caller observed failure or ambiguity. The distinction is
essential for commit-then-error and lost-ack tests.

A required failure rule that never matches fails the scenario. Typos
must not produce false-green chaos tests.

Probabilistic failures, process OOM, disk corruption, packet simulation,
and arbitrary CPU scheduling are outside the version.

## Pillar 8 — Remaining capability fakes

The cut line is the CIaC contract, not complete provider emulation:

| Capability | Simulated behavior | Not claimed |
|------------|--------------------|-------------|
| cache | ordered values, TTL, get/set/delete | Redis persistence, eviction, clustering |
| object store | bucket/key bytes, overwrite/get/delete/list | S3 signing, IAM, multipart |
| email | inspectable simulated mailbox and failures | SMTP/SES delivery |
| search | deterministic documented query subset | scoring, analyzers, shards |
| external HTTP | strict ordered fixtures by instance/method/path | DNS, TLS, remote correctness |
| auth/users | deterministic principals, scopes, issuer/audience/expiry | cryptography and JWKS |
| realtime | subscriber registration and delivery transcript | socket/proxy framing |
| logging | structured transcript entries | collector formatting |
| metrics | in-memory counters/gauges where generated | scrape/exposition correctness |
| tracing | request/call/publish/worker correlation tree | OTel exporter/Jaeger correctness |
| scheduler | virtual cron/catch-up | durable production scheduling |

Unmatched external HTTP calls fail closed. A declared capability without a
fake is a preflight error, never a network fallback.

## Pillar 9 — Handler test scaffolding and parity

### Inline handlers

Generated inline tests move from call-count mocks to stateful outcomes:

- insert then query observes the row;
- failed transaction observes rollback;
- cache expiry follows virtual time;
- publish produces a message;
- object/email/search/HTTP effects are inspectable;
- validation and typed errors are normalized.

Rust receives the same tests from the same portable vectors.

### Classic and `extern` handlers

Each seeded handler gets a one-time seeded simulation test scaffold:

- generated valid input;
- fake capability construction;
- invocation of the real handler;
- examples of row/message/email-mailbox assertions;
- clock/ID guidance.

It is user-owned and follows `.ciac-new` reconciliation when signatures
change.

### Cross-target equivalence

`typed_handler_equivalence.rs` graduates from string/count comparison to
portable execution for compiler-owned handlers. Normalized comparison
includes:

- response JSON/status;
- rows and constraint outcomes;
- transaction commit/rollback;
- messages/order/attempts;
- capability observations;
- virtual timestamps and trace tree.

It excludes target stack traces, exception wording, SQL strings, and
module paths. Arbitrary user-authored Python/Rust bodies are compared only
in paired fixtures written to be equivalent.

## CLI, JSON, and MCP

### `ciac sim`

```sh
ciac sim main.ciac -t python -o ./build \
  --scenario sim/retry.json --seed 42
```

It:

1. runs the normal front end;
2. builds `SimPlan`;
3. regenerates through manifest/sidecar safety;
4. builds/imports the target runner;
5. runs synthesized or selected cases;
6. returns a deterministic summary/replay.

It does not first run the complete generated lint/test suite; that keeps
it the edit loop.

Useful flags:

| Flag | Meaning |
|------|---------|
| `--scenario` | repeatable scenario path |
| `--seed` | deterministic `u64`, default zero and always reported |
| `--start-at` | virtual RFC 3339 start |
| `--failures` | failure schedule |
| `--record` | atomically write complete replay |
| `--replay` | execute compatible recording |
| `--max-steps` | bounded semantic-effect limit |
| `--wall-timeout` | guard for arbitrary user code that never yields |
| `--json` | one versioned stdout document |

There is no `--allow-network`, provider emulator, Docker, or production
data mode.

### `ciac verify --sim`

```sh
ciac verify main.ciac -t rust -o ./build --sim
```

Phase order is:

```text
regeneration/static → simulation → optional system/live
```

Plain `verify` remains unchanged. `--sim --system` is legal and stops at
the first failed phase. `--keep` applies only to Docker-backed phases.

### JSON

The envelope version increments from whatever v0.16 actually shipped.
Simulation has an independently versioned result:

```json
{
  "command": "sim",
  "success": false,
  "diagnostics": [],
  "simulation": {
    "simulation_version": 1,
    "target": "rust",
    "scenario": "retry",
    "plan_hash": "sha256:...",
    "seed": "42",
    "counts": {
      "effects": 31,
      "transactions": 2,
      "messages": 1,
      "deliveries": 3
    },
    "failure": {
      "code": "SIM0001",
      "kind": "assertion_failed",
      "message": "expected 3 attempts, observed 2"
    },
    "transcript_hash": "sha256:..."
  }
}
```

Large successful event logs are referenced by hash/replay, not embedded
by default.

### MCP `verify_sim`

The current MCP `verify` remains static. A separate tool makes the
stronger execution policy visible:

```text
verify      = generated-project static truth, no Docker
verify_sim  = bounded in-process behavioral truth, no Docker/network
--system    = real provider/wire truth, CLI/CI only
```

`verify_sim` calls the same internal envelope function as
`verify --sim`; it does not shell through the human CLI. It has fixed
server-side step/wall limits, cannot request Docker/live/keep, cannot
write arbitrary replay paths, and states that user-authored target code
will execute.

## Diagnostics and limits

Invalid source/scenario/replay configuration uses append-only CIAC codes
allocated after v0.16. Runtime outcomes use a separately versioned
`SIM` registry:

| Code | Meaning |
|------|---------|
| `SIM0001` | assertion failed |
| `SIM0002` | unhandled handler/capability error |
| `SIM0003` | effect/message/row/time/wall limit |
| `SIM0004` | pending work cannot progress |
| `SIM0005` | replay mismatch/divergence |
| `SIM0006` | missing external fixture |
| `SIM0007` | required failure rule unmatched |
| `SIM0008` | reached unchanged TODO seed |
| `SIM0009` | effect escaped generated seam |

Default per-case limits are bounded and documented:

- semantic effects;
- messages/delivery attempts;
- rows and object/cache/search entries;
- payload bytes;
- catch-up ticks and virtual-time span;
- transcript bytes;
- wall time.

Hitting a limit is a failure with actor/effect/replay context, never a
truncated success. MCP limits are stricter and cannot be raised beyond
server caps.

## Implementation map

### New `crates/ciac-sim`

Owns:

- plan and canonical hash;
- scenario parser/validation;
- deterministic scheduler/clock;
- failure and replay formats;
- transcript and `SIM` codes;
- reference relational engine;
- reference broker;
- limits and conformance vectors.

It depends on normalized IR, not on Python or Rust.

### `ciac-ir` / sema

- Ensure v0.16 relations/transactions/validation are fully normalized.
- Retain source spans on simulated effects where available.
- Add reusable effect visitors rather than duplicating HIR recursion.
- Keep simulation concerns out of syntax unless source semantics are
  genuinely missing.

### Shared codegen

- Add simulation plan/case assembly.
- Extend the backend trait with explicit simulation support and runner
  specification.
- Emit root runner/support files under normal ownership.
- External backends default to unsupported; no fake promise from opaque
  typed-handler IDs.

### Python backend

- Add app/state factories and real/sim adapters.
- Make generated imports multi-service-safe.
- Refactor routes/workers/jobs/clients/stores/logic onto ports.
- Emit runner and fake-backed tests.
- Preserve production behavior and system tests.

### Rust backend

- Split concrete clients from capability ports.
- Preserve Python's lazy clients and make Rust queue/JWKS production
  initialization lazy.
- Add current-thread runner and fake adapters.
- Emit fake-backed handler/route tests.
- Feature-gate heavy real-provider dependencies where possible for the
  sim runner.

### CLI and docs

- `main.rs` / `commands.rs` / `json_out.rs`: `sim`, `verify --sim`,
  replay/result handling.
- `mcp.rs`: `verify_sim`.
- `describe`/vocab: simulator actions, limits, codes.
- New `docs/simulation.md`.
- Update architecture, dev-loop, agents, backend, and external-backend
  guides.
- Generated `AGENTS.md` explains the three truth layers.

## Verification strategy

### Infrastructure-free compiler tests

`cargo test --workspace` must never start Docker or a network service.
It covers:

- canonical plan IDs/hashes;
- same seed → byte-identical transcript;
- documented different-seed variation;
- scheduler tie-breaking and quiescence;
- replay/mismatch;
- before/after failures;
- all limits;
- v0.16 references/cascades/constraints/indexes/transactions;
- cache expiry;
- broker order/groups/fan-out/retry/duplicate/lost ack/exhaustion;
- cron and `catch_up`;
- auth scope/expiry;
- strict HTTP fixtures.

A CI sentinel makes accidental Docker use fail immediately.

### Generated target tests

For every example:

- simulation state constructs without infrastructure;
- no queue connection/JWKS fetch occurs;
- inline fake-backed tests pass;
- selected seeded fixtures execute real code;
- Python remains Ruff-clean;
- Rust compiles/tests with warnings denied.

### Cross-target vectors

Run the same cases on both targets for:

- every closed capability verb;
- relations/constraints/validation;
- transaction rollback;
- internal service calls;
- publish→worker and duplicate delivery;
- retries/exhaustion;
- cron/catch-up;
- auth;
- paired extern handlers.

### Fake-versus-real probes

A focused subset also runs through existing compose verification:

- relation/cascade/unique/rollback against supported databases;
- NATS and Kafka delivery;
- cache TTL at a coarse boundary;
- OAuth2 scopes through Keycloak;
- one external capability fixture.

The comparison uses normalized external outcomes. These remain explicit
Docker jobs.

### Performance acceptance

After dependencies are present:

- `SimPlan` construction for the flagship is at most 100 ms;
- a prebuilt/warm Python or Rust runner completes the canonical
  100-effect whole-system scenario in at most 1.0 s at p95;
- the virtual-week fixture (cron, retries, expiry, and 1,000 semantic
  effects) completes in at most 500 ms and performs no wall-clock sleep;
- the 10,000-effect scheduler/reference-engine benchmark completes in at
  most 5 s with peak resident memory at most 256 MiB;
- cold dependency installation/compilation is reported separately.

The reference is the project's standard Linux CI runner after one warm-up
run. `cargo bench -p ciac-sim --bench simulation` measures plan,
scheduler, and reference-engine cases; a checked-in
`simulation_runner_perf` integration test measures each generated target
adapter and writes machine-readable samples. Five consecutive measured
runs must satisfy the caps. Budgets are checked-in benchmarks, not hidden
marketing claims.

## Flagship

Add a simulation-focused multi-service example using:

- authenticated API and validation;
- v0.16 references/uniqueness/transaction;
- internal call;
- external HTTP fixture;
- publish→worker retries;
- object/email/search observations;
- cron `catch_up`;
- realtime progress.

Scenarios prove happy path, rollback, missing target, unique violation,
external timeout, lost acknowledgement, duplicate effects, cache expiry,
and a virtual week of scheduled work.

Existing `inventory-system`, `multi-service-media`, `typed-handlers`,
`kafka-pipeline`, and the v0.16 flagship remain smaller conformance
fixtures rather than being replaced.

## Milestones

1. **M1 — Semantic freeze and confidence gate:** reconcile actual v0.16
   IR; freeze scheduling, retry, clock, failure, scenario, and replay
   semantics.
2. **M2 — `ciac-sim` contracts:** plan/scenario/replay/transcript schemas,
   hashes, validation, codes, deterministic tests.
3. **M3 — Production dependency seams:** Python app/state factories,
   Rust ports/adapters and lazy broker/JWKS, Python package isolation;
   production generation remains green.
4. **M4 — Scheduler and virtual time:** stable actors, quiescence,
   retries, operational `catch_up`, deterministic IDs, failure engine.
5. **M5 — Relational fake:** full v0.16 schema, constraints, indexes,
   transactions, validation, fake-versus-real probes.
6. **M6 — Broker and temporal fakes:** ordering, groups, logical lanes,
   duplicate/lost-ack delivery, cache TTL, channels.
7. **M7 — Remaining fakes:** external HTTP, auth/users, object, email,
   search, log/metrics/tracing observations.
8. **M8 — Python runner and handler scaffolds:** all services in one
   asyncio process, actual seeded code, generated cases, replay.
9. **M9 — Rust runner and parity:** current-thread Tokio runner,
   fake-backed tests, normalized transcript equivalence.
10. **M10 — CLI/JSON/MCP:** `ciac sim`, `verify --sim`, `verify_sim`,
    bounded child protocol and generated guidance.
11. **M11 — examples, CI, docs, performance, v0.17.0:** all-example sim
    jobs, no-Docker guard, outer-truth reconciliation, whole-version
    analysis.

## Explicit cuts

- No deployment target or maturity work.
- No Docker/provider containers in simulation.
- No cloud emulators.
- No database lock/MVCC/deadlock/query-planner simulation.
- No Kafka partition/rebalance/retention model.
- No claim of durable Core NATS delivery.
- No complete OpenSearch/Redis/S3/Keycloak emulation.
- No production cron persistence.
- No process OOM/disk/packet chaos or probabilistic failures.
- No transparent interception/sandboxing of arbitrary user I/O.
- No mixed-target project simulation.
- No external-backend simulation without an explicit future protocol.
- No `ciac dev --sim` requirement.
- No general-purpose scenario programming language.

## Risks

- **Faithful-looking fakes can create false confidence.** Mitigation:
  explicit claim boundaries, real-provider comparison probes, and
  `verify --system` retained as outer truth.
- **The dependency-seam refactor touches most generated runtime files.**
  Mitigation: land real adapters first and keep system verification green
  before adding fakes.
- **Seeded code may depend on concrete provider types.** Mitigation:
  compatibility wrappers where honest, sidecar migration seeds, and
  explicit diagnostics rather than casts.
- **Python package isolation is subtle.** Mitigation: relative imports and
  unique package identities are a release gate; no contextual global
  alias trick.
- **Two adapters can drift.** Mitigation: one plan, one scenario corpus,
  normalized transcripts, and fake-versus-real vectors.
- **The relational fake can become a database project.** Mitigation:
  implement only normalized CIaC semantics—no SQL parser or optimizer.
- **User code can escape determinism.** Mitigation: explicit ports,
  strict external fixtures, wall timeout, and honest unsupported-effect
  reporting.
- **Rust cold builds can erase the speed win.** Mitigation: fake-only
  feature separation, shared targets, and separate warm/cold reporting.
- **Retry/cron scenarios can fail to terminate.** Mitigation:
  quiescence plus hard step/time/catch-up limits.
- **MCP executes user code.** Mitigation: separately named tool,
  documented policy, no network fallback, and strict server caps.

## Confidence and v0.18 handoff

Per-capability fake seams are structural: the current target asymmetry
and skipped no-infrastructure paths are direct implementation facts.
Whole-system one-process simulation with virtual time is a
high-conviction bet, not arithmetic.

It earns the full version only if the checked-in
`sim/vertical-slice.ciac-sim.json` executes an API → transaction → call
→ publish → third worker attempt plus a virtual-time job on both targets,
produces equivalent replay transcripts, and is the canonical prebuilt
100-effect benchmark that meets the 1.0 s p95 cap above. Its companion
`sim/virtual-week.ciac-sim.json` must meet the 500 ms cap. If either gate
fails, the provider seams and stateful handler tests still ship; the
project does not relabel a collection of mocks as whole-system
simulation.

Once it succeeds, v0.18 can focus on the next recurring cost: reviewing,
gating, and mechanically applying change to a system that now already
works.
