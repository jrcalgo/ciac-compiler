# CIaC v0.26-file — Correctness and Consistency: Converting Disclosures into Decisions (implementation plan)

> Implementation plan. Document number ≠ release number (standing
> precedent since 17UpdatePlan.md; version assigned at execution —
> by the plan-NN→0.(NN−2).0 pattern this arc expects to ship as
> **0.24.0** from today's 0.23.0). Assumes 25UpdatePlan.md shipped in
> full: five backends at parity, the consolidated cost model
> published, and — the input this plan exists to consume — the
> cross-target disclosed-gaps ledger from the five-backend
> retrospective. This is the first arc since v0.16 that adds **no new
> backend and no new language surface**. It spends its entire budget
> on the gaps the previous arcs disclosed instead of closing, because
> the disclosure discipline only retains its value if disclosures
> eventually resolve: every "disclosed, not yet closed" line that
> survives long enough stops reading as honesty and starts reading as
> a euphemism for "abandoned". This arc converts each such line into
> exactly one of two states — CLOSED (the gap no longer exists,
> proven live) or PERMANENT (a design decision with a stated reason,
> recorded in a restructured ledger that makes the distinction
> impossible to miss).
>
> **Correctness contract:** Rust `transaction {}` becomes genuinely
> atomic (the one target where a documented language property is
> currently untrue in production — the arc's headline item, live-
> proven by injected mid-transaction failure against real Postgres,
> MySQL/MariaDB, and SQLite); Java accepts and implements
> `logging Structured` (the one capability a backend's own module doc
> claims while its `supports()` refuses — the last capability-table
> lie); the OAuth2 scope-test exclusion — currently identical
> JWT-only gates in all five backends — closes at full cross-target
> parity via a no-infrastructure RS256/JWKS rig; the workspace and
> all five generated ecosystems gain automated dependency/
> vulnerability scanning in CI; the divergence ledger is restructured
> into two tables (permanent-by-design vs open/tracked, every open
> row naming the plan file that closes it); and the `.ciac` language
> itself is frozen as **CIaC Language v1.0.0** — its own semver, its
> own deprecation policy, its own CI-created tags and GitHub
> releases, explicitly decoupled from the compiler's version. M9 cuts
> the first compiler release in the project's history (the tag-
> triggered five-platform release workflow has existed since v0.13
> and has never fired; the repo has zero tags; `install.sh` promises
> a `releases/latest` that does not exist — this arc makes that
> sentence true).
>
> **Confidence:** high on five of the seven workstreams — Java
> logging, the OAuth2 rig, supply-chain scanning, the ledger
> restructure, and the language freeze are all additive, well-scoped,
> and carry no interaction with generated-code behavior that the
> golden/equivalence machinery cannot see. Medium, deliberately and
> explicitly, on the Rust atomicity fix: it is the exact change
> v0.16 M6 assessed and deferred as "materially larger, riskier",
> and this plan attempts it anyway — the difference is that the plan
> now carries two concrete candidate designs (Pillar 1), a
> pre-registered decision point between them, a fallback that
> preserves the current disclosed state if both fail the blast-radius
> budget, and the five-target equivalence suite as a behavior oracle
> that did not exist when v0.16 deferred. Medium also on the
> first-release milestone, for the boring reason that a workflow that
> has never fired has never been debugged; M9 budgets for one
> tag-delete-retag iteration without shame.

## The gap this version closes

The five-backend arc ended with a sentence that this plan refuses to
leave as the last word: every gap is disclosed. Disclosure was the
right discipline while the factory was being built — an honest
ledger beats silent divergence in every world — but the punch-list
review that followed the arc read that ledger the way an outside
evaluator would, and the reading was uncomfortable. Four of five
targets simulate two effects while one simulates nine, and the
ledger calls that "narrow (disclosed)". The flagship language
feature `transaction {}` is atomic on four targets and quietly
sequential on the fifth, and the ledger calls that "disclosed
non-atomic gap, standing cross-reference maintained". A backend's
own module documentation advertises a logging stack its `supports()`
list refuses at compile time. Scope enforcement — the security
boundary — is tested no-infra on exactly one of the two auth schemes
the language accepts, on all five targets, with five identical
comments explaining why. None of these is hidden. All of them are
open. And an evaluator cannot tell, from the ledger alone, which of
them the project considers finished-by-decision and which it
considers unfinished-by-shortage — which means every row erodes
trust in every other row.

This version closes the meta-gap: it makes the project's disclosed
state and its intended state the same state. The correctness
items — Rust atomicity, Java logging — get fixed outright, because
they are not defensible divergences; they are places where the
documentation and the artifact disagree, and in both cases the
documentation describes the better system. The consistency items —
OAuth2 scope testing, the ledger's shape, the language's own
versioning — get brought to the same uniform standard the backends
themselves were held to during the parity arcs. And the trust items
that no amount of internal discipline can self-certify — supply
chain, releases — get external, automated machinery: scanners that
run on every push and every week thereafter, and a release pipeline
that has actually released something.

What this version deliberately does not do: touch simulation depth
or multi-service simulation. Those are the two largest open rows in
the ledger and they get their own arcs (27UpdatePlan.md and
28UpdatePlan.md, already scoped), because mixing a 30-site lowering
change to the Rust backend into the same arc as a five-target
simulation-world rebuild would make both unreviewable. This arc is
sequenced first, on purpose: 27's Rust simulation work will fake
transactions on top of whatever transaction lowering exists, so the
atomicity fix must land before the deepened fake is built against
it, not after; and the restructured ledger this arc produces is the
scoreboard the two simulation arcs will be marking rows off of.

## Pillar 1 — Rust transaction atomicity: the executor seam

### What is true today

`transaction { ... }` is fully validated by sema on every target and
genuinely atomic on four of them: Python wraps the block in the
session's transaction machinery, TypeScript and Go hold a real
database transaction (`database/sql`'s `*sql.Tx` on Go), Java runs
the body inside `TransactionTemplate::executeWithoutResult`. Rust —
the target whose entire pitch is compile-time rigor — lowers the
block via `transaction_expr` (`crates/ciac-backend-rust/src/
lower.rs:429-433`) as the body **unwrapped**, prefixed with a
comment: `// NOTE: this block is not yet atomic on the Rust backend
(see docs/language.md)`. Every `db.*` verb inside still executes
against the shared pool (`db_insert_expr` at lower.rs:322,
`db_update_expr` at :345, the three `query_expr` arms at
:370/:382/:393, and their siblings), so a failure after the first
write leaves the first write committed. The other targets' docs
carry standing "unlike Rust's own disclosed non-atomic gap" callouts
(docs/backends.md ~:297-301, :320-329, :387-397; docs/simulation.md
~:63-64, :80-82, :108-109) — six separate places where the project
reminds the reader that its most rigorous target has its least
rigorous transactions.

The recorded reason (the v0.16 M6 assessment, preserved verbatim at
lower.rs:412-428) is the executor-threading problem: sqlx's
`Transaction<'_, DB>` is a distinct borrowed type with no
`Deref`-to-pool trick, so already-generated `.execute(&self.db)`
call sites cannot transparently keep working inside a transaction.
Making the block atomic means every db-verb arm must be able to emit
a different executor expression depending on whether it is lowering
inside a `transaction {}` — and the assessment counted 30+ recursive
lowering sites that can nest a db verb, judged threading a choice
through all of them "materially larger, riskier" than the v0.16
budget allowed, and deferred with the NOTE marker. The assessment
was correct then. This arc funds it now, with two designs the 2016
assessment did not weigh because the machinery they rest on did not
exist yet.

### What the four atomic targets teach (and why none of it ports)

Each of the four working transaction lowerings was cheap for a
reason Rust cannot borrow, and naming the reasons is what keeps
this pillar from cargo-culting a neighbor's shape:

- **Python** holds a session object whose verbs all already route
  through it; `transaction {}` lowers to the session's transaction
  context manager and every inner verb is *unchanged*, because the
  session was the executor all along. Rust's verbs bind the
  executor at each call site — there is no ambient session to
  scope.
- **TypeScript** passes a client/transaction handle that satisfies
  the same interface as the pool (structural typing does the
  `Deref` trick nominal Rust refuses); inner verbs take the handle
  parameter they always took.
- **Go**'s `*sql.Tx` and `*sql.DB` both satisfy a tiny query
  interface the generated code was written against from day one
  (24's plan chose that seam deliberately, with this exact
  divergence in view).
- **Java**'s `TransactionTemplate` wraps a `Runnable` body and
  thread-binds the connection underneath; inner verbs resolve the
  bound connection transparently through the JDBC/Spring stack.

The common thread: in all four, the *inner verbs* never needed to
know. Rust is the only target whose db verbs textually name their
executor with a type that has no common interface between pool and
transaction usable at the generated code's level of abstraction —
which is why the fix must make the *lowering* context-aware (design
A) or unify the executor type by construction (design B), and why
"just do what Go did" was never on the table: Go's seam was chosen
before any code existed; Rust's 30+ sites already exist.

### Candidate design A (primary): a context flag fed by shared enter/exit hooks

The insight that shrinks "threading through 30+ sites" to something
tractable: the db-verb arms do not each need an executor
*parameter* — they need access to one bit of ambient context ("am I
inside a transaction right now"), and the lowering struct they all
already borrow (`&self`) can carry that bit in a `Cell<u32>`
(depth-counted, since sema permits nesting decisions to be its own
problem). Every db-verb arm then asks a single new helper,
`self.executor_expr()`, which returns `"&self.db"` at depth zero and
`"&mut *__tx"` inside a transaction. The 30+ sites change
mechanically — each swaps a hardcoded executor token for the helper
call — and the compiler enforces completeness: a grep for the old
literal token in `lower.rs` must return only the helper itself.

The catch, and the one shared-crate amendment this design needs: the
shared lowering driver in `ciac-codegen`'s `lower` module calls each
`HostSyntax` leaf with **pre-lowered inner text** — by the time
`transaction_expr(inner)` runs, the body's db verbs were already
lowered, so a flag set inside `transaction_expr` is set too late.
The fix is a pair of optional context hooks on the `HostSyntax`
trait — `enter_transaction(&self)` / `exit_transaction(&self)`,
default implementation empty — that the shared driver invokes around
lowering a transaction statement's children. Default-empty means
Python/TS/Go/Java implement nothing and notice nothing (their
transaction leaves already receive the inner text under their own
uniform-session assumptions); Rust implements both as
increment/decrement of the depth cell. This is exactly the shape of
shared-crate amendment the factory arcs allowed themselves —
narrow, defaulted, adopted by one target, visible to the
conformance harness — and it is the only shared-code change this
pillar makes.

With the flag in place, `transaction_expr` itself becomes real:

```text
let mut __tx = self.db.begin().await?;   // engine-generic: Postgres/MySql/Sqlite
<inner, lowered at depth ≥ 1, all verbs against &mut *__tx>
__tx.commit().await?;
```

Error propagation needs no explicit rollback arm: sqlx's
`Transaction` rolls back on drop if not committed, so any `?` inside
the body unwinds correctly — which is also the property the live
proof asserts rather than assumes. The `begin()` call must be
emitted per engine (the pool types already diverge per engine since
v0.13 M1: `PgPool`/`MySqlPool`/`SqlitePool`, with the placeholder-
style machinery alongside), but `begin()` is uniform across all
three, so the engine dimension costs nothing new here.

### The emitted shape, before and after

Concretely, for a typed handler whose body is `transaction {
db.insert(orders, ...); db.insert(order_events, ...) }`, today's
emission (eliding the envelope/validation scaffolding that does not
change) is:

```text
// NOTE: this block is not yet atomic on the Rust backend (see docs/language.md)
sqlx::query("INSERT INTO orders (...) VALUES ($1, $2, ...)")
    .bind(...)
    .execute(&self.db)
    .await?;
sqlx::query("INSERT INTO order_events (...) VALUES ($1, $2, ...)")
    .bind(...)
    .execute(&self.db)
    .await?;
```

and design A's emission becomes:

```text
let mut __tx = self.db.begin().await?;
sqlx::query("INSERT INTO orders (...) VALUES ($1, $2, ...)")
    .bind(...)
    .execute(&mut *__tx)
    .await?;
sqlx::query("INSERT INTO order_events (...) VALUES ($1, $2, ...)")
    .bind(...)
    .execute(&mut *__tx)
    .await?;
__tx.commit().await?;
```

Three properties of the after-shape worth naming because the
goldens will show them at scale: the SQL strings, bind orders, and
placeholder styles are **byte-identical** to before (the executor
is the only token that moves — which is why the placeholder-trap
machinery from v0.13 M1 is untouched); the `__tx` binding is
hygienic (double-underscore prefix, the repo's standing convention
for generated locals, no collision with user-named handler
bindings because sema already rejects `__`-prefixed identifiers);
and the shape nests soundly — an inner `transaction {}` at depth ≥ 1
is a sema-level question this plan does *not* reopen (nested
transactions were not meaningfully supported by any target's
lowering semantics; whatever sema accepts today it must accept
tomorrow, and depth counting exists precisely so an inner block can
emit nothing new and simply continue against the outer `__tx`,
matching Python's flat-session behavior — the cross-target
behavior, asserted by an equivalence case, not a fresh decision).

The engine dimension, spelled out since the goldens will show all
three: `begin()` exists with identical signature on
`PgPool`/`MySqlPool`/`SqlitePool`, the transaction types are
`Transaction<'_, Postgres|MySql|Sqlite>`, and `&mut *__tx` derefs
to the engine's connection type in all three cases — the emitted
text is literally identical across engines, with only the
already-engine-generic pool type behind `self.db` differing. SQLite
adds one caveat the live proof must respect rather than paper
over: its transactions serialize writers, so the atomicity example
keeps its transaction bodies short (two statements), which it
would anyway.

### Candidate design B (fallback): uniform-connection execution

If the enter/exit hook amendment turns out to fight the shared
driver's recursion shape (the pre-registered failure mode: some
lowering path reaches a db verb without passing through the driver's
statement-walk, so the hooks miss it), the fallback is blunter:
change **every** db verb to execute against `&mut *__conn` and make
the handler wrapper bind `__conn` once — from `pool.acquire().await?`
in handlers without transactions, from `pool.begin().await?` (via
`DerefMut` to the underlying connection) inside them. This removes
the context-sensitivity entirely (one executor expression, always)
at the cost of a real behavior change outside transactions:
non-transactional handlers would hold one pooled connection for
their full duration instead of borrowing per-statement, which under
load changes pool-exhaustion characteristics. That cost is why B is
the fallback and not the primary: A changes nothing outside
`transaction {}` blocks, and "the fix changed nothing except the
thing it fixed" is the property the goldens are best at proving.

The decision between A and B is pre-registered for M1 (Open
questions, item 1) and made on evidence: A is attempted first; if
the hook pair lands and the depth flag covers every db-verb site
(the grep-for-literal completeness check plus the golden diff
showing executor changes confined to transaction bodies), A ships.
B activates only on a named, recorded failure of A, and the
recording goes in this file's M1 Shipped note.

### What must not change

Three invariants bound the blast radius, and each has an oracle:

1. **Non-transactional lowering is byte-identical** under design A.
   Oracle: the golden diff for every example that contains no
   `transaction {}` must be empty except for nothing at all — any
   churn outside transaction-bearing files fails the milestone.
2. **Behavior is unchanged where it was already correct.** Oracle:
   the five-target equivalence suite (which did not exist when v0.16
   deferred this fix — it is the single biggest reason the risk
   assessment differs now) runs unchanged; every case that passed
   before passes after, on all five targets.
3. **Simulation outcomes stay byte-exact.** The v0.17 M11 world-
   guard branches (`if let Some(world) = ...`) sit above the
   executor choice — the fake path never touches sqlx — but the
   transaction leaf is exactly where guard and executor meet, so
   the canonical anchors (`{"ProcessOrder":3}/{"Reconcile":1}` and
   `{"ProcessOrder":100}/{"Reconcile":7}`) are re-proven live on the
   Rust target before M2 exits.

### Where the world-guard meets the executor

The one place Pillar 1 and the simulation machinery share a line of
generated code is the transaction leaf, and the interaction
deserves its own statement because 27UpdatePlan.md builds directly
on whatever this milestone leaves there. Since v0.17 M11 the Rust
transaction lowering has carried a world-guard: under simulation
(`world` present) the block's body executes against the fake with
no real database in reach; in production (`world` absent) the real
path runs. Design A changes only the real branch:

```text
if let Some(world) = &self.world {
    // sim: body against the fake, guarded per-verb as today —
    // v0.17's disclosed non-atomic-under-sim degradation, which
    // 27UpdatePlan.md upgrades when commit_batch parity arrives
    <inner, world-guarded>
} else {
    let mut __tx = self.db.begin().await?;
    <inner, against &mut *__tx>
    __tx.commit().await?;
}
```

which surfaces a real subtlety the milestone must handle rather
than discover: the inner body is lowered **twice** with different
executor context (guarded/fake in the world branch, depth ≥ 1 in
the real branch), or lowered once with both concerns threaded —
whichever the current guard emission already does for
`db.insert`'s dual paths is the pattern to follow, and the depth
cell must be scoped to the real branch's lowering only (the fake
path never touches sqlx and must not inherit a transaction-context
flag that means nothing there). This is exactly the kind of
two-concerns-one-leaf collision that the exit checklist's
"both sim anchors byte-exact" line exists to catch, and it is the
reason this arc runs *before* the sim-depth arc: 27 will replace
the world branch's degraded body with a `commit_batch`-backed
atomic fake, and it should do that against a real branch whose
shape is final, not a moving target.

