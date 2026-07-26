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

## Status: Python + Rust (full), TypeScript/Go/Java (narrow) (v0.17 M11, TypeScript v0.23 M9, Go v0.24 M9, Java v0.25 M9, Rust v0.25 M4)

See [backends.md](backends.md)'s Divergence ledger — Open (tracked)
table for this gap's classification and address ("Simulation depth:
only `db.insert` + publish faked", closing in `27UpdatePlan.md`) and
"Multi-service programs refused by `ciac sim`" (closing in
`28UpdatePlan.md`). The table below is this page's own per-surface
detail, not a restatement of the ledger's entry.

| Surface | Python | Rust | TypeScript | Go | Java |
| --- | --- | --- | --- | --- | --- |
| `ciac sim` | done, every capability faked | done as of `27UpdatePlan.md` M4 — every verb `SimWorld` fakes (db/cache/object store/email/search/http/auth), gate-emptiness proven across the whole example corpus | narrow: only `db.insert` + broker publish/consume + cron jobs faked — refused with the specific reason for anything else | same narrow slice as TypeScript | same narrow slice as TypeScript/Go |
| `verify --sim` | done | same | same | same | same |
| MCP `verify_sim` | done | same | same | same | same |

Rust's ports/adapters seam and generated per-program simulation runner
(v0.17 M11) started at the same narrow slice TypeScript/Go/Java still
occupy; `27UpdatePlan.md` M4 grew `crates/ciac-sim/src/world.rs`'s
`SimWorld` (already deepened by that arc's M2-M3) into every remaining
verb's own world-guard leaf in Rust's `lower.rs` — `db.get`/`update`/
`delete`/`query`/`count`/`delete_where`, cache/object store/email/
search/external HTTP, and `auth` (claims-lookup against the world,
matching Python's own `FakeAuth`, not real JWT/JWKS crypto) — plus a
schema-aware `SimWorld::with_schema` call built from the same
`ciac_codegen::migrations::snapshot_schema` the migration DDL itself
reads (so cascade/restrict/unique checks under simulation can never
drift from what production actually enforces), a real atomic
`commit_batch_checked` for `transaction {}` blocks (retiring the
disclosed non-atomic-under-simulation gap below), and a `world.broker`-
based `(subject, group)`-cursor `drain()` replacing the old shared-
queue dispatch, so two independent workers on one subject now both see
every message (true fan-out) instead of only the first-registered one.
`ciac_backend_rust::unsupported_sim_capabilities` reflects this: it
always returns empty now, proven by an in-crate test iterating the
whole example corpus. One structural note the `Full`/`Narrow` column
above doesn't capture: `ciac`'s own `SimSupport::Full` variant is
hardcoded to Python's dynamic-import driver
(`crates/ciac/src/commands.rs`), so Rust's `TargetInfo` stays
`SimSupport::Narrow` with an always-empty refusal list rather than
switching enum variants — a real behavioral difference from Python
("full" in outcome, still "Narrow" in the type) rather than a loose
end. `crud <Name>: <Record>` resources remain outside this milestone's
scope for a structural reason, not an oversight: their generated store
(`resource_store.rs.j2`) still never reads `self.world`, but a
scenario's `request` step can only address `c.apis` (typed/classic-
pipeline routes with an attached `Pipeline`), which a `crud` resource's
synthesized api node never has — confirmed by inspecting a generated
`sim_runner.rs`'s own route-dispatch match arms — so the missing guard
is real but not reachable through anything `ciac sim` exposes today.
See [backends.md](backends.md) for the lazy-init work (broker client,
OAuth2 JWKS) that made constructing `AppState` infrastructure-free in
the first place, a precondition for `AppState::simulation` existing at
all.

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
*really* atomic in production (matching Rust's own production code
since `26UpdatePlan.md` M1), but degrade to non-atomic,
unwrapped-statement behavior *only* under simulation — a degradation
Rust's own simulation path no longer has as of `27UpdatePlan.md` M4
(its `transaction {}` world branch batches every db-verb inside it
into one real `commit_batch_checked` call) — since there is no live
database for a real `BEGIN`/`COMMIT` to run against a `SimWorld`, and
every db-verb inside a transaction this checkpoint's own gate allows is
`db.insert`, already world-guarded per statement.

Go's own gated bet (v0.24 M9) reaches the same scope again, via the
same hand-written-restatement shape TypeScript's own pass established
(Go cannot `include_str!` Rust source either): `internal/world/
world.go`'s `World` type (an in-package `failureEngine`/table map/
queue slice, occupying the same position Python's/TypeScript's own
restatements do) fakes the identical `db.insert` + broker publish/
consume pair, gated on the identical `db`/`queue` declaration check,
refused with the identical per-verb/per-capability reasons
`unsupported_sim_capabilities` computes over the same shared HIR
scanner Rust's/TypeScript's own gates use. Go's own production code
gives `transaction {}` **real**, unconditional atomicity
(`database/sql`'s `*sql.Tx`, the same bar TypeScript's and Rust's own
Postgres branches hold) and — like TypeScript — degrades to a guarded
no-op only under simulation, for the identical reason: every db verb this checkpoint's
own gate allows inside a transaction is `db.insert`, already
world-guarded per statement. One Go-specific wrinkle the other two
narrow targets don't have: `cmd/sim_runner/main.go`'s worker-dispatch
table cannot be a Go `switch` on the subject string (two workers
sharing one subject — `examples/sim-broker-slice.ciac`'s own shape —
would be two `case` arms with the same constant value, a compile
error, not merely dead code the way it would be in Rust's `match`
guards or TypeScript's `if`/`else` chain), so it lowers to an
`if`/`else`-chain with a `delivered` flag instead — the same
first-worker-registered-wins semantics, expressed the one way Go's own
`switch` uniqueness rule allows.

Java's own gated bet (v0.25 M9) reaches the same scope a fourth time,
via the same hand-written-restatement shape TypeScript's/Go's own
passes established (Java cannot vendor `ciac-sim`'s Rust source
either): `sim/World.java`'s `World` class (a nested `FailureEngine`/
table map/queue list, occupying the same position Python's/
TypeScript's/Go's own restatements do) fakes the identical `db.insert`
+ broker publish/consume pair, gated on the identical `db`/`queue`
declaration check, refused with the identical per-verb/per-capability
reasons `unsupported_sim_capabilities` computes over the same shared
HIR scanner Rust's/TypeScript's/Go's own gates use. Java's own
production code gives `transaction {}` **real**, unconditional
atomicity too (`TransactionTemplate`, matching Go's/TypeScript's/
Rust's own Postgres branches) and degrades to a guarded no-op
only under simulation, for the identical reason every other narrow
target does: every db verb this checkpoint's own gate allows inside a
transaction is `db.insert`, already world-guarded per statement — the
`transaction {}` wrapper itself is what simulation skips, not anything
inside it. One design choice specific to Java's own architecture: every
class holding a `JdbcClient`/`Queue` field also holds a
constructor-injected, nullable `World` (via Spring's own
`ObjectProvider<World>` — `null` in production, since `World` is never
a `@Component` no production context ever registers one), rather than
threading one shared state object through every call site the way
Go's `*state.AppState`/Rust's `&AppState` do — `Queue.publishJson`
becomes the single choke point every `publish` call site (pipeline
steps and the `publish <Stream>(..)` HIR leaf alike) shares, needing
no world-awareness of its own at either call site. `SimRunner.java`
(`src/test/java/.../sim/SimRunner.java`, test-scoped since `MockMvc`/
`spring-test` never sit on the packaged application's own classpath)
resolves the milestone's own pre-registered "SimRunner packaging" open
question: not `@SpringBootTest`, not a `sim` Spring profile on the main
jar, but a plain `AnnotationConfigApplicationContext` scanning every
package below the service root *except* `Application` itself (whose
conditional `@EnableScheduling`/`@EnableWebSocket` would otherwise
activate Spring's own background timer/WebSocket machinery — exactly
the real side effects a scenario's own explicit `advance`/`drain`
steps exist to replace) plus one manually-registered `World` bean,
driving requests through Spring's own standalone `MockMvc` (`@RestController`
beans and `@RestControllerAdvice` gathered by annotation, no embedded
servlet container, no bound port) and worker/job beans directly via
their own `handleMessageOnce`/`handleTickOnce` entry points — the same
"real routes, real handlers, no live listener" contract every other
target's own runner already holds, reached without needing Spring
Boot's own `SpringApplication` bootstrap (and its banner/startup
logging) at all.

