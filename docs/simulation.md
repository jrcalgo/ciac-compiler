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

## Status: Python + Rust + TypeScript + Go + Java (all full) (v0.17 M11, TypeScript v0.27 M6, Go v0.27 M7, Java v0.27 M8, Rust v0.27 M4)

See [backends.md](backends.md)'s Divergence ledger — Open (tracked)
table for this gap's classification and address ("Simulation depth:
only `db.insert` + publish faked", closing in `27UpdatePlan.md`) and
"Multi-service programs refused by `ciac sim`" (closing in
`28UpdatePlan.md`). The table below is this page's own per-surface
detail, not a restatement of the ledger's entry.

| Surface | Python | Rust | TypeScript | Go | Java |
| --- | --- | --- | --- | --- | --- |
| `ciac sim` | done, every capability faked | done as of `27UpdatePlan.md` M4 — every verb `SimWorld` fakes (db/cache/object store/email/search/http/auth), gate-emptiness proven across the whole example corpus | done as of `27UpdatePlan.md` M6 — every verb `world.ts`'s `SimWorld` class fakes (db/cache/object store/email/search/http/auth), gate-emptiness proven across the whole example corpus | done as of `27UpdatePlan.md` M7 — every verb `internal/world`'s `World` struct fakes (db/cache/object store/email/search/http/auth), gate-emptiness proven across the whole example corpus | done as of `27UpdatePlan.md` M8 — every verb `sim.World` fakes (db/cache directly, object store/email/search/external_http via their own wrapper classes' own `ObjectProvider<World>`, auth), gate-emptiness proven across the whole example corpus |
| `verify --sim` | done | same | same | same | same |
| MCP `verify_sim` | done | same | same | same | same |

Rust's ports/adapters seam and generated per-program simulation runner
(v0.17 M11) started at the same narrow slice TypeScript/Go/Java once
occupied; `27UpdatePlan.md` M4 grew `crates/ciac-sim/src/world.rs`'s
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

TypeScript's own restatement started at the same narrow slice
(v0.23 M9) Go/Java still occupy; `27UpdatePlan.md` M6 grew `src/
world.ts`'s `SimWorld` class — a from-scratch, self-contained port
(TypeScript can no more `include_str!` Rust source than Python can, so
this occupies the same position `sim/pyrunner/world.py`'s own
restatement does) — into every remaining verb's own world-guard leaf
in TypeScript's `lower.rs`: `db.get`/`update`/`delete`/`query`/`count`/
`delete_where` (a `LoweredPredicate`-to-JS-boolean-expression compiler,
`world_predicate_expr`, since `SimWorld.db.findWhere`'s own filter only
supports equality), `cache.*`/`object_store.*`/`email.send`/`search.*`/
`http.call` (each keyed by the capability instance's own declared name,
matching `given.cache`/`given.store`/etc.'s own `instance` field), and
`auth` (claims-lookup against `state.world.authVerify` in `auth.ts.j2`,
matching Python's/Rust's own `FakeAuth`, not real JWT/JWKS crypto) —
plus a schema-aware `RelationalSchema` built from the same
`ciac_codegen::migrations::snapshot_schema` the migration DDL itself
reads (so cascade/restrict/unique checks under simulation can never
drift from what production actually enforces). `transaction {}` blocks
are real, atomic under simulation too as of this milestone, closing
the degradation this page previously disclosed here: rather than
Rust's twice-rendered expression branches (one call site emitting code
once for the world path, once for production, letting `db.insert`
calls switch between "call `db_insert_checked` directly" and "push
onto an explicit `BatchOp` accumulator"), TypeScript's `Orientation::
Statement` renders a handler body's statements once, so atomicity
under simulation is instead an *ambient* mode on `SimWorld` itself
(`beginWorldBatch`/`commitWorldBatch`/`rollbackWorldBatch`): while a
batch is open, `dbInsertChecked`/`dbUpdateChecked`/`dbDeleteChecked`
queue instead of applying immediately, and the generated `transaction
{}` wrapper (unchanged in shape otherwise) calls `this.state.world?.
beginWorldBatch()`/`commitWorldBatch()`/`rollbackWorldBatch()` around
its existing body — a "structure may diverge; answers may not" design
choice (Pillar 4), not a departure from Rust's own semantics, live-
verified identically (`sim/atomic-batch.ciac-sim.json` against
`domain-orders.ciac`). `ciac_backend_ts::unsupported_sim_capabilities`
reflects all of this: it always returns empty now, proven by an
in-crate test (`typescript_gate_is_empty_for_the_whole_corpus`)
iterating the whole example corpus. The same structural note Rust's
own M4 disclosed applies here too: `ciac`'s own `SimSupport::Full`
variant is hardcoded to Python's dynamic-import driver
(`crates/ciac/src/commands.rs`), so TypeScript's `TargetInfo` stays
`SimSupport::Narrow` with an always-empty refusal list rather than
switching enum variants. `crud <Name>: <Record>` resources remain
outside this milestone's scope for the identical structural reason
Rust's own M4 found and disclosed: `resource_store.ts.j2` never reads
`this.state.world`, but a scenario's `request` step can only address
`c.apis`, which a `crud` resource's synthesized api node never has —
the same shared `ciac-codegen` `c.apis` builder both backends read
from, so the finding transfers without needing to be re-proven.

Go's own restatement started at the same narrow slice TypeScript did
before `27UpdatePlan.md` M6 closed it; `27UpdatePlan.md` M7 grew
`internal/world/world.go`'s `World` struct — a from-scratch,
self-contained port (Go cannot `include_str!` Rust source either, so
this occupies the same position `sim/pyrunner/world.py`'s/`world.ts`'s
own restatements do), single-mutex-guarded rather than lock-free the
way Node's/Python's single-threaded runtime lets those restatements
be, since a generated Go service's handlers can genuinely run on
concurrent goroutines — into every remaining verb's own world-guard
leaf in Go's `lower.rs`: `db.get`/`update`/`delete`/`query`/`count`/
`delete_where` (a `LoweredPredicate`-to-Go-closure compiler,
`world_predicate_expr`, evaluated against `world.Row` —
`map[string]any` decoded from JSON — via `world.JSONEq`/`Contains`/
`Lt`/`LtEq`/`Gt`/`GtEq` helpers rather than Rust's/TypeScript's own
inline boolean expressions, since Go has no expression-position
boolean-operator overloading to lean on), `cache.*`/`object_store.*`/
`email.send`/`search.*`/`http.call` (each keyed by the capability
instance's own declared name, matching `given.cache`/`given.store`/
etc.'s own `instance` field — resolved via a `bindings`-lookup closure
mirroring TypeScript's own `instance_of`), and `auth` (claims-lookup
against `World.AuthVerify` in `auth.go.j2`'s `VerifyToken`, matching
Python's/Rust's/TypeScript's own `FakeAuth`, not real JWT/JWKS
crypto) — plus a schema-aware `relationalSchema` built from the same
`ciac_codegen::migrations::snapshot_schema` the migration DDL itself
reads (so cascade/restrict/unique checks under simulation can never
drift from what production actually enforces). Go's own production
code already gave `transaction {}` **real**, unconditional atomicity
before this milestone (`database/sql`'s `*sql.Tx`, the same bar
TypeScript's/Rust's own Postgres branches hold — `26UpdatePlan.md` M1's
atomicity work reached Go for free, since `database/sql` gives every
engine including SQLite the same `*sql.Tx` shape); under simulation,
`World`'s own ambient batch mode (`BeginWorldBatch`/`CommitWorldBatch`/
`RollbackWorldBatch`) now stands in for it, the identical design
TypeScript's own M6 introduced for the identical structural reason
(Go, like TypeScript, renders a handler body's statements once —
`Orientation::Statement` — so there is no second, world-only render
pass the way Rust's `Orientation::Expression` gives `transaction {}`
to switch codegen-time between "call `World` directly" and "push onto
an explicit `BatchOp` accumulator"): `defer st.World.
RollbackWorldBatch()` immediately after `BeginWorldBatch` is a safe
no-op once `CommitWorldBatch` has already run, the exact same
"defer rollback, commit clears it" idiom the real-`*sql.Tx` branch
already used (`sql.ErrTxDone`), not a hand-rolled scheme.
`ciac_backend_go::unsupported_sim_capabilities` reflects all of this:
it always returns empty now, proven by an in-crate test
(`go_gate_is_empty_for_the_whole_corpus`) iterating the whole example
corpus. The same structural note Rust's/TypeScript's own M4/M6
disclosed applies here too: `ciac`'s own `SimSupport::Full` variant is
hardcoded to Python's dynamic-import driver
(`crates/ciac/src/commands.rs`), so Go's `TargetInfo` stays
`SimSupport::Narrow` with an always-empty refusal list rather than
switching enum variants. `crud <Name>: <Record>` resources remain
outside this milestone's scope for the identical structural reason
Rust's/TypeScript's own M4/M6 found and disclosed: `resource_store.
go.j2` never reads `st.World`, but a scenario's `request` step can
only address `c.apis`, which a `crud` resource's synthesized api node
never has — the same shared `ciac-codegen` `c.apis` builder every
backend reads from, so the finding transfers without needing to be
re-proven. One Go-specific wrinkle worth naming: `cmd/sim_runner/
main.go`'s worker-dispatch table for the orphan-subject detection
sweep cannot be a Go `switch` on the subject string (two workers
sharing one subject — `examples/sim-broker-slice.ciac`'s own shape —
would be two `case` arms with the same constant value, a compile
error, not merely dead code the way it would be in Rust's `match`
guards or TypeScript's `if`/`else` chain), so it lowers to an
`if`-chain with a `delivered` flag instead — the same
already-drained-above semantics, expressed the one way Go's own
`switch` uniqueness rule allows; found live via `go build` against
`sim-broker-slice.ciac`'s own fanout scenario, not anticipated.

Java's own restatement started at the same narrow slice Go's own did
before `27UpdatePlan.md` M7 closed it; `27UpdatePlan.md` M8 closes
Java's own gate the identical way, via the same hand-written-
restatement shape TypeScript's/Go's own passes established (Java
cannot vendor `ciac-sim`'s Rust source either): `sim/World.java`'s
`World` class (occupying the same position Python's/Rust's/
TypeScript's/Go's own restatements do) now fakes every remaining
verb a typed handler can call — `db.get`/`update`/`delete`/`query`/
`count`/`delete_where` (in addition to the narrow `db.insert`/publish),
schema-aware reference/unique/cascade checking (`WorldTable`/
`WorldReference`, computed once at codegen time from the same source
the migration DDL is built from, mirroring Rust's/TypeScript's/Go's
own `sim_world_tables`), a group-aware broker log (`BrokerLog`, true
fan-out — two workers sharing one subject each see every message,
matching Rust's/TypeScript's/Go's own M4/M6/M7 fix), a virtual clock,
`cache`, `object_store`, `email`, `search`, `external_http`, and
`auth` (claims-lookup, not real JWT/JWKS crypto, matching Python's own
`FakeAuth`) — so `unsupported_sim_capabilities` always returns empty,
proven by the same gate-emptiness test the other three restatements
carry.

One structural choice specific to Java's own architecture (Pillar 4:
"structure may diverge; answers may not"): `db`/`cache` verbs get a
`lower.rs` world-guard leaf directly (both bind to raw Spring types --
`JdbcClient`/`StringRedisTemplate` -- that can't embed world-awareness
of their own), while `object_store`/`email`/`search`/`external_http`
instead push their world-awareness into their own wrapper classes
(`ObjectStore`/`Email`/`Search`/`ExternalHttp`, each holding its own
constructor-injected, nullable `World` via Spring's own
`ObjectProvider<World>` -- `null` in production, since `World` is
never a `@Component` no production context ever registers one) --
mirroring the *existing* precedent `Queue.java` already established
for `publish` before this milestone (`Queue.publishJson` was already
the one choke point every `publish` call site shares, needing no
world-awareness of its own at the call site). `lower.rs`'s own
`object_store_put`/`get`/`delete`/`list`, `email_send`,
`search_index`/`query`, `http_call` leaves therefore needed *zero*
changes for this milestone -- only the four wrapper classes and
`AppState`'s own `@Bean` factory methods (threading each instance's own
declared name into the new `instanceName` constructor parameter) did.

Java's own production code already gave `transaction {}` **real**,
unconditional atomicity (`TransactionTemplate`, matching Go's/
TypeScript's/Rust's own Postgres branches) before this milestone; M8's
own job was closing the *simulation*-side gap the narrow scope left
open -- only `db.insert` was world-guarded pre-M8, so a `transaction
{}` block mixing `db.insert` with `db.update`/`db.delete` had no
atomicity guarantee spanning the whole block under simulation once
those verbs gained their own per-statement world-guard. `World`'s own
ambient-batch-mode mechanism (`beginWorldBatch`/`commitWorldBatch`/
`rollbackWorldBatch`, mirroring TypeScript's/Go's own M6/M7 design)
closes it: `transaction_stmt`'s world branch now wraps the lambda body
in `beginWorldBatch()` / a `try { ...; commitWorldBatch(); } finally {
rollbackWorldBatch(); }`, so a validation failure partway through
leaves the store exactly as it was before the call, the same guarantee
production's own `TransactionTemplate` already gave. `SimRunner.java`
(`src/test/java/.../sim/SimRunner.java`, test-scoped since `MockMvc`/
`spring-test` never sit on the packaged application's own classpath)
resolves the same "SimRunner packaging" question the narrow slice
already had answered: not `@SpringBootTest`, not a `sim` Spring
profile on the main jar, but a plain
`AnnotationConfigApplicationContext` scanning every package below the
service root *except* `Application` itself (whose conditional
`@EnableScheduling`/`@EnableWebSocket` would otherwise activate
Spring's own background timer/WebSocket machinery) plus one manually-
registered `World` bean, driving requests through Spring's own
standalone `MockMvc` and worker/job beans directly via their own
`handleMessageOnce`/`handleTickOnce` entry points. M8 found one
further wrinkle live: `SecurityConfig`'s own `securityFilterChain`
`@Bean` needs a real `HttpSecurity` bean that only exists under a real
`SpringApplication`, never true here — `SecurityConfig`'s own bean
definition is now removed by name right after the scan and before
`ctx.refresh()` (a no-op on a program with no `auth` at all), and
every `JwtDecoder` constructor dependency across `ApiController`/
`ResourceController` became an `ObjectProvider<JwtDecoder>` for the
identical reason (`Auth.verifyToken`'s own `world != null` branch
never reaches it anyway, but the real bean must still be optional for
the controller to construct at all under simulation) — both found only
once an auth-declaring program first became sim-reachable this
milestone, not anticipated. A second live-found bug, disclosed since
it predates this milestone but was only exercised for the first time
by `sim-broker-slice.ciac`'s own fanout scenario: `World.findWhere`'s
row/filter comparison used `Objects.equals` directly, which silently
fails whenever a stored integer field's `Long`-typed round-trip (via
`Schemas.MAPPER.convertValue`'s own `TokenBuffer`-preserved
`NumberType`) is compared against a scenario JSON's own `Integer`-typed
filter value — fixed by routing through the same `jsonEq` helper
`db.query`'s own world-guard predicate already needed for the
identical `Integer`-vs-`Long` reason, JSON-serializing both sides
before comparing instead of comparing boxed types directly.

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