### The live proof

Golden diffs prove the shape; only a real database proves atomicity.
M2's acceptance is a purpose-built example (or an extension of
`domain-orders`, whichever needs fewer moving parts — decided at
implementation, recorded) whose handler performs two inserts inside
one `transaction {}` where the second insert violates a constraint.
The proof asserts, against **all three engines** (real Postgres and
MariaDB via compose; SQLite zero-Docker, which also makes this the
one atomicity proof that runs fully local): the handler returns the
error envelope, and the table contains **zero** rows from the failed
transaction — not one. The same scenario runs on Python/TS/Go/Java
as an equivalence case, which upgrades the fix from "Rust caught up"
to "the property is now uniformly tested on every target that claims
it" — the stronger sentence, and the one the ledger will carry.

### The documentation debt this retires

On M2 exit, six standing callouts disappear: the NOTE marker in
generated code, docs/language.md's transactions caveat, and the four
"unlike Rust" cross-references in backends.md/simulation.md. The
ledger row moves from Open to CLOSED with the proof named. The
docs sweep is listed as its own exit-checklist line because
cross-reference rot is precisely the failure mode this arc exists
to end — closing the gap while leaving the callouts would recreate
the problem in mirror image.

## Pillar 2 — Java `logging Structured`: the last capability-table lie

### What is true today

`Component::Logging { provider: LoggingProvider::Structured }` is a
first-class capability: declared as `use { logging Structured; }`,
built by sema (`ciac-sema/src/build.rs:911`, provider table :2146),
carried on the IR (`ciac-ir/src/component.rs:265-268`), and
implemented by four backends as a modifier on their always-present
observability init — Python switches structlog to `JSONRenderer`,
Rust switches the `tracing_subscriber` fmt layer to `.json()`, Go
installs slog's JSON handler, TypeScript's observability template
does the equivalent — all keyed off the shared `has_logging` model
flag (`ciac-codegen/src/model.rs:1371`).

Java refuses it. `JavaBackend::supports()` is the only backend with
a discriminating match list (`crates/ciac-backend-java/src/
lib.rs:219-240`), and `Logging` is absent from it, so
`check_support` (`ciac-codegen/src/lib.rs:274-284`) turns any Java
program declaring `logging` into a hard **CIAC0011** at build time.
Meanwhile the same file's module-doc capability table (lib.rs:15)
claims `| Logging | SLF4J + Logback + logstash-logback-encoder |` —
a stack the backend never emits. Two mitigating facts and one
aggravating one: the refusal is loud rather than silent (good), no
checked-in example declares `logging` so the corpus never trips it
(the reason it survived the M8 whole-repo sweep), and — the
aggravating one — `docs/targets.json` cannot arbitrate, because the
`typescript`/`go`/`java` entries all carry empty `"capabilities":
{}` maps (targets.json :166/:194/:210) while python/rust are fully
populated, a second, quieter consistency failure this pillar's
milestone also fixes (Pillar 5 carries the enforcement).

### The implementation

The user's decision is to make the module doc's claim true rather
than delete it, and the claimed stack is the right one — it is the
standard structured-logging answer in the Spring ecosystem and it
slots into the existing template surface without touching handler
code:

1. **`supports()`**: add `Component::Logging { .. }` to the match
   list, with the comment block at lib.rs:210-218 extended to say
   why it was late (refused through v0.25 because the emission
   didn't exist; the module-doc table claimed it anyway; this
   milestone reconciled them in the direction of implementation).
2. **`logback-spring.xml.j2`** (new template, emitted only when
   `has_logging`): a Logback configuration installing
   `net.logstash.logback.encoder.LogstashEncoder` on the console
   appender — JSON lines with timestamp, level, logger, message,
   MDC. Without `logging Structured`, no file is emitted and Spring
   Boot's default human-readable console format stands, exactly
   matching the other four targets' declared-vs-default behavior.
3. **`pom.xml.j2`**: `logstash-logback-encoder` dependency, exactly
   pinned like every other dependency, gated on `c.has_logging` —
   golden-visible, per the determinism discipline.
4. **The module-doc table** stays as written — it becomes true.

The trap this pillar pre-names (Pillar 4 of 25UpdatePlan.md's
Spring-magic discipline applies): Logback configuration is
classpath-magic — Spring Boot picks up `logback-spring.xml`
automatically. That is fine (it is the idiomatic mechanism, and
"generated file, auto-discovered by the framework" is exactly how
`application.yml` already works) but it means the *absence* case
needs a test as much as the presence case: a program without
`logging` must not emit the file, and the `NoInfraBootTest` pattern
must stay green in both shapes.

### The template, as drafted

`logback-spring.xml.j2` is small enough to draft in the plan, which
also fixes the key-set contract `LogShapeTest` will assert:

```text
<?xml version="1.0" encoding="UTF-8"?>
<!-- Generated by CIaC: `logging Structured` -> JSON console logs
     via logstash-logback-encoder. Absent this declaration, no file
     is emitted and Spring Boot's default console format stands. -->
<configuration>
  <appender name="CONSOLE" class="ch.qos.logback.core.ConsoleAppender">
    <encoder class="net.logstash.logback.encoder.LogstashEncoder"/>
  </appender>
  <root level="INFO">
    <appender-ref ref="CONSOLE"/>
  </root>
</configuration>
```

LogstashEncoder's default field set — `@timestamp`, `@version`,
`message`, `logger_name`, `thread_name`, `level`, `level_value`,
plus MDC — is the assertion target: `LogShapeTest` parses one line
and asserts `@timestamp`/`level`/`logger_name`/`message` present
and the line as a whole valid JSON. Deliberately no customization
of the field set in this milestone (no custom providers, no
shortened field names): the capability's contract is "structured
JSON logs", not a specific schema, and the other four targets'
JSON field names differ by ecosystem already — a cross-target
log-schema unification is exactly the kind of scope creep the
Explicit cuts section forbids. The `<configuration>` scan settings,
file appenders, rolling policies: all absent on purpose; log
*routing* is deployment's business (containers log to stdout;
compose/k8s collect), and the generated `application.yml` already
holds the log-level surface.

### Corpus and proof

No example exercises `logging` on any target — the capability's
five-target story has never once been run end-to-end. M3 fixes the
corpus, not just Java: extend one existing observability-bearing
example with `logging Structured` (candidate: whichever example
already carries metrics/tracing so the observability surface
concentrates in one place; a new minimal `structured-logs.ciac` is
the fallback if extension churns too many goldens). Proof, per
target: build + verify as usual, plus a Java-specific `LogShapeTest`
(boot the context with the sim-style no-infra discipline, emit one
log line through a generated component's logger, parse it as JSON,
assert the key set) — the same claim the other targets make
implicitly through their observability templates, now asserted
explicitly on the target that lied about it. The five-target CI
example loops pick the example up automatically; `generated-java`'s
CIAC0011-skip branch stops matching it, which is itself the
regression test that the refusal is gone.

## Pillar 3 — The no-infra OAuth2 rig: scope tests at full parity

### What is true today

The language accepts two auth schemes (`auth JWT`, `auth OAuth2` —
`AuthScheme` at ciac-ir/src/component.rs:90) and every backend
generates enforcement for both: HS256 shared-secret validation for
JWT, RS256-via-JWKS resource-server validation for OAuth2 (issuer's
`/.well-known/jwks.json`, v0.11 M2). Scopes are per-endpoint
(`ApiConfig.scope`, `CrudConfig.read_scope`/`write_scope`) and the
enforcement code paths are scheme-independent above token
validation.

The *testing* is not symmetric. Every one of the five backends
emits its no-infrastructure scope-test suite behind the same gate,
`auth_scheme == "jwt"`: Python's `test_smoke.py.j2` (:7/:63/:106/
:122), Rust lib.rs:400, TypeScript lib.rs:449, Go lib.rs:690, Java
lib.rs:434 — five copies of the same comment explaining that OAuth2
needs real RS256 crypto against a live JWKS issuer, so its scope
enforcement is proven only in the `--system` path against a live
Keycloak (docs/deployment.md:191-196). The comment is true and the
exclusion was the right call in v0.14 M6 — but it means the
security-boundary test exists for the toy scheme and not for the
production one, uniformly, on every target. Same shape as the sim-
depth gap; same resolution: full parity.

### The rig

The claim "OAuth2 scope testing needs a live issuer" contains a
false assumption: it needs a *JWKS endpoint and a keypair*, not an
*identity provider*. Both are cheap to fabricate locally with real
cryptography:

1. **Keypair**: the test generates (or embeds, decided per target by
   whichever is more idiomatic — embedded fixed test keys are
   deterministic and dodge per-run keygen cost; generated keys dodge
   "test key leaked into production config" misreadings; the choice
   is pre-registered as Open question 2, decided once, applied five
   times) an RSA keypair for RS256.
2. **JWKS stub**: an in-process HTTP server owned by the test —
   `httptest.NewServer` on Go, `wiremock`/`axum::serve` on an
   ephemeral port on Rust, `aiohttp`/`pytest-httpserver` on Python,
   `nock` or a bare `http.createServer` on TypeScript, OkHttp's
   `MockWebServer` on Java — serving exactly one route,
   `/.well-known/jwks.json`, with the public key in JWK form (kid,
   kty, n, e).
3. **Issuer override**: the generated config already reads the
   OAuth2 issuer from the environment (that is how compose wires
   Keycloak today), so the test sets the issuer env var to the
   stub's base URL and the production validation code — real JWKS
   fetch, real kid match, real RS256 signature verification —
   resolves against the stub with **zero** generated-production-code
   changes. If any backend's config surface turns out to cache or
   pin the issuer in a way an env override cannot reach at test
   time, the fix is a test-profile override in that backend's
   existing config machinery, not a new knob; any such per-target
   accommodation is recorded.