Single-service projects only, every target: `ciac sim` refuses cleanly
(not a crash, not a silent partial run) when it finds more than one
project descriptor (`pyproject.toml`/`Cargo.toml`/`package.json`/
`go.mod`/`pom.xml`) under `--out`. Multi-service simulation — one
driver process per service, coordinated through one shared virtual
clock — is real future work, not attempted here for any target.
`--record`/`--replay` remain Python-only: no generated-runner target
(Rust's, TypeScript's, Go's, Java's) has plan/replay-tape support (a
plain scenario interpreter, not the bounded child protocol below).

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

### Go's protocol mirrors Rust's and TypeScript's own, line-for-line

Same reasoning again, same shape: `ciac build`/`verify --target go`
emits `cmd/sim_runner/main.go` whenever the program declares `db` or
`queue`, dispatching on the closed `request`/`publish`/`advance`/
`drain`/`expect` step vocabulary exactly like `sim_runner.rs`/
`sim_runner.ts` do — a generic scenario interpreter generated per
program, needing concrete per-program route/worker/job names baked in
at codegen time, the same reason Rust's and TypeScript's own runners
are generated rather than embedded. `cmd/sim_runner` is a second `main`
package Go's own `go build ./...`/`go vet ./...`/`go test ./...`
already discover automatically (the same way Cargo auto-discovers
`src/bin/sim_runner.rs`), so no `TargetInfo.validate` change was needed
to pick it up — resolving 24UpdatePlan.md's own pre-registered open
question about third-binary packaging. `ciac sim --target go` builds it
once (`go build ./cmd/sim_runner`), then runs it once per `--scenario`:

