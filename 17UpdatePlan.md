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

That cliff has two audiences. For many coding agents Docker is absent,
not merely slow—the v0.15 tracing and Keycloak work could be compiled in
the sandbox but needed CI delegation for the first real capability proof.
For developers who do have Docker, image startup, compose `--wait`, and
bounded health probing turn an edit into roughly 30–90+ seconds rather
than an inner loop; current command budgets allow compose wait up to 180
seconds and health probing up to 60 seconds in
`crates/ciac/src/commands.rs`. A `0 3 * * *` job, third retry, or
`cache_ttl: 300` makes the
problem worse: nobody writes the wall-clock test because waiting is the
test.

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

Frameworks can fake one service's database; CIaC owns the whole system
graph and can fake the topology, broker, service-call boundary, and clock
together. The ambition is FoundationDB/Antithesis-style deterministic
simulation as a compiler artifact, with a bounded semantic failure model
rather than host-level chaos.

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

The compact distinction is: simulation proves generated logic and
topology under the CIaC contract; system verification proves real drivers
and wire protocols.

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

## Rollout strategy: a checkpoint before the full build

The milestone list below intentionally departs from a flat "build
everything, then prove it" sequence. Threading explicit `production()`/
`simulation()` ports through nearly every generated runtime file in two
backends is the single largest, most invasive commitment in this
document, and an earlier draft of this plan only checked whether the bet
paid off *after* both targets and every capability fake were built. That
is backwards: the architecture should be proven cheaply before it is
generalized expensively.

**Checkpoint (M5 below):** build the port abstraction for exactly one
capability (database) plus the minimum slice of the broker fake and
virtual clock needed to run the existing vertical-slice scenario
(API → transaction → call → publish → third worker attempt → virtual-time
job), on **Python only**, and hold it to this document's own stated bar:
replay-equivalent transcripts and the 1.0 s p95 / 500 ms virtual-week
budgets defined in Verification strategy. This is roughly M1–M4 plus a
narrow cut of the relational/broker fakes and a minimal runner, not the
whole version.

**Go/no-go, explicitly:** if the checkpoint holds, continue building out
the full capability-fake matrix and the complete Python runner (M6–M10).
If it does not, the fallback already described in this document's
Confidence section applies without modification: the provider-port
refactor and stateful handler tests still ship, and the project does not
relabel a collection of mocks as whole-system simulation. There is no
sunk-cost obligation to finish every downstream milestone once the
checkpoint's answer is known.