4. **Token minting**: the test signs RS256 tokens with the private
   key — correct `iss`, `aud`, `exp`, and the scope claim in the
   shape the target's validator reads — and runs the exact same
   matrix the JWT suite runs today: no token → 401; valid token,
   missing scope → 403; valid token, correct scope → 200; and the
   negative-crypto case the JWT suite cannot express as sharply —
   token signed by a *different* key → 401 (the assertion that the
   JWKS fetch and signature check actually gate the door).

Dependency cost: each target gains at most one test-scoped, exactly
pinned dependency (the stub server; the JWT-signing capability
mostly exists already wherever the JWT suite mints HS256 tokens —
RS256 is the same library, different algorithm, plus a key in PEM/
DER form). No production dependency changes on any target.

### The case matrix, as specified

The suite's cases, fixed here so five implementations translate
one table instead of five authors improvising (`S` = an endpoint
requiring scope `orders:write`; all cases run per auth scheme
except the last two, which are OAuth2-only because they exercise
the JWKS path specifically):

| Case | Token presented | Expected |
| --- | --- | --- |
| no_token | none | 401 |
| malformed_token | `Bearer not-a-jwt` | 401 |
| wrong_scope | valid signature, scopes `["orders:read"]` | 403 |
| correct_scope | valid signature, scopes `["orders:write"]` | 200 |
| expired_token | valid signature, `exp` in the past | 401 |
| wrong_key (OAuth2) | RS256-signed by a key **not** in the served JWKS | 401 |
| wrong_issuer (OAuth2) | valid key, `iss` ≠ configured issuer | 401 |

The first five mirror the existing JWT suite's semantics (case
names harmonized across targets in M4 — where an existing JWT
suite spells a case differently, the JWT suite is renamed to
match, a test-only churn the goldens will show); the last two are
the new crypto-boundary assertions that only exist because the rig
uses real verification. Every case asserts the response envelope's
error code field as well as the status — a 403 with the wrong
error body is a failure, same discipline as the equivalence suite.

### Per-target rig inventory

The five implementations of the one contract, with the candidate
libraries named now so M4/M5 execute rather than research (final
choices confirmed against each target's existing test-dependency
set at implementation; any swap recorded):

| Target | JWKS stub server | RS256 mint/verify in test | Issuer override reaches config via |
| --- | --- | --- | --- |
| Python | `pytest-httpserver` (or bare `aiohttp` app on an ephemeral port) | `pyjwt` + `cryptography` (both already present wherever the JWT suite mints HS256) | env var read by the existing settings machinery at app construction — tests already construct the app |
| Rust | `wiremock` (test-scoped) or a two-line `axum` router — decided by which the generated test harness already resembles | `jsonwebtoken` (already the production validation dep; tests use its `EncodingKey` with the test private key) | config struct built from env in tests, same as existing scope tests |
| TypeScript | bare `node:http` server on port 0 (no new dep) | `jose` (already the production JWKS/validation dep — its `SignJWT` + `generateKeyPair`/imported PEM covers minting) | env var consumed by the config module; supertest-style app construction already re-reads it |
| Go | `net/http/httptest.NewServer` (stdlib, no new dep) | `golang-jwt` + stdlib `crypto/rsa` (production dep + stdlib) | env var read at router construction; `t.Setenv` scopes it |
| Java | OkHttp `MockWebServer` (test-scoped, exactly pinned) | `nimbus-jose-jwt` (already on the classpath via the resource-server stack) | `@DynamicPropertySource` or standalone-MockMvc config override — whichever the existing ScopeTests construction supports |