```text
go run ./cmd/sim_runner scenario.json
```

No implementation-level wrinkle equivalent to TypeScript's `{ logger:
false }`: `net/http/httptest`'s recorder writes to an in-memory buffer,
never stdout, and Go's own `log/slog` default handler already writes
to stderr — so the runner's one-line `SimScenarioOutcome` JSON reply on
stdout is never at risk of interleaving with anything else, the same
freedom Rust's `tracing` setup already had.

### Java's protocol mirrors Rust's/TypeScript's/Go's own, with a Maven-native build/run split

Same closed step vocabulary, same generated-per-program shape (route/
worker/job names baked in at codegen time), different toolchain: `ciac
build`/`verify --target java` emits `src/test/java/.../sim/
SimRunner.java` whenever the program declares `db` or `queue`, and the
generated `pom.xml` gains one more plugin — `exec-maven-plugin`,
preconfigured with `SimRunner`'s main class and the `test` classpath
scope — purely to drive it; nothing about the packaged application
changes. `ciac sim --target java` compiles it once (`./mvnw
test-compile`), then runs it once per `--scenario`:

```text
./mvnw exec:java -Dexec.args=scenario.json
```

No implementation-level wrinkle equivalent to TypeScript's `{ logger:
false }` either: `SimRunner` never calls `SpringApplication.run` (see
above), so Spring Boot's own startup banner/INFO logging never fires;
`MockMvc`'s own one-time "Initializing Spring TestDispatcherServlet"
log lines land *before* the scenario runs, never after, so the
runner's one-line `SimScenarioOutcome` JSON reply is always the true
last line of stdout — the same "parse the last line" contract every
other target's own runner already relies on, confirmed live rather
than assumed.

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

# Go: identical shape and identical refusal behavior to Rust's/TypeScript's.
ciac sim service.ciac -t go -o build/ --scenario sim/checkout.ciac-sim.json

# Java: identical shape and identical refusal behavior to Rust's/TypeScript's/Go's.
ciac sim service.ciac -t java -o build/ --scenario sim/checkout.ciac-sim.json
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

## Fidelity boundary: families with no cheap real counterpart

27UpdatePlan.md M3's fidelity ratchet compares fake-vs-real wherever a
real counterpart is cheap to stand up — relational semantics against
an embedded SQLite database (`crates/ciac-sim/tests/sqlite_ratchet.rs`,
zero Docker), and cache TTL / broker fan-out remain delegated to the
existing Docker-backed rows under `verify --system`. Three families
have no such cheap real counterpart, and get the opposite treatment: an
explicit statement of what the fake deliberately is not, rather than a
parity claim it can't back up.

- **Email.** `FakeEmail` records sent messages (`to`/`subject`/`body`)
  instead of talking SMTP. It never establishes a connection, never
  validates an address, and never simulates bounce/deferral/rate-limit
  behavior a real mail provider would apply.
- **Search.** `FakeSearch` matches the *shape* `search.query` actually
  lowers to (`{"query": {"query_string": {"query": <text>}}}`) with a
  case-insensitive substring match over each document's JSON — not a
  real query language. There is no ranking, no tokenization, no fuzzy
  matching, and no index configuration; a scenario asserting anything
  beyond "this text appears somewhere in this document" is asserting
  something the fake was never built to model.
- **Auth (claims-lookup).** `FakeAuth` verifies a bearer token by
  direct lookup against claims a scenario configured ahead of time
  (`world.auth.issue`), instead of real JWT/JWKS cryptography. It
  bypasses signature verification, key rotation, and the JWKS HTTP
  round-trip entirely, while keeping scope enforcement real — the same
  simplification behind "dev-identity scope behavior passes with fake
  JWKS and no Keycloak process" (see 17UpdatePlan.md's M8 milestone
  entry). A green `expect.response` on a scope-gated route proves the
  generated authorization *logic* is correct; it proves nothing about
  token forgery resistance or a real identity provider's behavior.

These three boundaries are ported verbatim into each target's own
`world.rs`/`world.py`/(TypeScript/Go/Java restatements as they land)
doc comments — the disclosure lives next to the code it describes, not
only here.

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