**Rust is a second, separately gated bet, not a co-requirement.**
Python's clients are already lazy (see Pillar 1), so its seam refactor is
strictly smaller than Rust's; nothing in "one simulation run uses one
backend for every service" requires both targets to ship together. v0.17
now ships `ciac sim`/`verify --sim`/`verify_sim` for Python once the
checkpoint and the full Python capability matrix are proven, and Rust
parity (originally folded into M3/M9) becomes its own later milestone
(M11 below), undertaken only once the Python-only tool surface is live.
If Rust parity is deferred past this version, that is a disclosed, not
silent, gap — the same discipline this compiler already applies to every
other target-parity gap (see docs/backends.md's status table).

**Two decisions frozen at M1, not left implicit** (recorded, with the
reasoning behind each, in "M1 findings" below):

1. *Anti-drift port design.* `production()` and `simulation()` must not
   be two adapters that can silently diverge. In Rust, the port is a
   trait both implementations satisfy — a divergent signature is a
   compile error, not a review finding. In Python, both are generated
   from one shared interface stub, checked by a golden parity test. This
   converts "two adapters can drift" (see Risks) from a review problem
   into a compile/lint-time one.
2. *What backs the relational fake (Pillar 4).* The default in this
   document is a small hand-written reference engine — portable by
   construction, with no real SQL dialect leaking into what is supposed
   to be a provider-neutral fake. The alternative is backing it with
   SQLite `:memory:`, which gets constraints/cascades/indexes for free
   at the cost of quietly reintroducing one specific dialect's behavior
   into the fake. This is a deliberate trade-off, not an implementation
   detail; M1 records the decision and the reasoning, rather than
   defaulting into the larger hand-rolled build without having weighed
   the cheaper option.

## M1 findings: reconciling the plan against actual v0.16 IR and generated code

M1's job is to freeze semantics against reality, not against this
document's own assumptions. These findings come from reading
`ciac-ir`/`ciac-sema`'s actual types and empirically confirmed by
building a real program and inspecting its generated output — not
inferred from this plan's prose.

**Confirmed alignments (zero-cost seams).** The single-effect entry
points Pillar 2 calls for already exist verbatim in generated code:
`handle_message_once(payload)` in `worker.py.j2` and
`handle_tick_once()` in `job.py.j2`. The simulator registers these as
actors directly; no new function needs inventing for this seam.

**Real retry semantics differ from a naive reading of Pillar 6.**
Generated `handle_message` does not model broker redelivery at all today
— it is a synchronous `for attempt in range(MAX_RETRIES + 1)` loop
inside one message-handling call, with no acknowledgement or scheduling
gap between attempts (confirmed by inspecting a generated worker
directly). Pillar 6's "retries are later events at the same virtual
instant unless a scenario injects delay" already happens to describe
this correctly; this finding makes it a **frozen, empirically-grounded
fact** rather than an assumption: the simulator must reproduce
zero-elapsed-time synchronous retry, not invent a broker-redelivery
model the real generated code doesn't have.

**`catch_up` is confirmed dead code today, not merely under-tested.**
Building a job with `catch_up: true` and inspecting the output shows
`CATCH_UP = True` defined and never referenced again anywhere in the
generated tree — `_sleep_until_next` always computes only the single
next fire time via `croniter(...).get_next()`. Pillar 6's catch_up
semantics (oldest-first vs. coalesce-latest) is therefore **new behavior
CIaC is defining for the first time**, not behavior the simulator merely
has to preserve — the real scheduler and the simulator must be built
together against one frozen rule, exactly as this document's Pillar 6
already says, now confirmed as a real (not hypothetical) gap.

**`index: true` on a scalar field is silently discarded, not merely
inert.** Correction to an earlier pass over this finding: `build.rs`'s
`record()` function never reads a plain (non-`Reference<T>`) field's
`attrs` at all — not even to reject an unknown one — so
`name: String { index: true; }` compiles clean with a plain `TEXT`
column, and so would any other garbage attribute name on a scalar
field. Only a `Reference<T>` field's `index` attribute gets the
forward-compatible recognize-but-no-op treatment `build.rs` comments
about. `CIAC0059`/`CIAC0060` (`InvalidStorageConstraint`/
`InvalidFieldValidation`) are allocated error codes for a
`unique`/`index`/`non_empty`/`min`/`max`/`format` scalar-attribute
profile the AST's own doc comment describes as sharing the field-attr
grammar slot — but neither code is ever raised anywhere in `ciac-sema`
today; they are reserved, not wired up. **Frozen scope:** Pillar 4's "a
declared field index covers the leading equality/range field" is
narrowed to what v0.16 actually has today — primary-key lookup and a
to-one reference's unique-FK index only. A general scalar secondary
index does not exist in the language yet, so the fake cannot claim to
model one.

**No generic field-level validation attribute exists.** Only
`Reference<T>`'s own cardinality/action/uniqueness rules and record-
construction type/completeness checking are real today; a
`non_empty`/`min_length`/`email`-style validation profile was reserved
(`CIAC0059`/`CIAC0060`, see above) but never built (confirmed against
v0.16 M7's own retrospective and against `ciac-sema` directly). **Frozen
scope:** Pillar 4's fake enforces required-field completeness, type
correctness, and the relation/cascade/uniqueness model — not business-
rule validation, because the language doesn't have it.

**`transaction { .. }` body shape is confirmed narrow.** Sema already
rejects `return`, nested `transaction`, `publish`, and every
non-database capability verb inside a transaction block — a
transaction's body HIR contains only `Let`/`Expr`/`Fail` and, inside
`if`/`match`, more of the same. There is no `publish`-inside-transaction
until v0.19; v0.17's transaction crash matrix (Pillar 1's "Atomic
meaning" table, adapted for v0.16 rather than v0.19's outbox) is
correctly scoped to write-effect-only transactions.

**The two frozen decisions from Rollout strategy, now recorded as
final:**

1. *Anti-drift port design* — adopted as written in Rollout strategy: a
   Rust trait both `production()`/`simulation()` satisfy; a Python
   shared interface stub with a golden parity test.
2. *Relational-fake backing store* — **decided: the hand-rolled minimal
   reference engine**, not SQLite `:memory:`. The findings above show
   v0.16's actual relational surface is smaller than the "can become a
   database project" risk feared: no generic secondary indexes, no
   generic field validation, a transaction body limited to database
   writes only. The engine only has to model primary keys, one to-one/
   to-many reference shape with restrict/cascade, and serial transaction
   commit/rollback — a tractable, closed surface that also preserves the
   provider-neutral fidelity property (no real dialect behavior leaking
   into a fake meant to test the CIaC contract, not Postgres/SQLite
   specifically).

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

Per Rollout strategy above, this boundary is not two independently
maintained adapters: Rust expresses it as one trait both `production()`
and `simulation()` implement, so a divergent signature fails to compile
rather than silently drifting; Python generates both from one shared
interface stub with a golden parity test.

The fake implementations and runner are generated test artifacts, not a
production runtime mode. Python simulation modules are excluded from the
production image/package; Rust puts them behind a `sim` cargo feature and
`#[cfg(feature = "sim")]`, which the release binary does not enable. The
shared ports remain because production and tests need one honest seam.

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

This retires the v0.14-era JWT-only/no-queue testing gate: a fake JWKS
provider gives OAuth2 routes the same no-infrastructure scope proof while
the real JWKS path remains in `verify --system`.

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

Two layers keep an escape from quietly producing false confidence
instead of a loud gap: a runtime backstop (`SIM0009`, "effect escaped
generated seam" — see Pillar 7/Diagnostics) fails the case the moment a
real provider call slips through during a run, and a static advisory
lint (reusing the same seeded-file-scanning approach `ciac rename --out`
already uses) flags a seeded/`extern` handler importing a raw provider
SDK directly at `ciac check` time, before any scenario happens to
exercise that path.

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

The target adapters reuse proven framework seams: Python routes execute
through `httpx.ASGITransport` against the uniquely loaded app, while Rust
uses `tower::ServiceExt::oneshot` (the existing generated scope-test
pattern). Generated Python `app/clients/*.py` and Rust reqwest call
clients receive in-process transport adapters rather than bypassing their
request/response contracts.

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
- read-your-writes inside a transaction;
- the same normalized API error semantics as real adapters (for example,
  unique/reference/restrict constraint failures map to the v0.16
  409/422 shapes rather than fake-only exceptions).

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

Python materializes eligible deliveries as scheduler-owned awaited tasks;
Rust uses a `tokio::sync` channel fabric under the same scheduler. Those
are adapter details only—the `SimPlan` ordering and acknowledgement rules,
not asyncio or Tokio wake order, define observable behavior.

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

Rust may implement the adapter with Tokio's paused clock and explicit
`advance`; Python routes every generated `croniter` job decision, worker
retry, TTL check, and `Timestamp.now()` through the generated clock port.
The harness does not monkeypatch the event loop, global `datetime`, or
use `freezegun`.

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

The fixture corpus includes direct assertions:

- advance 24 hours and observe the 03:00 job once;
- fail a worker twice and observe the third attempt without sleeping;
- advance 301 seconds and observe a `cache_ttl: 300` entry miss.

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

The human and JSON summaries include the first semantic event ordinal
(`seed 42, step 17`) so an agent can report a complete reproducible bug
without attaching target stack traces.

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

Python writes `tests/logic/test_<handler>.py`; Rust writes
`tests/logic/<handler>_test.rs`. The scaffold reuses the typed
`sample_json` machinery already used by generated system tests, contains
one passing smoke assertion plus commented behavioral examples, and is
explicitly `FileRole::Seeded`. It follows `.ciac-new` reconciliation when
signatures change.

In addition to portable whole-system scenarios, every declared job,
retried worker, and TTL-bearing CRUD resource receives one small
compiler-owned fake-backed unit proof. These locate a broken construct
quickly; they do not replace the scenario that proves the cross-service
path.

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

Artifact execution is unambiguous:

- plain `verify` compiles/type-checks simulation scaffolds but runs only
  the existing production-target test profile;
- `verify --sim` enables the Rust `sim` feature/Python simulation test
  environment, runs compiler-owned per-construct proofs and user-seeded
  `tests/logic` scaffolds, then runs portable whole-system scenarios;
- `ciac sim` runs selected/synthesized scenarios directly for speed and
  does not first run the full generated test suite.

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

The claim boundary ("simulation proves generated logic and topology
under the CIaC contract; it does not prove SQL dialect, broker
durability, cryptography, or network correctness") is not only prose in
`docs/simulation.md`: it is stated inline in the MCP tool's own
`description` field returned by `tools/list`, so an agent sees the limit
before ever calling `verify_sim`, the same way `check`/`build`/`diff`
already describe their own scope in-line rather than deferring to
external docs.

Generated `--deploy ci` workflows place a `sim` job after static tests
and before `compose-smoke`; a fast deterministic failure prevents the
expensive stack from starting. Generated `AGENTS.md` uses blunt wording:
`sim` is the fast logic/topology loop, while `verify --system` is the
real-provider merge bar.

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

`docs/simulation.md` is the public contract, not a tutorial only. It
contains per-capability fake/real tables, deterministic scheduling and
replay guarantees, the fidelity-ratchet rule, limits, and an explicit
“what simulation does not prove” section cross-linked to system
verification.

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

This becomes a permanent **fidelity ratchet**: wherever practical, the
same generated assertion vector runs once against fakes and once against
compose in `generated-system`. A disagreement is a fake/adapter bug and
blocks merge; a fake without a corresponding parity vector is not
considered complete.

The ratchet is an enforced release gate, not a described practice: a
capability fake cannot appear in the documented capability-support
matrix (Pillar 8's table) until its parity vector passes — the same
graduation discipline v0.19's architecture lints later apply to their
labeled corpus. Results are published as a checked-in, CI-regenerated
parity report (pass/fail per capability) rather than asserted only in
prose, so the claim is auditable by anyone reading the repository, not
just trusted from a milestone's own commit message.

The shared vector is the normalized `expect` portion of a portable
scenario, rendered once for the in-process runner and once for the
compose/system harness. Per-construct unit tests remain fast diagnostic
helpers and are not themselves required to run against compose.

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

Per the Rollout strategy above, M1–M5 form the checkpoint: a narrow,
Python-only vertical slice that must hold before the rest of the version
is undertaken. M6–M10 are the full Python build, gated on the checkpoint
passing. M11 (Rust parity) is a second, separately gated bet undertaken
only once the Python-only tool surface has shipped. M12 closes the
version out.

1. **M1 — Semantic freeze and confidence gate:** reconcile actual v0.16
   IR; freeze scheduling, retry, clock, failure, scenario, and replay
   semantics; record the two frozen decisions from Rollout strategy
   (anti-drift port design; relational-fake backing store).
2. **M2 — `ciac-sim` contracts:** plan/scenario/replay/transcript schemas,
   hashes, validation, codes, deterministic tests.
3. **M3 — Python production dependency seams:** Python app/state
   factories; production generation remains green. (Rust ports/adapters
   move to M11.) **Shipped, disclosed scope:** a new `app/state.py`
   (`AppState.production(settings)`/`AppState.simulation(world)`, the
   latter `NotImplementedError` until M4 has a fake to construct) is the
   seam every provider module (`db`/`cache`/`queue`/`auth`/
   `object_store`/`email`/`search`/`http_clients`) now reads through via
   a `contextvars`-backed `current()`, installed once by a new
   `create_app(state)` factory (`app/main.py`) and by `app/workers.py`'s
   entrypoint — module-global provider caches are gone, replaced by
   `AppState`'s own fields, achieving "cease to be the primary
   behavioral seam" without threading an explicit `state` parameter
   through every route/worker/job signature (those still call the same
   `get_engine()`/`get_cache()`/... accessors as before; only the
   accessors' own storage moved). Verified against every checked-in
   example (single- and multi-service): generated Python still parses,
   `ruff check` is clean, and `order-system`'s full pytest suite (17
   tests spanning db/cache/auth/CRUD) passes unchanged; the worker
   entrypoint installs state correctly and proceeds to a real connection
   attempt (confirmed live). **Deferred to M9** (disclosed, not silent):
   Python multi-service package isolation (relative imports, stable
   per-service module identity) — not needed until a runner actually
   loads more than one service's package in one process, which is M9's
   job, not M3's.
4. **M4 — Scheduler and virtual time:** stable actors, quiescence,
   retries, operational `catch_up`, deterministic IDs, failure engine.
5. **M5 — Checkpoint: minimal Python vertical slice, go/no-go.** The
   narrowest cut of the relational fake (insert/query/transaction commit-
   rollback only — not yet the full constraint/index/cascade matrix) and
   the broker fake (single publish + one worker retry — not yet ordering/
   groups/duplicate delivery), wired through a minimal Python runner
   sufficient to execute one scenario end to end. Run the checked-in
   `sim/vertical-slice.ciac-sim.json` and `sim/virtual-week.ciac-sim.json`
   against this slice only, on Python only, and hold them to this
   document's stated replay-equivalence and 1.0 s p95 / 500 ms budgets.
   **Decision point:** pass → continue to M6; fail → ship the M1–M4
   seam/scheduler work and stateful handler-test groundwork without the
   `ciac sim`/simulation label, and stop here for this version.
6. **M6 — Full relational fake:** complete v0.16 schema, constraints,
   indexes, transactions, validation, fake-versus-real probes; the v0.16
   domain-orders suite passes infrastructure-free.
7. **M7 — Full broker and temporal fakes:** ordering, groups, logical
   lanes, duplicate/lost-ack delivery, cache TTL, channels.
8. **M8 — Remaining fakes:** external HTTP, auth/users, object, email,
   search, log/metrics/tracing observations; `dev-identity` scope
   behavior passes with fake JWKS and no Keycloak process.
9. **M9 — Python runner and handler scaffolds, complete:** all services
   in one asyncio process, actual seeded code, generated cases, replay,
   full fidelity-ratchet parity report.
10. **M10 — CLI/JSON/MCP (Python):** `ciac sim`, `verify --sim`,
    `verify_sim` (with the inline claim-boundary description), bounded
    child protocol and generated guidance.
11. **M11 — Rust ports/adapters and runner parity (second gated bet):**
    Rust ports/adapters, lazy broker/JWKS, current-thread Tokio runner,
    fake-backed tests, normalized transcript equivalence with Python.
    Undertaken only after M10 ships; if not pursued this version, the
    gap is disclosed in docs/backends.md and docs/simulation.md, not
    silently dropped.
12. **M12 — examples, CI, docs, performance, v0.17.0:** all-example sim
    jobs, no-Docker guard, outer-truth reconciliation, whole-version
    analysis covering whichever of M1–M11 actually shipped.

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
- No interactive simulation REPL in the release-critical path.
- No simulation of generated k8s/Terraform/other deploy artifacts.
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
  normalized transcripts, and fake-versus-real vectors — sharpened by the
  Rollout strategy's anti-drift port design: a Rust trait both
  `production()`/`simulation()` must satisfy (compile-time enforcement,
  not review), and a golden parity test over Python's shared interface
  stub.
- **Mock rot can create a faster lie.** Mitigation: the permanent fidelity
  ratchet runs shared assertions against fakes and real providers; parity
  coverage is required for every fake, enforced as a release gate — no
  capability enters the documented support matrix without a passing
  parity vector, and results are a checked-in, CI-regenerated report.
- **The relational fake can become a database project.** Mitigation:
  implement only normalized CIaC semantics—no SQL parser or optimizer.
- **User code can escape determinism.** Mitigation: explicit ports,
  strict external fixtures, wall timeout, and honest unsupported-effect
  reporting.
- **Rust cold builds can erase the speed win.** Mitigation: fake-only
  feature separation, shared targets, and separate warm/cold reporting.
- **Retry/cron scenarios can fail to terminate.** Mitigation:
  quiescence plus hard step/time/catch-up limits.
- **Python clock seams are easy to bypass accidentally.** Mitigation:
  every generated clock call site is centralized and golden-tested; host
  clock use in compiler-owned simulation paths is a failing test.
- **Two test stacks can confuse users.** Mitigation: command/help/AGENTS
  consistently say “sim = fast logic/topology; system = real wiring.”
- **The fake matrix grows with the capability registry.** Mitigation:
  one closed operation contract and one parity vector per capability;
  future capability additions include fake maintenance in their cost.
- **MCP executes user code.** Mitigation: separately named tool,
  documented policy, no network fallback, and strict server caps.

## Confidence and v0.18 handoff

Per-capability fake seams are structural: the current target asymmetry
and skipped no-infrastructure paths are direct implementation facts.
Whole-system one-process simulation with virtual time is a
high-conviction bet, not arithmetic.

Per Rollout strategy and M5, the version now earns its continued
investment in two separately gated steps rather than one all-or-nothing
proof at the end. First: the checked-in `sim/vertical-slice.ciac-sim.json`
must execute an API → transaction → call → publish → third worker attempt
plus a virtual-time job **on Python only**, against the minimal M5 slice,
produce equivalent replay transcripts across repeated runs, and meet the
canonical prebuilt 100-effect 1.0 s p95 benchmark; its companion
`sim/virtual-week.ciac-sim.json` must meet the 500 ms cap. If either gate
fails, the provider seams and stateful handler-test groundwork from
M1–M4 still ship; the project does not relabel a collection of mocks as
whole-system simulation, and the version stops there. Second, only after
the full Python build (M6–M10) has shipped: Rust parity (M11) is its own
bet, undertaken with the same vertical-slice and virtual-week gates
re-run on Rust and checked for transcript equivalence with Python, not
assumed from the Python result.

`traced-checkout.ciac` is the smaller early honesty check for the
call→publish→worker boundary: if the generated call clients cannot run
through the in-process transport without bypassing serialization, the
whole-system claim is not ready. The fallback is explicitly per-service
fake-backed tests, not a weaker feature marketed under the same name.

Once it succeeds, v0.18's confirmed semantic-diff pillar can focus on the
next recurring cost: reviewing, gating, and mechanically applying change
to a system that now already works. The arc is intentional: express the
domain in v0.16, verify it instantly in v0.17, then change it safely in
v0.18.