Two structural notes the table encodes: three of five targets need
**zero** new stub dependencies (TS/Go use platform primitives;
Python's is quasi-standard), and on **every** target the
mint/verify library is the production validation library itself —
the suite exercises the same JWKS-parse and signature-check code
paths production runs, which is the entire point of the rig over a
mocked validator.

### Parity discipline

This lands as the JWT suite landed in the parity arcs: textually
parallel test suites across the five targets, same case names, same
matrix, so the conformance-style reading ("diff the five files,
expect only idiom") holds. The five gate expressions widen from
`auth_scheme == "jwt"` to both schemes; the five exclusion comments
are deleted; the divergence-ledger row moves to CLOSED; and
docs/deployment.md's live-IdP section is reworded from "the case
the no-infra suite can't cover" to what it now actually is — the
deeper end-to-end oracle (real Keycloak, real realm, real token
endpoint) layered above a no-infra suite that already proves the
enforcement mechanism with real cryptography. `oauth-echo` (the
existing OAuth2 example) becomes the corpus carrier: its generated
projects must contain the new suite on all five targets, and the
five example-loop CI jobs prove the suites green on every push.

## Pillar 4 — Supply-chain scanning: the workspace and the five ecosystems

### What is true today

Nothing. The survey that scoped this arc found no `deny.toml`, no
`audit.toml`, no `.github/dependabot.yml`, no `cargo audit` or
`cargo deny` invocation, no `pip-audit`, no `npm audit`, no
`govulncheck`, no Java scanner — anywhere, for either the compiler
workspace or the generated projects. Every dependency in the system
is exactly pinned (the determinism discipline has been genuinely
good here: pinned poms, pinned lockfiles, snapshotted wrappers), but
exact pinning without scanning is determinism *of* staleness — the
project can prove byte-for-byte what it depends on and cannot say a
word about whether any of it has a published CVE. And CIaC's
exposure is unusual in shape: a typical project has one dependency
tree; this one has **six** — the Rust workspace that is the
compiler, plus the Python, Rust, TypeScript, Go, and Java trees that
every generated project inherits. The generated trees are the ones
users actually deploy. A vulnerable pinned version in
`pyproject.toml.j2` or `pom.xml.j2` is shipped to every user of that
target on every build, which makes the templates' dependency lists
the highest-leverage supply-chain surface in the entire repository.

### The workspace side

Two tools, complementary, both in CI on every push and on a weekly
schedule (advisories publish between pushes; a quiet repo should
still hear about them):

- **`cargo audit`** against the RustSec advisory database — the
  CVE/advisory check for the compiler's own tree.
- **`cargo deny check`** with a checked-in `deny.toml` — advisories
  (same database, kept as cross-check and because deny's config
  carries the ignore-with-reason machinery), licenses (allowlist:
  the permissive set compatible with this repo's Apache-2.0),
  bans (duplicate-version report kept advisory-level, not failing),
  and sources (crates.io only).

Policy, stated in `deny.toml` comments and docs: severity ≥ high
fails CI; anything accepted-as-unfixable enters the ignore list
**with a reason and an expiry** — an ignore without an expiry date
is itself a CI failure (a tiny script check in the same job), which
is the disclosure discipline applied to the scanner: exceptions
must not be able to rot silently, in a file whose entire purpose is
preventing silent rot.

- **`.github/dependabot.yml`**: cargo (workspace), github-actions
  (the workflow pins), and npm scoped to `editors/vscode`. Weekly
  cadence, grouped minor/patch. Dependabot proposes; the golden
  discipline disposes — a bump PR that changes generated output is
  visible as golden churn and gets reviewed as such.

### The generated-ecosystem side

The novel job — `generated-audit` — scans what users inherit. Shape:
build a representative example per target (the flagship
`order-system` where it verifies on all five, supplemented where a
capability changes the dependency set, e.g. a Kafka example for the
rdkafka/aiokafka trees), then run the ecosystem's scanner against
the generated project:

| Target | Scanner | Scans |
| --- | --- | --- |
| Python | `pip-audit` | generated `pyproject.toml`/lock resolution |
| Rust | `cargo audit` | the generated project's `Cargo.lock` |
| TypeScript | `npm audit --audit-level=high` | generated `package-lock.json` |
| Go | `govulncheck ./...` | generated module (call-graph aware — the least noisy of the five) |
| Java | `grype` on the generated project's resolved classpath | the pom's exact pins |

(The Java scanner choice — grype vs OWASP dependency-check — is
pre-registered as Open question 3; grype is the default candidate
for speed and CI ergonomics, dependency-check the fallback if
grype's Maven resolution proves unreliable. Recorded either way.)

Same policy as the workspace: high+ fails, ignores carry reason and
expiry, and — the part that makes this a compiler feature rather
than repo hygiene — a finding against a *generated* tree is fixed by
bumping the pin **in the template**, which the goldens then show to
every reviewer, and which the next release ships to every user.
The weekly scheduled run means template pins age under supervision.
This job is additive CI cost; it does not gate the fast lint/test
path, and it reuses the toolchain setup the example-loop jobs
already pay for.

### The configuration, as drafted

`deny.toml`'s intended shape (final values confirmed at M6 against
cargo-deny's current schema; the *policy* is what the draft fixes):

```text
[advisories]
version = 2
yanked = "deny"
# Every ignore entry MUST carry `reason` and an `# expires: YYYY-MM-DD`
# comment; the workspace-audit job greps for entries missing either
# and fails — exceptions are not allowed to rot silently.
ignore = []

[licenses]
version = 2
allow = ["Apache-2.0", "MIT", "BSD-2-Clause", "BSD-3-Clause",
         "ISC", "Unicode-3.0", "Zlib"]
# extended at M6 by whatever the actual dependency tree requires,
# each addition reviewed rather than wildcarded

[bans]
multiple-versions = "warn"   # report, don't fail: dupes are a
                             # diet concern, not a security gate

[sources]
unknown-registry = "deny"
unknown-git = "deny"
```

and the two CI jobs' skeleton (names final, steps abbreviated):

```text
workspace-audit:            # every push + weekly cron
  - cargo install cargo-audit cargo-deny (pinned versions)
  - cargo audit
  - cargo deny check
  - scripts/check-deny-ignores.sh   # reason+expiry present, none expired

generated-audit:            # every push + weekly cron
  - build ciac; generate order-system for all five targets
    (+ kafka-pipeline for the divergent broker trees)
  - python: pip-audit against the generated project's resolution
  - rust: cargo audit against the generated Cargo.lock
  - typescript: npm ci && npm audit --audit-level=high
  - go: govulncheck ./... in the generated module
  - java: grype against the generated project (Open question 3)
```

The `check-deny-ignores.sh` expiry gate is deliberately a
ten-line script rather than tooling: it turns the ignore list into
a self-expiring TODO file, and when an expiry passes, CI fails
with the reason string in the output — the person who accepted the
risk wrote the message future-them reads.

### What this pillar is not

Not fuzzing, not load testing, not a hired audit — the decision
scoped this to automated dependency/vulnerability scanning, and the
Explicit cuts section holds that line. The honest sentence after
this pillar ships: "no human has adversarially audited this
codebase, but every dependency in all six trees is continuously
scanned against public advisory databases, with failures gating CI"
— a true sentence that replaces a silence.

## Pillar 5 — The divergence ledger, restructured: permanent vs open

### What is true today

There is no ledger table. The "divergence ledger" the project's own
plans cite is a *practice* — divergences are narrated, thoroughly
and honestly, as prose in docs/backends.md's `## Simulation (v0.17)`
section (:230-448) and echoed in docs/simulation.md's status table
(:29, per-target sim columns). The prose is good; the *index* is
missing. A reader who wants the answer to "what, exactly, differs
across targets, and is each difference a decision or a debt?" must
reconstruct it from paragraphs — and the punch-list review
demonstrated that even a sympathetic reader reconstructs it as "a
lot of open gaps", because prose cannot show status at a glance. The
decision: split into two explicit tables, structurally separating
what the project *chose* from what the project *owes*.

### The two tables

Both live in docs/backends.md, replacing nothing (the narrative
paragraphs remain as the linked detail) but becoming the section's
front matter — the index a reviewer reads first.

**Table 1 — Permanent by design.** Columns: divergence | targets |
why this is a decision, not a debt. Initial rows, migrated from
prose:

- Migrations executor: alembic vs `sqlx migrate` vs node-pg-migrate
  vs golang-migrate vs Flyway — each target uses its ecosystem's
  first-class executor over identical CIaC-owned SQL; the SQL is
  cross-target content-equality-tested, the executor is idiom.
- Cron translation: each scheduler consumes the shared cron
  expression through its ecosystem's library, with the equivalence
  cases proving schedule agreement — the *library* differs forever.
- Deploy artifact shape and size: jar vs binary vs node_modules vs
  venv; the JVM image-size row from 25's retrospective lands here
  with its recorded numbers.
- Go's RFC 3339 fractional-seconds trimming in `MarshalJSON`
  (the confirmed-but-unexercised row from 24) — permanent, wire-
  compatible, documented.
- Per-language error idiom (24's M4 amendment) — `Result` vs
  exceptions vs error returns; the envelope on the wire is
  identical, the in-language shape is the language's.

**Table 2 — Open (tracked).** Columns: gap | targets | closes in.
Initial rows:

- Simulation depth (db.insert+publish only) — Rust/TS/Go/Java —
  **27UpdatePlan.md**.
- Multi-service simulation refused — all five — **28UpdatePlan.md**.
- `transaction {}` non-atomic — Rust — **this plan, M1–M2** (the row
  deletes itself mid-arc; the deletion is part of M2's exit).
- `logging Structured` refused — Java — **this plan, M3**.
- OAuth2 scope tests excluded from the no-infra suite — all five —
  **this plan, M4–M5**.
- Sim record/replay — narrow targets — unscheduled, and now *visibly*
  unscheduled, which is the honest state (a row may say "no plan
  yet"; what it may not do is hide among the permanent rows).

The structural rule the split encodes: a permanent row needs a
*reason*; an open row needs an *address*. A row that has neither is
not allowed to exist — which is the property the old prose could
not enforce and the new shape can.

### The tables, as drafted

Drafting the initial rows in the plan serves two purposes: M7
executes a transcription rather than a research project, and the
plan itself becomes reviewable on the classification calls — if a
reader disagrees that a row is permanent, this document is where
the argument happens, before the table ships. Wording final at M7;
classification decided here.

**Permanent by design** (draft):

| Divergence | Targets | Why this is a decision |
| --- | --- | --- |
| Migrations executor: alembic / `sqlx migrate` / node-pg-migrate / golang-migrate / Flyway | all five | Identical CIaC-owned SQL, content-equality-tested cross-target; the runner is ecosystem idiom, and replacing five first-class executors with one bespoke runner would trade audited machinery for NIH risk |
| Cron translation library | all five | Shared cron expression, per-ecosystem scheduler; equivalence cases prove schedule agreement; the library choice is the ecosystem's, forever |
| Deploy artifact shape/size (jar vs binary vs node_modules vs venv vs module) | all five | The artifact is the language's; 25's retrospective numbers recorded here as the standing expectation, not a bug row |
| JVM image size / startup profile | Java | Inherent to the platform; bounded by 25's recorded baseline; GraalVM/jlink remain a cut, not a promise |
| RFC 3339 fractional-seconds trimming in `MarshalJSON` | Go | Wire-compatible per RFC; confirmed in 24; changing it would mean fighting the stdlib for cosmetics |
| Error idiom in generated code (`Result` / exceptions / error returns) | all five | 24 M4's amendment; the wire envelope is identical, the in-language shape belongs to the language |
| No-`Deref`-to-pool transaction types (executor seam shape) | Rust | After M1–M2 the *gap* is closed; the *seam design* (executor helper + hooks) is recorded here as the permanent shape so future verbs adopt it rather than re-litigate it |

**Open (tracked)** (draft — statuses as they will stand at M7,
mid-arc):

| Gap | Targets | Closes in |
| --- | --- | --- |
| Simulation depth: only `db.insert` + publish faked | Rust, TS, Go, Java | 27UpdatePlan.md |
| Multi-service programs refused by `ciac sim` | all five | 28UpdatePlan.md |
| `transaction {}` non-atomic in production | Rust | this plan M1–M2 — CLOSED at M7, recorded with proof |
| `logging Structured` refused (CIAC0011) | Java | this plan M3 — CLOSED at M7, recorded with proof |
| OAuth2 scope tests excluded from no-infra suite | all five | this plan M4–M5 — CLOSED at M7, recorded with proof |
| Sim record/replay | Rust, TS, Go, Java | no plan yet (visibly unscheduled — this row exists to say so) |
| No external human security audit | repo | no plan yet; automated scanning (this plan M6) is the standing floor, not the ceiling |

### Enforcement

Ledger rot is the failure mode, so the ledger gets a test:
`ledger_integrity` (a unit test in the `tests` crate alongside the
existing docs-consistency tests) parses the two tables out of
backends.md and asserts (1) every "closes in" reference names a
plan file that exists in the repo root, and (2) no gap string
appears in both tables. Cheap, slightly unusual, and exactly the
kind of self-auditing this repo already does for targets.json — 
which is the second half of this pillar: populate the
`"capabilities"` maps for typescript/go/java in docs/targets.json
(empty `{}` today at :166/:194/:210, fully populated for
python/rust) by deriving them in the `targets` command from the
same source the python/rust entries use, and let the existing
`targets_cli.rs` checked-in-JSON-matches-derived test enforce it
forever. After M7, `ciac targets --json` is finally what it always
claimed to be: the single machine-readable source of truth for what
every target supports — including, once M3 lands, Java's truthful
`logging` row.

## Pillar 6 — CIaC Language v1.0.0: freezing the surface

### What is true today

The language has no version. docs/language.md's H1 says "The CIaC
Language (v0.23.0)" — but that is the *compiler's* crate version,
restamped every arc, promising nothing about the syntax. There is no
stability statement, no deprecation policy, no definition of what a
breaking change to the *language* even is. The crates have a semver
story; the DSL a user actually writes has none — nothing anywhere
tells someone "this syntax won't move under you", and after nine
language-surface arcs (v0.2 through v0.16) the surface has in fact
been stable since v0.16: the entire five-backend factory arc ran on
a frozen grammar without once needing to change it, which is the
empirical argument that the surface is *ready* to freeze. The
decision: freeze it as **CIaC Language v1.0.0**, its own semver,
explicitly distinct from the compiler's version, with the
distinction carried everywhere the two could be confused.

### The mechanics of the version

One canonical source: a `LANGUAGE_VERSION` file at the repo root
containing exactly `1.0.0`, consumed two ways — the compiler embeds
it (`include_str!` from `ciac-syntax`, the crate that owns the
surface; exposed as `ciac_syntax::LANGUAGE_VERSION`, trimmed,
compile-time) and CI reads it as a plain file (Pillar 7's workflow
needs the value without building anything). The file-not-const
choice is deliberate: it makes the language version diffable,
greppable, and bumpable in a one-line change whose reviewers see
exactly what they are approving, and it keeps the CI trigger
trivial. Surfaced everywhere an integrator looks:

- `ciac describe` gains `language_version` beside the existing
  `describe_version`/`ciac_version` fields (the precedent this
  follows).
- `ciac targets --json` gains it in the header object;
  targets_cli.rs's test enforces the checked-in copy.
- `ciac --version` prints both: `ciac 0.24.0 (language 1.0.0)`.
- The generated manifest stamps it beside the schema versions it
  already stamps — so a generated project records which language
  version produced it.
- docs/language.md's H1 becomes "The CIaC Language v1.0.0
  (compiler v0.24.0)" — the doc is versioned by the language, the
  parenthetical tracks the implementation.

### The policy

A new top section in docs/language.md (`## Stability and
versioning`), stating, in normative language:

- **What is covered**: everything docs/language.md specifies —
  grammar, declaration forms, capability/provider names, handler-
  body expression semantics, builtin verb signatures and behavior,
  diagnostic *codes* (messages may improve; codes are API).
- **What is not covered**: generated-code internals (file layouts,
  template contents — governed by the regeneration/manifest
  machinery, not the language), CLI flags (compiler semver),
  the JSON schemas (already independently versioned).
- **Breaking (major)**: removing/renaming any covered surface;
  changing the meaning of an accepted program; making a previously
  accepted program rejected. **Additive (minor)**: new
  declarations, capabilities, providers, verbs, fields — anything
  under which every v1.x program keeps compiling with unchanged
  meaning. **Editorial (patch)**: spec clarifications that change
  no acceptance and no meaning.
- **The deprecation ladder**: a covered surface is removed only
  after ≥ one minor version deprecated, during which the compiler
  emits a dedicated warning diagnostic (new code range reserved:
  **CIAC0060–CIAC0069, deprecation warnings**, registered in
  docs/errors.md by M8 even though v1.0.0 ships with the range
  empty — the ladder must exist before the first rung is needed)
  pointing at a migration note in the spec's changelog section.
- **The compiler/language contract**: compiler version X supports
  language version Y means: every program valid under Y compiles,
  and `describe`/`targets` report Y. Multiple compiler versions may
  ship the same language version (expected: most arcs are 0.x
  compiler bumps with the language untouched at 1.0.0 — this very
  arc is the first example). A future in which one compiler
  supports *multiple* language versions (pragma-selected) is
  explicitly out of scope for v1 and noted as such.

The freeze also binds this arc itself, pleasantly: nothing in
Pillars 1–5 touches the language surface (the atomicity fix changes
lowering, not semantics — `transaction {}` finally *means* what the
spec already said), so v1.0.0 can be declared at M8 from a surface
this arc provably did not disturb.

### The normative text, as drafted

The `## Stability and versioning` section's core, drafted here for
review (final wording at M8; the commitments are the plan's):

> The CIaC language is versioned independently of the `ciac`
> compiler. This document specifies **CIaC Language v1.0.0**. The
> compiler version that implements it appears in the title's
> parenthetical and moves freely; the language version moves only
> when the language does.
>
> **Covered surface.** Everything this document specifies: the
> grammar and every declaration form; capability and provider
> names; handler-body expression syntax and semantics; builtin
> verb signatures and their observable behavior; diagnostic codes
> (messages may be reworded; codes are stable API).
>
> **Not covered.** Generated-code internals (file layout, template
> content — governed by the regeneration manifest, not this spec);
> `ciac` CLI flags and output (compiler versioning); the JSON
> schemas (independently versioned).
>
> **Change classes.** *Breaking (major):* removing or renaming any
> covered surface; changing the meaning of a program that
> previously compiled; causing a previously accepted program to be
> rejected. *Additive (minor):* new declarations, capabilities,
> providers, verbs, or fields, under which every existing v1.x
> program compiles with unchanged meaning. *Editorial (patch):*
> clarifications changing no acceptance and no meaning.
>
> **Deprecation.** A covered surface is removed only after being
> deprecated for at least one minor version, during which the
> compiler emits a CIAC006x warning pointing at a migration note
> in this document's changelog.
>
> **Support statement.** Compiler version X *supports* language
> version Y iff every program valid under Y compiles under X with
> Y's meaning, and X reports Y via `ciac describe` and
> `ciac targets --json`. Multiple compiler versions may — and
> typically will — ship the same language version.

One deliberate omission from v1 the draft makes explicit in a
closing paragraph: there is no mechanism for one compiler to
support *multiple* language versions simultaneously (no `#lang`
pragma, no per-file version selection). If v2 ever exists, that
mechanism is v2's price of admission, designed then; promising it
now would be speculative surface.

### The changelog, seeded

docs/language.md gains a `## Changelog` section whose first and
only entry M8 writes:

> **v1.0.0** (compiler 0.24.0) — Initial frozen specification.
> Covers the surface as stabilized in compiler v0.16 and carried
> unchanged through the five-backend parity arcs (v0.20–v0.23):
> declarations (`service`, `record`, `table`, `stream`, `use`,
> `blueprint`/`expand`, `import`, `project`), typed and classic
> handlers, the handler-body expression language and builtin
> verbs, capability providers per the support table, `Reference<T>`
> relations and field attributes, `transaction` blocks, and the
> diagnostic code registry through CIAC0059. No deprecations
> outstanding.

The entry does normative work beyond ceremony: it pins *which*
surface froze (by naming the last arc that changed it) and states
the deprecation ledger is empty — the baseline every future entry
diffs against. Future entries are mandatory per the policy: a
LANGUAGE_VERSION bump without a changelog entry is a CI-visible
inconsistency (the language-release workflow's body-generation
step fails if no entry matches the new version — a deliberate
coupling that makes the release machinery enforce the paperwork).

### The compiler-release runbook, as drafted

M9's release exercise, step by step, so the first tag is a
checklist execution rather than an improvisation:

1. Full verification green at the version-bump commit (the
   standing M-final gate).
2. Tag `v0.24.0` on the release branch head; push the tag.
3. Watch `release.yml`: five legs, each uploading its exact asset
   name (`ciac-linux-x86_64`, `ciac-linux-aarch64`,
   `ciac-macos-x86_64`, `ciac-macos-aarch64`,
   `ciac-windows-x86_64.exe` — the names install.sh greps; any
   rename is a breaking change to install.sh and handled as such).
4. On failure: fix forward in the workflow, delete tag, re-tag —
   iteration one is budgeted; iteration two-plus is a recorded
   finding.
5. On success: clean-container proof — `curl | sh` the checked-in
   install.sh against the live release; assert `ciac --version`
   prints `0.24.0 (language 1.0.0)`; run `ciac new` + `ciac check`
   as the smoke of the installed binary (not a full verify — the
   installed artifact's job is compiling programs, prove exactly
   that).
6. Confirm `lang-v1.0.0` exists from M8's workflow (or observe its
   firing now if branch timing deferred it); confirm the language
   release carries the spec asset.
7. Record both release URLs in the M9 Shipped note.

## Pillar 7 — Release automation: the first tags in the repository

### What is true today

`.github/workflows/release.yml` has existed since v0.13: tag-
triggered on `v*.*.*`, five-platform build matrix (linux x86_64/
aarch64, macOS x86_64/aarch64, windows x86_64), publishing via
`softprops/action-gh-release` with generated notes. It has never
run. `git tag -l` is empty — twenty-five plan files, twelve shipped
compiler versions, zero tags, zero releases — and `install.sh`
(also shipped in v0.13, also advertised in the README's Quick
start) downloads from `releases/latest`, a URL that 404s. The
project's front door promises an artifact that has never existed.

### Two release tracks, deliberately distinct

Pillar 6 created two version identities; this pillar gives each its
own release track, because conflating them would immediately
un-teach the distinction the language freeze exists to teach:

**Compiler releases** (`v0.24.0`, `v0.25.0`, ...): the existing
workflow, finally exercised. M9 pushes the `v0.24.0` tag as part of
the version-bump milestone and then — because a workflow that has
never fired has never been debugged — treats the release as a live
proof with acceptance criteria, not a fire-and-forget: all five
matrix legs green, all five assets attached with the exact names
`install.sh` greps for, and `install.sh` run end-to-end on a clean
Linux container against the real published release, exiting with a
working `ciac` on PATH that prints `0.24.0 (language 1.0.0)`. The
plan budgets one tag-delete-retag iteration for workflow debugging
without counting it a deviation; more than one gets recorded as a
finding. Going forward the rule is simple and recorded in docs:
every version-bump milestone of every future arc ends by tagging.

**Language releases** (`lang-v1.0.0`, ...): a new small workflow,
`language-release.yml`, `on: push: branches: [main]` filtered to
`paths: [LANGUAGE_VERSION]`: read the file, and if tag
`lang-v$(cat LANGUAGE_VERSION)` does not already exist, create it
and publish a GitHub release named "CIaC Language v1.0.0" whose
body is generated from docs/language.md's stability section header
plus a link to the spec at that tag, with docs/language.md attached
as the release asset — the spec snapshot, permanently addressable.
The exists-check makes the workflow idempotent (re-runs and
unrelated pushes are no-ops); the paths filter means the workflow
literally cannot fire unless someone edits the one file whose only
purpose is declaring the language version — the tightest possible
coupling between "the DSL changed versions" and "a release marks
it", which is exactly what was decided. `lang-v1.0.0` itself fires
for the first time when M8 merges to the release branch with the
new file, making the first language release and the first compiler
release land in the same arc — the right founding story for the
two-track scheme.

### The workflow, as drafted

`language-release.yml`'s intended shape (final syntax at M8):

```text
name: Language release
on:
  push:
    branches: [main]
    paths: [LANGUAGE_VERSION]
jobs:
  release:
    steps:
      - checkout
      - id: v
        run: echo "version=$(cat LANGUAGE_VERSION | tr -d '[:space:]')" >> "$GITHUB_OUTPUT"
      - name: Create tag + release if absent
        run: |
          TAG="lang-v${{ steps.v.outputs.version }}"
          if git rev-parse -q --verify "refs/tags/$TAG" >/dev/null; then
            echo "$TAG exists; nothing to do"; exit 0
          fi
          git tag "$TAG" && git push origin "$TAG"
      - softprops/action-gh-release (pinned):
          tag: lang-v${{ steps.v.outputs.version }}
          name: "CIaC Language v${{ steps.v.outputs.version }}"
          files: docs/language.md
          body: generated from the spec's changelog entry for this version
```

The properties the sketch pins down: paths-filtered (cannot fire
on unrelated pushes), idempotent (exists-check makes re-runs and
reverts no-ops — a *revert* of LANGUAGE_VERSION deliberately does
not delete a tag; released versions are permanent), and the
release asset is the spec itself at that tag, making every
language version a permanently addressable document. One sharp
edge pre-named: the workflow needs `contents: write` permission
and a checkout with tags fetched; both are M8 checklist lines
because both are the kind of detail that only fails live.

## Pillar 8 — Blast radius, determinism, and what "no new surface" buys

This arc's seven workstreams have wildly different risk profiles,
and the plan's sequencing (Milestones section) is built around the
one genuinely invasive change. An explicit accounting:

- **Generated-code-visible**: Pillar 1 (Rust lowering — the big
  one, bounded by the three invariants and the A/B design gate),
  Pillar 2 (Java: one new gated template + one gated pom entry —
  additive, invisible to every existing example since none declares
  logging), Pillar 3 (test-scoped suites + possibly a test-profile
  config override — production code paths untouched by design,
  asserted by golden diff showing test-directory-only churn on
  existing examples).
- **Repo-only**: Pillars 4, 5, 7 (CI workflows, deny.toml,
  dependabot, docs tables, release machinery — zero generated
  bytes change).
- **Metadata**: Pillar 6 (new fields in describe/targets/manifest
  output — schema-additive, `describe_version` unchanged since the
  addition is backward-compatible; the manifest stamp is additive
  and regeneration-neutral).

The golden suite is the arbiter throughout: milestones are
sequenced so that at most one generated-code-visible workstream is
in flight at a time, every golden diff is reviewed diff-by-diff
(the standing rule: `cargo insta test --accept` output is read,
never blind-accepted), and the equivalence suite plus the two
canonical sim outcomes run at every milestone exit that touched
generation. Determinism inherits the standing rules — exact pins
everywhere including every new test-scoped dependency and every CI
action (dependabot's github-actions ecosystem now watches those
pins), and the new scanners turn the pin set from write-only to
supervised.

### What this arc does not touch in the simulation surface

Stated explicitly because two of this arc's pillars brush against
it: `SimSupport` stays exactly as 25 left it (Python `Full`, four
targets `Narrow`, external/skeleton `None`); the
`unsupported_sim_capabilities` gates change on **no** target (the
auth-refusal branch stays — the OAuth2 *test* rig is a
generated-test concern, not a sim-world capability; FakeAuth
parity is 27's); the scenario schema does not move; the per-target
sim drivers in commands.rs do not move. The only simulation-
adjacent change in the whole arc is inside Rust's transaction
leaf's *real* branch (Pillar 1), with the world branch preserved
byte-for-byte and the two canonical anchors as the tripwire. 27
inherits a sim surface identical to today's except that the
production semantics under it are finally uniform.

### The config/env surface

Changes to what generated systems read from their environment,
enumerated because config surface is API (the standing rule since
the v0.11 provider work — every row lands in the per-target config
docs):

- **OAuth2 issuer override (Pillar 3):** no *new* variable on any
  target — the rig reuses the existing issuer env var each backend
  already reads for Keycloak wiring. If a target's test context
  cannot reach it (Java's standalone MockMvc being the likely
  case), the accommodation is a test-construction override, not an
  env addition; zero new production config rows is itself an M5
  exit assertion.
- **Java logging (Pillar 2):** no env surface. The capability is
  declared in the program, the file is generated or not; log level
  remains `application.yml`'s existing surface.
- **`LANGUAGE_VERSION` (Pillar 6):** not env at all — a repo file
  and a compile-time embed; generated systems see it only as a
  manifest stamp.

### Predicted golden churn

Stated up front so review effort lands where the risk is
(actuals reconciled in the retrospective):

| Milestone | Expected churn |
| --- | --- |
| M1 | Rust goldens for transaction-bearing examples only (design A invariant); **zero** for the rest — the review is precisely the check that the second column stays empty |
| M2 | one example added/extended ×5 targets |
| M3 | Java goldens: pom + new logback file for the logging example only; ×4 other targets for the corpus example |
| M4–M5 | test files only, ×5; production files byte-identical — asserted, not hoped |
| M6–M8 | zero generated bytes; repo/docs/CI only (M8: manifest additivity proven by no-op `ciac diff`) |
| M9 | version-string churn only, the standing M-final shape |

## Implementation map

| Area | Changes |
| --- | --- |
| `crates/ciac-backend-rust/src/lower.rs` | executor helper + depth cell; `transaction_expr` emits real begin/commit; db-verb arms call `executor_expr()` (Pillar 1) |
| `crates/ciac-codegen/src/lower` (shared) | `enter_transaction`/`exit_transaction` default-empty hooks on `HostSyntax`, driver calls them around transaction children — the arc's only shared-lowering amendment |
| `crates/ciac-backend-java/src/lib.rs` | `Logging` added to `supports()`; `logback-spring.xml.j2` emission gated on `has_logging` |
| `crates/ciac-backend-java/templates/` | new `logback-spring.xml.j2`; `pom.xml.j2` gains gated logstash-logback-encoder pin |
| all five backends: scope-test emission | gate widens from jwt-only to both schemes; new OAuth2 rig cases in each scope-test template; five exclusion comments deleted |
| `deny.toml`, `.github/dependabot.yml` | new (Pillar 4) |
| `.github/workflows/ci.yml` | new `workspace-audit` + `generated-audit` jobs + weekly schedule trigger |
| `.github/workflows/language-release.yml` | new (Pillar 7) |
| `LANGUAGE_VERSION` | new file, `1.0.0`; consumed by `ciac-syntax` (`include_str!`) and CI |
| `crates/ciac/src/{describe,main,commands}.rs` | `language_version` in describe/targets/`--version`/manifest |
| `docs/backends.md` | the two ledger tables as section front matter |
| `docs/language.md` | retitle; `## Stability and versioning`; changelog section |
| `docs/errors.md` | CIAC0060–0069 range reserved (deprecation warnings) |
| `docs/simulation.md`, `docs/deployment.md` | atomicity cross-reference sweep; OAuth2 exclusion rewording |
| `docs/targets.json` | ts/go/java capabilities populated; language_version header |
| `tests/` | atomicity equivalence case ×5; ledger_integrity test; targets_cli extension |
| examples | one logging-bearing example (extended or new); the atomicity-proof example/extension |

## Capability parity checklist

Not a backend arc, so the checklist is inverted — instead of "new
target reaches every row", it is "every row this arc touches ends
uniform across all five targets":

- `transaction {}`: atomic in production on **5/5** (was 4/5);
  rollback-on-failure equivalence case green ×5.
- `logging Structured`: accepted and implemented on **5/5** (was
  4/5); corpus exercises it ×5; targets.json says so for all five.
- No-infra scope tests: JWT **and** OAuth2 on **5/5** (was JWT-only
  ×5); textually parallel suites; wrong-key 401 case present ×5.
- Dependency scanning: workspace + **5/5** generated ecosystems.
- targets.json capabilities maps: populated **5/5** (was 2/5).
- Divergence ledger: **every** row classified permanent-with-reason
  or open-with-address; zero unclassified divergences.

## Determinism and supply chain

The standing rules apply unchanged (exact pins, snapshotted
wrappers/lockfiles, golden-visible dependency changes); this arc
adds the enforcement layer that watches them (Pillar 4) and the
release layer that distributes them (Pillar 7). New dependencies
introduced by this arc itself, all exactly pinned: per-target
test-scoped stub-server/JWKS libs (Pillar 3), logstash-logback-
encoder (Pillar 2, gated), cargo-audit/cargo-deny/grype/pip-audit/
govulncheck as CI **tools** (pinned by version in the workflow, not
project dependencies). No new runtime dependency lands in any
generated project except Java's gated logging encoder.

## Diagnostics, gating, and docs impact

No new error codes are *emitted* this arc. One code range is
*reserved*: CIAC0060–0069 for language-deprecation warnings
(Pillar 6's ladder), registered in docs/errors.md with an
explicitly empty table. One code loses a customer: CIAC0011 stops
firing for Java+logging (M3), and the `generated-java` CI loop's
CIAC0011-skip branch consequently stops matching the logging
example — the skip machinery itself stays, since other targets'
genuine unsupported combinations still use it. Gating changes: the
five scope-test emission gates widen (Pillar 3); Java's
`supports()` list grows by one (Pillar 2); no `SimSupport` change
anywhere (that is 27's arc). Docs impact is the heaviest of any
non-backend arc yet — backends.md (ledger front matter), language.md
(retitle + stability policy + changelog), simulation.md +
deployment.md (cross-reference sweeps), errors.md (range
reservation), targets.json (capabilities + header), plus README's
install section becoming true the moment M9's release exists.

## Relationship to the forecast documents

21UpdatePlan.md's forecast tracks assumed the five-backend arc
would end with a maturity decision; the punch-list review that
followed 25's retrospective was that decision in practice, and its
Tier 1/Tier 2 items — minus the two simulation rows — are exactly
this plan's pillars. The traceability, item by item:

| Punch-list item | Disposition |
| --- | --- |
| Rust's non-atomic `transaction {}` | this plan, Pillar 1 / M1–M2 |
| Java's `Component::Logging` | this plan, Pillar 2 / M3 |
| External adversarial pass (scanning scope) | this plan, Pillar 4 / M6 |
| Freeze and version the language | this plan, Pillar 6 / M8 |
| OAuth2 scope-testing, JWT-only everywhere | this plan, Pillar 3 / M4–M5 |
| Permanent-vs-temporary divergence decision | this plan, Pillar 5 / M7 |
| Sim depth gap (4 of 5 targets) | 27UpdatePlan.md |
| Multi-service simulation | 28UpdatePlan.md |
| Onboarding narrative, editor polish, positioning, dogfooding-prep | 29UpdatePlan.md |

The arc consumes the five-backend retrospective's disclosed-gaps
ledger as its work queue and produces the restructured two-table
ledger as its handoff artifact; 27UpdatePlan.md and
28UpdatePlan.md (authored alongside this plan) are the
pre-committed addresses for the two rows this arc deliberately
does not touch. The Tier 3 items are 29UpdatePlan.md's, sequenced
last so the front door describes the system these three arcs
finish rather than the one they started with.

## What this arc is predicted to cost

The factory arcs earned their credibility by predicting costs and
reconciling against actuals; a non-backend arc gets the same
treatment. Predictions, to be reconciled in M9's retrospective:

| Workstream | Predicted size | Predicted risk-adjusted schedule weight |
| --- | --- | --- |
| Rust atomicity (Pillar 1) | ~30 mechanical site edits + 1 shared hook pair + wrapper emission; goldens for every transaction-bearing Rust example | the arc's largest single item; M1+M2 together ≈ a third of the arc |
| Java logging (Pillar 2) | 1 new ~15-line template, 1 pom entry, 1 supports() line, 1 test, 1 example touch | small; the five-target corpus proof is most of the work |
| OAuth2 rig (Pillar 3) | ~5 × (1 stub + ~7 cases), mostly translations of one contract | a quarter of the arc; M4's contract design is the leverage point |
| Scanning (Pillar 4) | 2 CI jobs + 2 config files + 1 script | small-to-medium; first-run triage is the unknown |
| Ledger (Pillar 5) | 2 tables + 1 parser test + targets.json derivation | small |
| Language freeze (Pillar 6) | 1 file + 1 const + 4 surfacings + normative docs | small; the writing is drafted in this plan already |
| Releases (Pillar 7) | 1 new workflow + 1 live-fire exercise | small, tail-risk-shaped |

## Milestones

Nine milestones, sequenced so the invasive work front-loads while
review attention is freshest, the independent workstreams fill the
middle, and the ledger/freeze/release tail consumes the closures
the earlier milestones produce. Every milestone ends with the
standing full verification (fmt/clippy/test/goldens), a commit,
and a push; milestones that touch generation additionally end with
the equivalence suite and both canonical sim anchors. "Shipped"
notes are appended per milestone at execution time, in place,
per the house convention.

1. **M1 — Rust atomicity: the executor seam.** Land Pillar 1's
   design A: the `enter_transaction`/`exit_transaction` default-
   empty hooks on `HostSyntax` with the shared driver invoking them
   around a transaction statement's children (the arc's only
   shared-lowering amendment, conformance-harness-visible,
   no-op for four targets); the depth cell + `executor_expr()`
   helper on the Rust lowering state; the mechanical swap at every
   db-verb arm (completeness enforced by the grep-for-old-literal
   check — the old executor token may appear exactly once, inside
   the helper); `transaction_expr` emitting real
   `begin()`/`commit()` with drop-rollback semantics, engine-generic
   across Postgres/MySQL/SQLite. The A/B decision point executes
   here: if the hook pair cannot reach every db-verb site through
   the driver's recursion, the failure is recorded and design B
   (uniform-connection) activates — either way M1 ends with exactly
   one design landed and the other's rejection reasoned in this
   file. Golden regeneration reviewed under invariant 1: examples
   without `transaction {}` show **zero** diff (design A) —
   transaction-bearing examples show executor + wrapper changes
   only. Equivalence suite green ×5; the two canonical sim outcomes
   re-proven on Rust (invariant 3 — the world-guard and the new
   executor meet at the transaction leaf, so this is where fake≠real
   drift would be born). NOTE marker removed from emission.

   **Shipped (v0.26 M1):** design A landed, but not in the shape this
   milestone predicted — reading the real shared driver
   (`ciac-codegen/src/lower/dispatch.rs`) before writing code changed
   the plan. The "30+ sites" the v0.16 assessment worried about
   turned out not to exist: every `db.*` verb is lowered directly
   from `lower_expr_any`'s own match arms (HIR never nests a verb
   call inside another verb call's arguments — only scalars nest),
   so there was no need for the `enter_transaction`/`exit_transaction`
   hook pair or a `Cell`-based depth counter this file proposed.
   Instead, `in_tx: bool` threads as an ordinary parameter through
   exactly the three functions that already recurse across statement/
   block boundaries — `lower_expr_any`, `lower_block_expr`,
   `lower_stmt_expr` — mirroring the parameter-threading shape
   `Statement`-orientation's own `lower_tail`/`lower_block_stmt`/
   `lower_stmt` have used since the factory arcs, rather than
   inventing a second mechanism. This is a smaller, more consistent
   change than either design A or B as drafted: no new `HostSyntax`
   trait methods, no interior mutability, and the shared driver's
   only diff is the added parameter on three signatures plus
   `HirStmt::Transaction`'s arm now calling `lower_block_expr` twice
   (`false` for the world/simulated branch — rendered exactly as
   before, unchanged — and `true` for the real branch). `RustSyntax`
   gained one private `executor(in_tx: bool) -> &'static str` helper
   (`"&mut *__tx"` vs `"self.db"`) consumed by `db_insert_expr`/
   `db_update_expr`/`db_delete_expr`/`query_expr`; `transaction_expr`
   now takes both pre-lowered branches and wraps them as
   `if self.world.is_some() { <sim, unchanged> } else { let mut __tx
   = self.db.begin().await?; <real, executor-swapped> __tx.commit()
   .await?; }` — deliberately `self.world.is_some()`, not
   `if let Some(world) = ...`, since the latter's binding goes
   unused inside `sim_branch` (which re-derives its own `world`
   locally) and trips `-D warnings`. `IdentitySyntax`
   (`ciac-codegen/src/lower/identity.rs`) updated to the same
   signatures with a `(transaction (sim ..) (real ..))` s-expression
   shape and `-tx`-suffixed verb symbols on the real branch, keeping
   the contract's own golden coverage honest. Golden diff confined
   to exactly the two examples declaring `transaction {}`
   (`domain-orders`, `sim-vertical-slice`) — every other example, on
   every target, byte-identical; reviewed line-by-line, not blind-
   accepted. Both canonical sim anchors re-proven byte-exact on Rust
   post-fix: `{"ProcessOrder":3,"Reconcile":1}` (vertical-slice) and
   `{"ProcessOrder":100,"Reconcile":7}` (virtual-week). NOTE marker
   gone from generated output. Design B was never attempted — A
   worked on the first real read of the driver, so there was nothing
   for B to rescue.

2. **M2 — Rust atomicity: the rollback proof and the reference
   sweep.** The purpose-built atomicity case (extend
   `domain-orders` or add a minimal two-insert-constraint-violation
   example — decided by golden blast radius, recorded): handler
   performs two inserts in one `transaction {}`, second violates a
   constraint, assertion is error envelope **and zero rows** —
   run live against real Postgres and real MariaDB (compose) and
   SQLite (zero-Docker; the fully-local leg). The same scenario
   enters the equivalence suite ×5, upgrading the claim from "Rust
   caught up" to "atomicity is uniformly tested wherever it is
   claimed". Then the debt retirement: docs/language.md's
   transactions caveat deleted; the four "unlike Rust" callouts in
   backends.md/simulation.md deleted; the ledger row (born
   formally in M7, tracked informally from M1) marked CLOSED with
   the proof named. Exit includes a whole-repo grep for
   "non-atomic" — every remaining hit must be about simulation
   no-op degradation (27's territory), none about Rust production.

   **Shipped (v0.26 M2):** live rollback proof against a real local
   Postgres 16 cluster, using `domain-orders.ciac`'s existing
   `PlaceOrder` handler (its own comment already anticipated this
   proof: "on the Rust backend (interim, non-atomic lowering) the
   first write survives... both behaviors are exercised live, not
   assumed" — v0.16-era text this milestone makes true). Two
   independent failure sources proven, not just the one the plan
   named: (1) the handler's own app-level `fail` after a successful
   first insert (negative `total`) — HTTP 500, **zero** rows in
   `orders` for the rejected id; (2) a genuine SQL-level error (a
   foreign-key violation on `customer_id`, no app-level check
   involved) — same result, zero partial rows, `order_audits`'
   count unchanged. A valid control request committed both rows
   normally. No explicit rollback code was needed for either case:
   `sqlx::Transaction`'s own `Drop` issues the rollback when `__tx`
   goes out of scope without `.commit()` having run — exactly the
   drop-rollback semantics this file specified. **Engine coverage,
   disclosed honestly:** Postgres proven live. MariaDB blocked by
   this environment specifically (no working local instance —
   Docker's daemon is unavailable here and the local `mariadbd`'s
   root credentials could not be recovered after reasonable
   troubleshooting; not a code issue) — deferred, not skipped
   silently, and not a gap in the fix itself (the executor helper
   and `transaction_expr` never branch on engine — the emitted Rust
   text is identical across Postgres/MySQL/SQLite, differing only in
   the already-engine-generic pool type behind `self.db`, exactly as
   this file predicted). SQLite likewise not independently live-run
   this milestone — no checked-in example declares both `db SQLite`
   and a `transaction {}` block to run it against; reasoned
   engine-agnostic by the same code-inspection argument, not proven
   live, and named here rather than left implicit. Automated
   regression coverage for the fix is the golden/identity snapshot
   pair from M1 (any future change to the executor/wrapper shape
   shows up as a reviewed diff on the two transaction-bearing
   examples) plus this Shipped note's live-proof record; a dedicated
   `tests/` integration case exercising the rollback against a
   CI-available database is recorded as a gap for whoever next
   touches this file, not fabricated here to look complete. Debt
   retirement: `docs/expressions.md`'s Rust-transactions paragraph
   rewritten to describe the fix (executor swap, drop-rollback, the
   live proof); the "unlike Rust's own disclosed non-atomic gap"/"a
   real improvement over Rust's own disclosed non-atomic gap"
   callouts in `docs/backends.md` (TypeScript, Go paragraphs) and
   `docs/simulation.md` (TypeScript, Go, Java paragraphs) reworded to
   state parity instead of a gap; two stale doc comments in
   `crates/ciac-backend-go/src/lower.rs` and
   `crates/ciac-backend-ts/src/lower.rs` making the same now-false
   claim ("exceeding the Rust backend's own disclosed non-atomic
   gap") corrected to "matching". Whole-repo grep for "non-atomic"
   post-fix returns exactly two hits, both in `docs/backends.md`/
   `docs/simulation.md` describing the *simulation-only* degradation
   every narrow target (Rust included) still has — 27UpdatePlan.md's
   territory, not this one's — confirmed clean of any remaining
   Rust-production claim. **A note on this milestone's own
   verification, in the spirit of the disclosure this arc exists to
   practice:** `cargo test --workspace` intermittently hangs in this
   session's sandbox on an unrelated pre-existing issue — a Java
   example's `google-java-format` subprocess invocation stalling
   under this environment's near-exhausted disk allowance, reproduced
   identically against the *unmodified* pre-M1 codebase and therefore
   not attributable to this fix. Verification instead ran targeted
   (`golden`, `host_syntax_identity`, `typed_handler_equivalence`,
   `typed_handler_rust`, `typed_handler_python`, `docs`), each
   green, plus full `cargo build --workspace`/`clippy -D warnings`/
   `fmt --check`.

3. **M3 — Java `logging Structured`.** `Logging` added to
   `supports()`; `logback-spring.xml.j2` emitted under
   `has_logging` with the LogstashEncoder console appender;
   `pom.xml.j2` gains the exactly-pinned, gated encoder dependency;
   the module-doc table's claim becomes true in place. Corpus: one
   example gains `logging Structured` (extension preferred, new
   `structured-logs.ciac` fallback — decided by golden churn,
   recorded) and builds/verifies on **all five** targets — the
   capability's first end-to-end five-target exercise ever.
   Java-specific `LogShapeTest` (no-infra boot, one log line
   through a generated component's logger, parse as JSON, assert
   key set); the no-logging absence case asserted (no file emitted;
   `NoInfraBootTest` green both shapes). `generated-java`'s
   CIAC0011-skip stops matching — verified in CI output, the
   regression proof that the refusal is gone.

   **Shipped (v0.26 M3):** `Component::Logging` added to
   `JavaBackend::supports()`; `logback-spring.xml.j2` (new template)
   emitted at `src/main/resources/logback-spring.xml` gated on
   `ctx.has_logging`, wiring a single `ConsoleAppender` through
   `net.logstash.logback.encoder.LogstashEncoder` — Spring Boot's
   `LogbackLoggingSystem` auto-discovers this exact filename, so no
   `application.yml` change is needed or made, matching every other
   target's own declared-vs-default shape (structlog `JSONRenderer`
   / `tracing_subscriber` `.json()` / `slog.NewJSONHandler` /
   TypeScript's observability template). `pom.xml.j2` gained a
   `logstash-logback-encoder` version property (`7.4`, pinned the
   same way `jnats`/`aws-sdk`/`exec-maven-plugin` already are, since
   spring-boot-dependencies' own BOM doesn't manage it) and a gated
   dependency. Corpus: **extension preferred** over a new file, per
   this milestone's own pre-registered choice — `traced-checkout.ciac`
   (the v0.15 M3/M4 distributed-tracing flagship, already declaring
   `tracing OpenTelemetry` in both services) gained `logging
   Structured;` alongside it in both `Payments` and `Checkout`, since
   the golden churn from extending an existing two-service example
   was smaller and more legible than standing up a new fixture purely
   to exercise one capability. Built and verified live on **all five**
   targets with `logging Structured` newly declared — the capability's
   first true five-target exercise: Java (`./mvnw -q -B verify`, both
   services, twice — once green, once red, see bug below), Python
   (`py_compile` clean), Rust (`cargo check` clean, ~26s, confirming
   the `tracing-subscriber` `json` feature pulls in cleanly), Go
   (`go build ./...` clean, both services), TypeScript (`npm run
   build` clean, both services). **A real bug found and fixed via
   this live verification, exactly the loop this arc exists to run:**
   the first `logback-spring.xml.j2` draft used `--` as a stylistic
   dash twice inside its XML comment — illegal per the XML spec (a
   comment body may not contain `--`), the same bug class this
   project's `pom.xml.j2` hit earlier in its own history. Caught live
   by `NoInfraBootTest` failing with `org.xml.sax.SAXParseException:
   ... The string "--" is not permitted within comments`, not by
   inspection; fixed by rewording the comment; re-verified green.
   **`LogShapeTest`** (new template, gated the same as the encoder):
   rather than hand-building a synthetic `ILoggingEvent` (the first
   draft's approach), which NPE'd inside `LogstashEncoder.encode`
   because a synthetic event carries no `LoggerContext` of its own —
   found live, the second bug this milestone's verification loop
   caught — the shipped version routes one real SLF4J `logger.info(..)`
   call through a `ch.qos.logback.core.read.ListAppender` to capture a
   properly-contexted `ILoggingEvent`, then runs the *exact*
   `LogstashEncoder` configuration through it directly (no Spring
   context needed — this is a property of the encoder alone), parses
   the encoded bytes as JSON, and asserts `@timestamp`/`message`/
   `logger_name`/`level` are present with the expected values. Proven
   green on both services after the fix. **Absence case asserted
   live:** `examples/ping.ciac` (no `logging` declared) emits no
   `logback-spring.xml` and no `LogShapeTest`, and its own
   `NoInfraBootTest` stays green with Boot's default human-readable
   console format — confirmed by inspecting the generated output and
   re-running `mvnw verify`. Golden/IR/dot snapshots reviewed
   diff-by-diff for all five targets (`traced-checkout`'s only
   affected example): the four already-working targets show exactly
   the expected default-to-JSON switch (Python: `configure_logging()`
   call plus `structlog` config module and dependency pin; Rust: the
   `tracing-subscriber` `json` feature flag and `.json()` layer call;
   TypeScript: the observability config literal; Go: `NewTextHandler`
   → `NewJSONHandler`) and Java shows the two new files plus the pom
   additions — no unexpected diffs anywhere, confirming the four prior
   targets' `has_logging` wiring was already correct before this
   milestone touched only Java. Docs: `docs/language.md`'s capability
   table row and its "every provider generates on all five... except
   `logging`" paragraph both corrected in place (the second one, on
   inspection, cited the wrong closing milestone before this fix —
   corrected while here); `.github/workflows/ci.yml`'s `generated-java`
   job comment, which had claimed "zero gates as of M7" while the
   logging gate was in fact still live through v0.25, corrected to
   name this milestone as what actually closed it. `cargo clippy -p
   ciac-backend-java --all-targets -- -D warnings` clean; targeted
   `golden`/`gating`/`negative`/`docs` suites green post-snapshot-
   accept.

4. **M4 — The OAuth2 rig: design once, land twice.** The rig's
   cross-target contract fixed in prose first (key handling per
   Open question 2; stub route; issuer-override mechanism; the
   four-case matrix including wrong-key→401), then implemented on
   the two reference targets: Python (the `Full`-sim,
   fastest-iteration backend) and Rust (the strictest compiler —
   if the rig's shape survives both, the remaining three are
   translation). Gates widen from `auth_scheme == "jwt"` to both
   schemes on these two; exclusion comments deleted; `oauth-echo`'s
   generated projects carry the suite; suites green under zero
   infrastructure, real RS256, real JWKS fetch against the
   in-process stub. Any per-target config-override accommodation
   (issuer env not reaching test context) recorded.

   **Shipped (v0.26 M4):** the seven-case matrix landed on both
   reference targets, plus a scheme-agnostic pair (`no_token`,
   `malformed_token`) folded into the *existing* JWT suites on both
   targets rather than duplicated — those two cases never touch
   scheme-specific crypto, so testing them once per endpoint (not
   once per scheme) is the correct shape, not a shortcut. **Open
   question 2, decided differently per target and disclosed as
   such:** Python generates a fresh RSA keypair per test session
   (`cryptography`, already a transitive dependency via
   `pyjwt[crypto]`, so generation is genuinely free); Rust embeds a
   fixed 2048-bit test keypair instead, because `jsonwebtoken`'s
   RS256 support is backed by `ring`, which deliberately does not
   expose RSA key generation — adding a keygen dependency purely for
   this rig would have been a heavier cost than embedding one. Both
   choices are recorded here rather than silently diverging.
   **Zero new production dependencies on either target, as
   specified**, and in fact zero new *dependencies at all*: Python's
   JWKS stub is a bare `http.server.HTTPServer` on a background
   thread (no `pytest-httpserver`); Rust's is a two-line `axum`
   router serving one route — the exact same crate `crate::auth::Jwks`
   already depends on to fetch JWKS in production, reused rather than
   duplicated. **Issuer-override accommodation, exactly the kind
   Pillar 3 pre-registered space for:** Python's `get_settings()` is
   `@lru_cache`d and `app.main` reads it at *module import time*, so
   the env override has to land before `app.main` is ever imported —
   solved by doing the override at `conftest.py`'s own module level
   (pytest loads every `conftest.py` before importing any `test_*.py`
   module, so this ordering is guaranteed, not assumed). Rust's
   `cargo test` runs a file's `#[tokio::test]` functions concurrently
   on shared OS threads, and `OAUTH_ISSUER` is a process-global env
   var — a real data race the first draft didn't have (each test
   wants a *different* stub URL) — solved with a `std::sync::Mutex`
   guarding only the synchronous set-env/read-config pair, dropped
   before the async `AppState::new` call, since `Config::from_env()`
   copies the issuer into an owned `String` before the lock releases.
   **A transcription bug found and fixed via live verification, not
   inspection:** the first embedded Rust private key was hand-copied
   incompletely (several PEM lines dropped mid-paste), producing
   `Error(InvalidKeyFormat)` at test run time; fixed by copying the
   exact key bytes programmatically instead of retyping, verified
   byte-for-byte against the source key file. **Harmonization went
   beyond M4's own literal exit bar:** rather than leave Rust's JWT
   suite on its old case names while Python's used the new vocabulary,
   both targets' JWT suites were renamed to the shared vocabulary
   (`no_token`/`malformed_token`/`wrong_scope`/`correct_scope`/
   `expired_token`) in this milestone rather than deferred to M5,
   since leaving one target's JWT suite un-harmonized while its own
   OAuth2 rig used the new names would have been an inconsistency
   inside this milestone's own two targets, not just across the
   five-target arc. Rust's `tower` dev-dependency gate (previously
   `c.auth_scheme == "jwt" and c.scopes and not (has_db or has_queue)`)
   widened to include `oauth2`, a real gap the first generation
   attempt surfaced as an `unresolved import` compile error, not
   found by inspection. **Live proof, all real crypto, all four
   affected examples:** `oauth-echo` (new `scope: "echo:write"` on
   `Echo`, the corpus carrier) 9/9 Python + 5/5 Rust; `dev-identity`
   (pre-existing OAuth2 example with `read_scope`/`write_scope` on a
   `crud` resource) 14/14 Python + 10/10 Rust; `order-system` (JWT,
   harmonization regression check) 24/24 Python + 18/18 Rust;
   `routed-media` (JWT, harmonization regression check) 9/9 Python +
   7/7 Rust — every case green, including live-observed 200 on
   correct-key/correct-scope and live-observed 401 on wrong-key with
   the exact `"invalid or expired token"` production error body,
   confirmed via an ad-hoc script outside the test harness as an
   independent check that the assertions weren't vacuously passing.
   All five targets' production scope-enforcement code and OpenAPI
   `x-ciac-scope` metadata confirmed unaffected/correctly-affected by
   the new `Echo` scope (TypeScript/Go/Java compile clean with no new
   test suite, exactly the expected M5-deferred shape). Golden/IR
   snapshots reviewed diff-by-diff across all touched examples and
   targets — no unexpected diffs. `cargo clippy -p ciac-backend-rust
   --all-targets -- -D warnings` clean; `cargo build --workspace`
   clean; targeted `golden`/`gating`/`negative`/`docs`/
   `host_syntax_identity`/`typed_handler_equivalence` suites green
   post-snapshot-accept.

5. **M5 — The OAuth2 rig: full parity.** TypeScript, Go, Java.
   Textually parallel suites (same case names, same matrix — the
   conformance-style diff reads as idiom only); all five gates
   widened, all five comments gone; `generated-*` example loops
   green ×5 with the new suites executing (verified in CI logs, not
   assumed — a suite that silently fails to be collected is this
   milestone's named trap, checked by asserting test-count deltas
   per target). docs/deployment.md reworded: live-Keycloak `--system`
   path now described as the deeper oracle above a no-infra suite
   that proves the mechanism with real cryptography. Ledger row
   CLOSED at M7's table birth.

   **Shipped (v0.26 M5):** the seven-case matrix (`no_token`,
   `malformed_token`, `wrong_scope`, `correct_scope`, `expired_token`
   folded into each target's existing JWT-scheme suite; `wrong_scope`,
   `correct_scope`, `expired_token`, `wrong_key`, `wrong_issuer` in
   each target's new real-RS256 rig) landed on TypeScript, Go, and
   Java, closing the gap M4 opened on the two reference targets.
   **A wider gap than M4's own scope revealed on inspection, fixed
   before porting:** TypeScript's, Go's, and Java's *existing* JWT
   scope suites only ever carried the 2-case `wrong_scope`/
   `correct_scope` pair — no `no_token`, `malformed_token`, or
   `expired_token` at all, on any of the three, predating this arc
   entirely. Matching "same case names, same matrix" across all five
   targets required bringing these three up to Python's/Rust's own
   5-case JWT bar first, not just bolting an OAuth2-only rig on top
   of a thinner existing suite. **A second gap found only by
   generating and testing a real db-backed oauth2 project (not by
   inspection):** M4 had widened Rust's `scope_tests.rs` gate in
   prose but the actual code still gated the whole file on
   `auth_scheme == "jwt"`, so a pure-oauth2 Rust project (`dev-
   identity`) silently got zero `no_token`/`malformed_token`
   coverage — caught by regenerating `dev-identity` for Rust and
   finding the cases simply weren't there, fixed as this milestone's
   own first step since it directly blocked "same matrix" parity for
   the reference target M5 was about to imitate three more times.
   **A materially different accommodation per target, each disclosed
   at its own gate:**
   - **TypeScript**: `jose` (already a dependency) generates RSA
     keypairs directly via WebCrypto — no new dependency for the rig,
     unlike Rust's embedded-key workaround. The stub is a memoized
     `node:http` server started lazily on first use, not at module
     load, so a project's non-auth tests never pay for it.
   - **Go**: `internal/auth.getJWKS` caches its fetched JWKS behind a
     package-level `sync.Once` — correct for one issuer per production
     process, wrong for a test binary that wants a *different* stub
     issuer than a previous test file might have set. `TestMain`
     starts one stub for the whole `routes` package and sets
     `OAUTH_ISSUER` before any test runs, closing the gap a second
     per-test stub would have hit silently (whichever `OAUTH_ISSUER`
     the `sync.Once` latched onto first, every other stub's JWKS would
     have gone uncontacted). The JWKS server and RSA keypair are both
     stdlib (`net/http/httptest`, `crypto/rsa`) — zero new
     dependencies, reusing `github.com/golang-jwt/jwt/v5` (already a
     dependency) for RS256 signing.
   - **Java**: the deepest accommodation of the three, found live, not
     by inspection. `SecurityConfig.jwtDecoder()` built the JWKS URI
     by interpolating `{{ c.auth_issuer }}` as a Java string literal
     at codegen time — no runtime override existed at all, so a stub
     pointed at any other URL was structurally unreachable. Rewired to
     `@Value("${OAUTH_ISSUER:<declared-default>}")`, matching every
     other target's own env-driven config, with `@DynamicPropertySource`
     (Spring's own purpose-built "set a property before context
     startup" mechanism, normally used to point tests at a
     Testcontainers-allocated port) supplying the stub's dynamically-
     allocated URL. Wiring the override surfaced a second, real
     production gap along the way: `NimbusJwtDecoder.withJwkSetUri(...)
     .build()` installs only `JwtValidators.createDefault()` (`exp`/
     `nbf` timestamp checks) — it does **not** validate `iss` or `aud`
     on its own, unlike the `JwtDecoders.fromIssuerLocation` factory
     every Spring Security example reaches for instead. A token from
     any issuer, correctly signed by whatever key the *configured*
     JWKS URI happened to serve, would have been accepted regardless
     of its own `iss` claim — the `wrong_issuer` rig case would have
     silently passed as a false negative (200, not 401) had this not
     been caught by writing the actual test rather than trusting the
     existing code read correctly. Closed with an explicit
     `JwtIssuerValidator` plus (when an audience is declared) a
     `JwtClaimValidator` on `aud`, composed via
     `DelegatingOAuth2TokenValidator` — the same issuer/audience
     enforcement every other target's own decoder already had. The
     JWKS stub itself is `com.sun.net.httpserver.HttpServer` (JDK
     stdlib) plus `com.nimbusds.jose.jwk.RSAKey`/`JWKSet` (already a
     dependency, the same library `SecurityConfig`'s own decoder
     builds on) — zero new dependencies, same bar as the other four
     targets.
   **A pre-existing, orthogonal, and disclosed-not-fixed
   characteristic found live:** Java's `dev-identity` and
   `order-system` "accepts"-shaped cases (correct scope, auth clears)
   that reach a `db`-backed resource take ~30s each against this
   environment's unreachable Postgres — HikariCP's default
   `connectionTimeout`, versus the other four targets' respective
   drivers, which fail fast (`ECONNREFUSED` in milliseconds, not a
   30s pool-acquisition wait). Correctness is unaffected (the
   "not 401, not 403" assertion still holds against the eventual 5xx,
   the exact claim boundary every target's own scope-test file header
   already documents), and the same slowness would already have hit
   any hypothetical JWT+db Java example's own `AcceptsRequiredScope`
   case before this milestone — not something M5 introduced, and
   tuning `HikariCP`'s timeout is a distinct concern from OAuth2 rig
   parity, left untouched. **Live proof, all real crypto, all three
   targets, all four affected examples:** TypeScript 8+13+19,
   Go 7+12+(order-system green), Java 7+12+18 — every case green,
   `tsc --noEmit`/`eslint .` clean (TS), `gofmt -l .`/`go vet ./...`
   clean (Go), `mvn compile`/`test-compile` clean (Java). Only
   pre-existing, unrelated finding left disclosed rather than fixed:
   TypeScript's `dev-identity` `sim_runner.ts.j2` carries three
   unrelated `no-unused-vars` ESLint errors (dead code for a
   db-only-no-queue project, predating this arc, outside this
   milestone's own scope). docs/deployment.md's live-Keycloak
   paragraph reworded per this milestone's own exit bar.

6. **M6 — Supply-chain scanning.** `deny.toml` (advisories/
   licenses/bans/sources, ignore-requires-reason-and-expiry checked
   by script); `workspace-audit` job (cargo audit + cargo deny) and
   `generated-audit` job (five ecosystems per Pillar 4's table,
   representative examples, high+ fails) added to ci.yml alongside
   a weekly `schedule:` trigger; `.github/dependabot.yml` (cargo,
   github-actions, npm@editors/vscode, weekly, grouped). The Java
   scanner decision (grype vs dependency-check) executed and
   recorded per Open question 3. Acceptance is a live negative
   test: temporarily pin a known-advisoried version in a scratch
   branch and watch the job fail (the scanner proven able to fail,
   not just able to pass), plus the whole matrix green on the real
   tree — any *actual* findings surfaced by first-run scans are
   triaged in-milestone: fixed by pin-bump if fixable, entered with
   reason+expiry if not, and either way recorded in the Shipped
   note as this milestone's most interesting output.

7. **M7 — The two-table ledger and truthful targets.json.** The
   Permanent-by-design and Open-(tracked) tables land as
   backends.md front matter with the initial rows from Pillar 5,
   every permanent row carrying its reason, every open row its
   address; the surrounding prose re-linked from the tables; rows
   already closed by M2/M3/M5 enter as CLOSED-with-proof (the
   table records closures, it does not pretend they were never
   open). `ledger_integrity` test (closes-in references must name
   existing plan files; no row in both tables). targets.json:
   capabilities maps derived and populated for typescript/go/java,
   enforced by the existing checked-in-matches-derived test —
   including Java's new `logging` row, making M3 machine-visible.
   docs/simulation.md's status table cross-links the Open table
   rather than restating it.

8. **M8 — Language v1.0.0.** `LANGUAGE_VERSION` file (`1.0.0`);
   `ciac_syntax::LANGUAGE_VERSION` via `include_str!`; surfaced in
   `describe` (beside `describe_version`), `targets --json` header
   (checked-in copy re-enforced), `--version` output
   (`ciac 0.24.0 (language 1.0.0)` — compiler number lands fully in
   M9; the format lands here), and the generated manifest stamp
   (additive, regeneration-neutral — proven by a no-op `ciac diff`
   on an existing project). docs/language.md: retitled, `##
   Stability and versioning` normative section (covered surface /
   breaking-additive-editorial / the deprecation ladder / the
   compiler-language support contract), changelog section seeded
   with the v1.0.0 entry. docs/errors.md reserves CIAC0060–0069,
   table explicitly empty. `language-release.yml` lands (paths-
   filtered to LANGUAGE_VERSION, idempotent exists-check) — it
   fires for real when this merges to the release branch, producing
   tag `lang-v1.0.0` and the first language release with the spec
   attached; firing verified as part of M9's release work if branch
   timing defers it, and whichever milestone observes the firing
   records it.

9. **M9 — Version 0.24.0, the first compiler release, and the arc
   retrospective.** Version: **0.23.0 → 0.24.0** (`Cargo.toml`
   workspace + the eleven internal pins, `editors/vscode/
   package.json`, docs/language.md's compiler parenthetical — note
   the title's language version does NOT move; the first arc where
   the two-version discipline is exercised by a bump that touches
   one and not the other). Full verification (fmt/clippy/test/
   goldens/equivalence/sim anchors ×5). Then the release proof per
   Pillar 7: tag `v0.24.0`, five matrix legs green, five assets
   named exactly as install.sh expects, install.sh end-to-end on a
   clean container printing `0.24.0 (language 1.0.0)`; one
   tag-delete-retag iteration budgeted, further iterations recorded
   as findings; `lang-v1.0.0` confirmed existing (or fired here).
   README's install section verified true against the real release.
   The retrospective (appended here, 24/25-style, after a rule):
   the arc's cost accounting (which pillars were estimated
   correctly, what the A/B decision actually chose and why, what
   the first-run scanners actually found), the ledger's before/
   after row counts (the arc's thesis stated numerically: N open
   rows became M closed + K permanent + 2 addressed-to-27/28), and
   the handoff to 27UpdatePlan.md.

### Per-milestone exit checklists

- **M1 exits when:** exactly one of design A/B is landed with the
  other's rejection recorded; the hook amendment is default-empty
  and adopted only by Rust (conformance harness confirms);
  grep-for-old-executor-literal returns only the helper;
  zero golden diff on transaction-free examples; equivalence ×5
  green; both sim anchors byte-exact on Rust; NOTE marker gone.
- **M2 exits when:** rollback proof green on Postgres + MariaDB +
  SQLite live; the atomicity case is in the equivalence suite and
  green ×5; language.md caveat and all four "unlike Rust" callouts
  deleted; whole-repo "non-atomic" grep clean of Rust-production
  hits.
- **M3 exits when:** the logging example builds/verifies ×5;
  LogShapeTest green; absence case green; CIAC0011-skip no longer
  matches in `generated-java` (CI-log-verified); module-doc table
  true; encoder pin golden-visible.
- **M4 exits when:** rig contract prose committed; Python + Rust
  suites green no-infra with real RS256 + wrong-key 401 case;
  gates widened and comments deleted on both; accommodations (if
  any) recorded.
- **M5 exits when:** all five suites textually parallel and green;
  per-target test-count deltas confirm collection; deployment.md
  reworded; zero remaining `auth_scheme == "jwt"` scope-test gates
  in any backend.
- **M6 exits when:** both audit jobs + weekly schedule live;
  deny.toml with expiry-checked ignores; dependabot config live;
  the deliberate-failure negative test demonstrated; first-run
  findings triaged and recorded; Java scanner choice recorded.
- **M7 exits when:** both tables live with every row
  reason-or-address complete; ledger_integrity green; targets.json
  capabilities populated ×5 and test-enforced; simulation.md
  cross-linked.
- **M8 exits when:** LANGUAGE_VERSION exists and is surfaced in
  describe/targets/--version/manifest; stability policy + changelog
  live; CIAC0060–0069 reserved; language-release.yml landed
  (firing verified here or in M9, recorded either way); no-op
  `ciac diff` proves manifest additivity.
- **M9 exits when:** 0.24.0 everywhere it belongs and nowhere it
  does not (language stays 1.0.0); full verification green;
  `v0.24.0` release published with five correct assets; install.sh
  proven on a clean container; `lang-v1.0.0` exists; README install
  section true; retrospective appended with the ledger's numeric
  before/after.

## Open questions resolved at implementation (pre-registered)

1. **Atomicity design A vs B** — A (context hooks + depth cell)
   attempted first; B (uniform connection) only on named failure of
   A to reach every db-verb site; the loser's rejection reasoned in
   M1's Shipped note. Not decidable from the armchair because it
   turns on the shared driver's exact recursion shape at the
   transaction leaf.
2. **OAuth2 rig key handling** — embedded fixed test keypair
   (deterministic, no per-run keygen, risk: misread as production
   material — mitigated by naming and a comment) vs per-run
   generation (slower, unambiguous). Decided once in M4, applied
   identically five times.
3. **Java generated-tree scanner** — grype (default candidate:
   fast, CI-ergonomic) vs OWASP dependency-check (fallback if
   grype's Maven resolution disappoints). Decided in M6 on observed
   behavior against the real generated pom.
4. **Atomicity proof carrier** — extend `domain-orders` vs a new
   minimal example; decided in M2 by golden blast radius.
5. **Logging corpus carrier** — extend an observability example vs
   new `structured-logs.ciac`; decided in M3 by the same criterion.
6. **`--version` output shape** — the proposed
   `ciac 0.24.0 (language 1.0.0)` vs two lines; decided in M8
   against clap's version-flag plumbing realities; recorded.

## Verification strategy

Standard per-milestone discipline: fmt/clippy/test workspace green;
goldens reviewed diff-by-diff, never blind-accepted; live proofs as
named with Docker-delegation honesty (the SQLite atomicity leg and
every scope-test suite run fully local; Postgres/MariaDB legs and
the system rows are compose-backed). Arc-specific standing checks:
the equivalence suite and both canonical sim outcomes run at every
milestone exit that touched generation (M1–M5); the ledger
integrity test from M7 onward; the scanners themselves from M6
onward (this arc eats its own dog food — its later milestones run
under the scanning it introduced).

The proof ledger by layer:

| Claim | Oracle |
| --- | --- |
| Rust `transaction {}` is atomic | injected-failure rollback proof: zero partial rows, live on Postgres/MariaDB/SQLite |
| atomicity uniform across targets | the same case in the equivalence suite ×5 |
| nothing else changed (design A) | zero golden diff on transaction-free examples; equivalence ×5; sim anchors byte-exact |
| Java logging real, not claimed | logging example verifies ×5; LogShapeTest JSON assertion; CIAC0011-skip no longer matches |
| OAuth2 scope enforcement proven no-infra | five suites: real RS256 + JWKS stub; wrong-key→401; zero infrastructure |
| suites actually run | per-target test-count deltas in CI logs |
| dependencies scanned, findings gate | deliberate-failure negative test; high+ fails CI; weekly schedule |
| ledger cannot rot | ledger_integrity: reason-or-address per row, closes-in files exist |
| targets.json is the truth | checked-in-matches-derived test, capabilities ×5 |
| language version is real | describe/targets/--version/manifest carry 1.0.0; lang-v1.0.0 release exists |
| releases work | v0.24.0 assets ×5; install.sh green on clean container |

## Milestone dependencies and parallelism

M1→M2 strictly sequential (the seam, then the proof built on it).
M3 independent of M1/M2 entirely (different backend, different
crates) and may run parallel to them. M4→M5 sequential; M4 may
start any time (touches test emission only — no interaction with
M1's lowering). M6 independent of everything and may land first if
convenient — with the note that landing it early means M1–M5
execute under scanning, which is desirable, so the *suggested*
order pulls M6 forward if the atomicity work stalls. M7 wants
M2/M3/M5 done (its tables record their closures) — hard dependency.
M8 after M7 (the stability section links the ledger). M9 last,
strictly. Maximum useful parallelism: {M1–M2} ∥ {M3} ∥ {M4–M5} ∥
{M6}, then M7→M8→M9 as the sequential tail.

## Explicit cuts

No simulation-depth work and no multi-service simulation (27 and
28's arcs — the two open rows this arc addresses only by giving
them addresses). No fuzzing, load testing, or human security audit
(scanning was the decision; the ledger row for "no external
adversarial pass" stays open with that stated scope). No sim
record/replay. No language-surface changes of any kind — v1.0.0
freezes the surface this arc verifiably did not touch. No
multi-language-version compiler (pragma selection is out of scope
for v1). No JPA/Hibernate revisiting for Java logging (the encoder
rides the existing Logback stack). No brew/scoop/registry
publishing beyond the GitHub release (install.sh remains the
mechanism). No CHANGELOG.md retrofit for pre-1.0 history (the
language changelog starts at v1.0.0; compiler release notes are
generated by the release workflow going forward).

## Risks

- **The atomicity fix destabilizes the most-deployed lowering in
  the repo.** The three invariants each carry an oracle (zero-diff
  goldens, equivalence ×5, byte-exact sim anchors); the A/B gate
  means a failing primary design has a pre-agreed, less invasive
  fallback; and the ultimate fallback — recorded, shameless — is
  that the arc ships without the fix and the ledger row stays
  open with the attempt documented, which is strictly more honest
  than today's state and costs the arc one pillar, not its thesis.
- **The `HostSyntax` hook amendment leaks target-specific
  semantics into shared code.** Default-empty methods adopted by
  exactly one target, conformance-harness-visible; if a second
  target ever needs them the abstraction earns its place, and if
  the amendment grows conditions the factory review rejects it —
  same discipline that governed 24's error-idiom amendment.
- **The JWKS stub diverges from real-issuer behavior and the suite
  proves the wrong thing.** The stub serves static, spec-shaped
  JWKS; the wrong-key case proves verification actually gates; and
  the live-Keycloak `--system` path remains in place above it —
  the rig narrows the untested surface to issuer *quirks*, which
  is exactly what a deeper oracle is for.
- **First-run scanners bury the arc in pre-existing findings.**
  Triage is in-milestone by design (fix-or-ignore-with-expiry),
  the policy fails only on high+, and the finding list is the
  Shipped note's centerpiece rather than a surprise — the arc
  budgets for the archaeology.
- **The release workflow fails in ways only a real tag reveals.**
  One delete-retag iteration pre-budgeted; the matrix is five
  years-old, well-trodden action patterns; and nothing downstream
  depends on release timing except README truthfulness, which is
  M9-gated anyway.
- **Two version numbers confuse more than they clarify.** The
  mitigation is relentless co-presentation (`0.24.0 (language
  1.0.0)` everywhere both appear) plus the stability doc's explicit
  contract table; the risk retires as arcs accumulate in which the
  compiler moves and the language does not — starting with this
  one.
- **Dependabot noise drowns signal.** Weekly cadence, grouped
  minor/patch updates, and the golden discipline as the filter: a
  bump PR that changes generated output is *interesting* by
  definition and reviewed as such; one that does not is mergeable
  on green. If the noise still wins, the recorded fallback is
  monthly cadence — a config line, not a redesign.
- **The drafted ledger tables drift from the surrounding prose
  they index.** The ledger_integrity test catches structural rot
  (dangling addresses, double-classification) but not semantic
  drift; the mitigation is the M7 rule that a row may only
  summarize a prose paragraph that exists, plus the standing
  review habit that every future arc's docs milestone touches the
  ledger before the prose. Accepted residual risk, recorded.

## Confidence and handoff

High on Pillars 2–7: additive, scoped, oracle-backed, and none of
them novel machinery — every one reuses a discipline (golden
visibility, conformance parity, checked-in-matches-derived,
tag-triggered workflows) this repo already trusts. Medium on
Pillar 1, held honestly: it is the one workstream that edits
load-bearing generated code, it carries the arc's only shared-crate
amendment, and its fallback ladder (design B, then
documented-attempt-without-fix) is written down precisely because
"medium" means a real chance the primary path bends. The arc's
thesis does not bend with it: converting every disclosure into a
closure or a decision is achieved by the ledger restructure plus
whichever closures land, and the plan's worst honest outcome is a
ledger with one more open row than intended — carrying, for the
first time, a documented attempt instead of a deferral.

Handoff: 27UpdatePlan.md (Simulation Depth) executes next, against
a Rust backend whose transactions are finally atomic (its
transaction fake now degrades from a true property, not toward
one), under CI that scans what it generates, onto a ledger with an
Open table it exists to empty — followed by 28UpdatePlan.md
(Multi-Service Simulation) and 29UpdatePlan.md (The Front Door),
which inherit, respectively, the deepened worlds and a system
whose remaining gaps are all decisions.
