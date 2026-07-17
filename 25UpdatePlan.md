# CIaC v0.25-file — The Java Backend: Spring Boot at Absolute Parity (implementation plan)

> Implementation plan. Document number ≠ release number (standing
> precedent; version assigned at execution). Assumes 22UpdatePlan.md
> (the factory) shipped and BOTH prior language checkpoints passed
> (23UpdatePlan.md M5, 24UpdatePlan.md M5) — Java goes last on
> purpose: it has the heaviest ecosystem, the slowest build/validate
> loop, and the largest expected template surface of the three, so it
> inherits the twice-hardened `HostSyntax` contract and the
> twice-validated cost model rather than discovering their gaps
> itself. This plan begins, like 24's M1, by reconciling its
> estimates against the two prior arcs' measured actuals.
>
> **Parity contract:** identical to plans 23/24 — every
> capability/provider row of docs/language.md's table; typed handlers
> (full HIR lowering); typed CRUD + keyed-document store; relations;
> REAL transactions (matching Python, exceeding Rust's disclosed
> non-atomic gap, standing cross-reference maintained); the shared
> migration pipeline; scopes + the no-live-infra scope-test suite
> (JWT-only, standing OAuth2 exclusion, same stated reason); OpenAPI
> embedding; observability with broker traceparent propagation; typed
> call clients; realtime channels; generated system tests;
> compose/k8s/Terraform/CI emission; AGENTS.md + ownership manifest
> discipline; `ciac verify` validators; the `ciac dev` loop;
> vocab/LSP/describe/MCP visibility; evolution/rename-replay
> participation; and the narrow simulation slice as the gated final
> milestone.
>
> **Confidence:** high on capability coverage — Spring's ecosystem
> has an official, dominant answer for literally every CIaC
> capability, making Pillar 1's selection table the least contested
> of the three language plans. Medium on two named engineering risks
> unique to Java, each carrying a decided mitigation rather than a
> hope: build hermeticity/latency (Maven downloads the world;
> Pillar 8) and framework-magic versus compiler-ownership tension
> (Spring auto-configuration does things; Pillar 4). A third,
> smaller named risk — Jackson/records/nullability friction — has a
> decided treatment in Pillar 2.

## The gap this version closes

Java is where the enterprises are. It remains the most-deployed
backend language in precisely the organizations whose systems look
like CIaC programs — fleets of services, queues, relational stores,
scheduled jobs, SSO-fronted APIs — and the absence of a Java target
is the absence of CIaC's largest addressable production estate. An
architecture team evaluating CIaC today must either adopt a new
language alongside the new compiler (two migrations) or pass; this
plan removes the forced coupling.

It is also the credibility test of the backend factory. Spring is
the most opinionated ecosystem on this list — it has its own ideas
about configuration, wiring, migrations, and lifecycle. If the
factory's contract (shared model, `TargetInfo`, `HostSyntax` leaves,
emission plan, conformance harness) absorbs Spring without special
cases leaking into shared code, the "new backends are mostly
templates" claim is proven for good; if Spring forces shared-code
special cases, better to learn it on the last planned backend than
the first. Either way the answer lands in M5's checkpoint report.

And it closes the arc: M9's retrospective delivers the consolidated
five-backend cost model — the final, measured answer to the question
that started 22UpdatePlan.md — plus the generated support matrix and
the cross-target disclosed-gaps ledger, as the input to whichever
v0.19–v0.21 forecast track executes next. The natural successor is
named in Confidence below.

## Pillar 1 — Ecosystem selection

Same criteria and rejection-recording discipline as plans 23/24. The
headline decision first, because everything hangs off it:

**Framework: Spring Boot 3.x on Java 21 (LTS), Spring MVC with
virtual threads enabled.** Quarkus and Micronaut are excellent,
faster-booting, AOT-friendly frameworks — and are rejected precisely
because "most widely accepted, utilized, and well-regarded" is not a
close call in Java: Spring Boot is the overwhelming production
default, and a Java developer opening a generated project must
recognize it instantly. WebFlux/reactive MVC is rejected for
generated business logic on DX grounds: virtual threads (Loom,
standard since 21) let generated handlers be plain blocking
imperative Java — matching the readability bar every other backend
holds — while still scaling I/O-bound workloads; Reactor types
leaking into seeded user stubs (`Mono<Order>` in a file the user
owns and edits) would be the single worst developer-experience
decision available to this plan. `spring.threads.virtual.enabled=
true` is emitted in application.yml with a comment saying exactly
this.

A governance note is nearly unnecessary here — the inverse of the
TS plan's situation: every row below except none is either a Spring
project, an official vendor SDK, an official foundation project, or
the Boot default, which is precisely the ecosystem-coherence
argument for Spring in the first place. The cost of that coherence
is Pillar 4 (the framework has opinions, and the compiler must fence
them); the benefit is a selection table with essentially no
maintenance-risk rows and no named fallbacks needed.

| Concern | Choice | Rejected alternatives, with reasons |
| --- | --- | --- |
| Language/runtime | Java 21 LTS: records, switch expressions, virtual threads | Java 17: previous LTS, loses records-era ergonomics this plan's generated code leans on; Kotlin: a different-language decision (a sixth backend candidate someday), not a Java-backend decision |
| Build | **Maven** + committed wrapper (`mvnw`, `.mvn/`) | Gradle: powerful and widely used, but a worse fit for *generated* builds — a Maven pom is declarative, diffable data (golden-friendly); a Gradle build is a program. Decision recorded |
| Web | **spring-boot-starter-web** (MVC) + virtual threads | WebFlux (rejected above); plain servlet without Boot: abandons the ecosystem coherence that motivated Spring at all |
| JSON | **Jackson** (Boot default) + JavaTimeModule, `SnakeCaseStrategy` | — |
| Validation | **jakarta.validation** / hibernate-validator (Boot default) + generated presence checks where bean validation can't distinguish absent-vs-null | — |
| Database | **spring-boot-starter-jdbc → `JdbcClient`** + PostgreSQL JDBC, MySQL Connector/J, **xerial sqlite-jdbc** | Spring Data JPA/Hibernate: the most-used Java persistence stack, rejected because an entity-managed ORM owns schema, dirty-checking, and SQL generation — all three owned by CIaC; Spring Data JDBC: closer but still repository-abstraction-shaped; jOOQ: a SQL compiler of its own (same reasoning as sqlc's rejection in the Go plan); MyBatis: XML mapper indirection generated code doesn't need. `JdbcClient` (Spring 6.1+) is the modern thin standard that executes exactly the SQL the shared model already emits. sqlite-jdbc note: JDBC's sqlite story is weaker than other languages'; xerial is the standard; its native library is bundled (not cgo-analogous build pain) — acceptable, recorded |
| Connection pool | **HikariCP** (Boot default) | — |
| Migrations | **Flyway** (spring-boot-starter-flyway) running CIaC's SQL renamed `V000N__slug.sql` | A generated runner (the TS/Go choice): viable and considered, but Flyway is THE Java migration standard and consumes plain sequential SQL — precisely CIaC's artifact shape; fighting the ecosystem's strongest default to ship a homegrown runner would itself be a credibility cost. Configured pinned: `validate-on-migrate` on, `out-of-order` off, no repair — CIaC's differ remains the only author, Flyway is only the executor. The `TargetInfo.migration_filename` hook (built for exactly this in 22UpdatePlan.md M1) makes the naming a mapping, not a system. Liquibase: second standard, XML-first, no advantage here |
| Cache | **spring-data-redis + Lettuce** (Boot default) | Jedis: the older standard; Lettuce is the current Boot default, netty-based, sync facade over async |
| Queue: NATS | **io.nats:jnats** (official) | — |
| Queue: Kafka | **spring-kafka** | raw kafka-clients: spring-kafka is the accepted idiom, and its listener-container concurrency maps directly onto worker `concurrency` |
| Auth | **spring-boot-starter-oauth2-resource-server** (Nimbus JOSE under the hood) for BOTH providers | jjwt / auth0 java-jwt for manual JWT: viable, but the resource-server starter covers static-secret HS256 (JWT provider) and issuer/JWKS RS256 (OAuth2 provider) in one supported mechanism with built-in lazily-fetched, cached JWKS — the fourth backend in a row where the v0.17 M11 laziness bar is met by dependency choice |
| Object store | **software.amazon.awssdk:s3** (v2, official) | MinIO client: endpoint override + path-style on the official SDK covers compose |
| Email | **spring-boot-starter-mail** (Jakarta Mail) | — |
| Search | **opensearch-java** (official) | — |
| External HTTP | **Spring `RestClient`** (6.1+, synchronous) | RestTemplate: maintenance-mode legacy; WebClient: drags Reactor into the classpath and the stubs; JDK HttpClient: fine, but RestClient is the framework-native modern standard with the better generated-code ergonomics |
| Logging | **SLF4J + Logback** (Boot default) + **logstash-logback-encoder** for JSON | log4j2: capable, not the Boot default; one logging story per target |
| Metrics | **Micrometer** + prometheus registry (Boot standard), `/actuator/prometheus` | — |
| Tracing | **opentelemetry-spring-boot-starter** (official OTel) | Micrometer Tracing + OTel bridge: workable, but the OTel starter keeps env conventions (`OTEL_EXPORTER_OTLP_ENDPOINT` etc.) identical to the other four targets — one tracing configuration story across the fleet |
| Realtime | **spring-websocket** (raw `TextWebSocketHandler`) + **`SseEmitter`** | STOMP: a protocol atop WebSocket — breaks cross-target channel parity exactly as socket.io would have for TS |
| Scheduler | **Spring `@Scheduled(cron=…)`** | Quartz: a persistence-backed scheduling engine CIaC's in-process jobs don't need |
| Testing | **JUnit 5 + spring-boot-starter-test + MockMvc** | — |
| Lint/format | Maven compiler warnings + **Spotless** (google-java-format) as the format validator | Checkstyle/PMD: config surface generated projects don't need; Spotless gives the gofmt-analog "generated code is canonically formatted, asserted" property |
| Docker | `maven:3.9-eclipse-temurin-21` build stage → **`eclipse-temurin:21-jre`** | GraalVM native-image / jlink custom runtime: real size/startup wins, real complexity and per-library metadata risk — recorded as the deployment-maturity follow-up, out of scope |

`TargetInfo` values:

- `project_marker`: `pom.xml`
- `migrations_dir`: `src/main/resources/db/migration`;
  `migration_filename`: `0001_slug.sql → V0001__slug.sql` — the
  first non-identity consumer of the factory's mapping hook, which
  the rename-replay machinery resolves through (M2 proves it)
- `validate`: `./mvnw -q -B verify` (compile + Spotless check +
  tests in one invocation — one JVM/plugin startup, per Pillar 8)
- `compose`: `db_url_scheme: "jdbc:postgresql"` — with the JDBC
  containment note below; `workers_command:
  ["java","-jar","/app/app.jar","--spring.profiles.active=workers"]`
- `dev`: `./mvnw -q -B -DskipTests package` + process restart
- `ci_test_steps`: setup-java@v4 (temurin 21, maven cache) +
  `./mvnw -q -B verify`
- `sim`: `None { reason }` until M9, then `Narrow`

**The JDBC URL containment note.** JDBC URLs
(`jdbc:postgresql://host/db`) carry no credentials — user/password
are separate datasource properties. Same containment move as Go's
DSN note: the generated `application.yml` + `@ConfigurationProperties`
consume the discrete env vars the compose layer already emits and
assemble `spring.datasource.url/username/password`; compose
templates untouched; validated by the existing system tests'
connection round-trips.

## Pillar 2 — Type system mapping and Java-specific semantics

| CIaC | Java | Wire (JSON) | Notes |
| --- | --- | --- | --- |
| `Str` | `String` | string | |
| `Int` | `long` | number | exact i64 parity |
| `Float` | `double` | number | |
| `Bool` | `boolean` | boolean | |
| `Uuid` | `String` | string | `java.util.UUID.randomUUID().toString()` for the builtin only; TEXT storage like every target |
| `Timestamp` | `java.time.Instant` | ISO 8601 | JavaTimeModule + `WRITE_DATES_AS_TIMESTAMPS=false` |
| `Json` | `com.fasterxml.jackson.databind.JsonNode` | any | indexing lowers to `.path(…)` chains + a generated missing-path check that throws the shared error shape — `path()` is null-safe by design, the check restores the KeyError-parity behavior TS/Go also enforce |
| `enum { A, B }` | Java `enum` + `@JsonValue` snake string + `fromString` | string | the generated `fromString` mirrors Rust's generated `from_str` |
| `Record` | **Java `record`** + compact-constructor validation | object | records give immutability/equals/hashCode free; Jackson handles records natively on 21 |
| `Option<T>` | nullable component + `@Nullable` annotation | null | decided below |
| `List<T>` | `java.util.List<T>` | array | never-null: compact constructors normalize null→`List.of()`, matching the Go plan's `[]` decision; wire parity asserted by the shared boundary cases |
| error records | `class XException extends RuntimeException` with record-like fields | — | `@RestControllerAdvice` maps to the same status/shape envelope |

**The nullability decision.** `Optional<T>` as a record component is
non-idiomatic (Jackson friction, serialization ambiguity, the
long-standing "Optional is a return type" guidance). Decided:
`Option<T>` lowers to a nullable component annotated `@Nullable`
(JSpecify annotations, the current standard), with required-field
presence enforced the same way Go does it — deserialization followed
by a generated presence check against the raw key set, because bean
validation's `@NotNull` cannot distinguish explicit-null from absent
for the 400-semantics parity the wire contract requires. The
conformance harness's boundary-decode suite (built for Go's
zero-value trap) runs against Java verbatim — shared paranoia, zero
new test code.

**`HostSyntax` leaves.** Java runs `StatementOriented` (third
consumer of the mode Python validated and Go re-validated) with one
pleasant exception: Java 21 `switch` *expressions* (with `yield`)
give Rust-like expression-position `match` — used where the lowered
code reads naturally, plain statements elsewhere; the per-construct
mode choice is fixed in M4 and golden-visible, not left to drift.
String concat via `String.format`/`+` per operand types; float
fidelity via the shared rule; i64 division native. No clone
discipline (the value-semantics hook is a documented no-op — records
are immutable, construction is construction). **No contract
amendment expected:** unchecked exceptions throughout (generated
error records extend `RuntimeException`) mean no throws-clause
threading, so Go's error-idiom amendment already covers the hardest
tail-shaping case and Java adds none — decided now, in writing,
precisely so M4 is transcription; if implementation falsifies this,
the amendment lands goldens-first by the standing procedure.

## Pillar 3 — Project shape

```text
pom.xml  mvnw  .mvn/  Dockerfile  README.md  AGENTS.md  openapi.json
docker-compose.yml
src/main/resources/application.yml
src/main/resources/db/migration/V000N__*.sql
src/main/java/com/ciac/<service>/
  Application.java        # @SpringBootApplication + profile-gated wiring
  config/                 # @ConfigurationProperties records; env assembly
  state/                  # AppState @Configuration: every client bean
                          #   @Lazy — the v0.17 lazy-init bar as a stated,
                          #   TESTED property (Pillar 4), not an accident
  schemas/                # records, enums, exceptions, decode helpers
  models/                 # row records + RowMapper constants
  db/                     # JdbcClient wiring (Flyway runs itself via starter)
  observability/
  routes/                 # @RestController per api pipeline
  logic/                  # compiler-owned lowered handlers
  services/               # seeded, user-owned stubs
  workers/                # listeners/loops + public handleMessageOnce
  clients/                # RestClient typed call clients
src/test/java/com/ciac/<service>/ScopeTests.java   # MockMvc suite (M6)
src/test/java/com/ciac/<service>/NoInfraBootTest.java  # Pillar 4 detector
tests/system/                                       # shared Python suite
```

**One artifact, two compose services.** The same jar runs as api or
workers selected by Spring profile — chosen over two jars because
Maven multi-module for generated code is complexity without parity
benefit: Rust's two-binary shape doesn't map cleanly onto Maven,
Python's one-package/two-entrypoints shape does. The workers profile
registers listener containers and schedulers and skips the web
server (`spring.main.web-application-type=none`); the api profile is
the inverse. Compose gets the same two services as every target with
only `workers_command` differing — no compose template change.

The profile-gated wiring, sketched because it is Pillar 4's
"explicit over scanned" rule made concrete — one readable file per
concern instead of ambient discovery:

```java
// state/WorkerWiring.java — Generated by CIaC.
@Configuration
@Profile("workers")
class WorkerWiring {
    @Bean ApplicationRunner processOrderWorker(AppState state) {
        return args -> ProcessOrderWorker.run(state); // jnats queue-group loop
    }
    // one bean per worker/consumer/job, compiler-emitted, diffable
}

// state/ClientsConfig.java — Generated by CIaC.
@Configuration
class ClientsConfig {
    @Bean @Lazy RestClient billingClient(ConfigProps cfg) {
        return RestClient.builder().baseUrl(cfg.billingUrl()).build();
    }
}
```

Every `@Bean` is `@Lazy` (or constructed lazily by the library, as
HikariCP and Lettuce are) — which is what `NoInfraBootTest` then
proves rather than assumes.

**Package naming.** `com.ciac.<snake_service>` by default, derived
via the factory's `NameForms`; overridable through the existing
`GenOptions.project_name` convention extended with an optional
`java_package` (defaulted, so no other target notices). The one new
GenOptions field this arc adds, recorded here.

A worked route sketch, pinning the same parity properties as plans
23/24 (envelope, validation, error mapping, publish-through-state):

```java
// routes/PlaceOrderApiController.java — Generated by CIaC.
@RestController
public class PlaceOrderApiController {
    private final AppState state;
    PlaceOrderApiController(AppState state) { this.state = state; }

    @PostMapping("/orders")
    public Envelope placeOrderApi(@RequestBody JsonNode body) {
        Order result = Schemas.decodeOrder(body);      // 400 on failure
        result = new PlaceOrder(state).handle(result);
        result = new RecordAudit(state).handle(result);
        state.publish("sim_vertical_slice.order_created",
                      Json.bytes(result));
        return Envelope.accepted(result);              // {"status":"accepted","data":...}
    }
}
```

`Envelope`/the `@RestControllerAdvice` error mapper are the
`AppError`/`httpx` analog (~80 generated lines, compiler-owned):
decode failures → 400; generated exceptions → their mapped status;
unknown → 500 + logged cause, canonical-reason body. `/health` is a
plain generated endpoint (NOT actuator health — one fewer
auto-config surface; actuator is enabled only for
`/actuator/prometheus` under `metrics`); `/openapi.json` serves the
embedded document (classpath resource, byte-faithful, same
single-source-of-truth comment). Handler classes take `state`, expose
`handle(payload)` — the uniform shape all five backends share.

**HTTP behavior parity, itemized** (plan 23's checklist; Java's
answers): JSON content type on all generated endpoints; 400 with
JSON body on malformed/invalid payloads (Jackson decode → presence
check → bean validation, in that order, all mapped by the advice);
MVC defaults for 404/405; 401 missing/invalid token vs 403 missing
scope (resource-server + requireScope split); health
`{"status":"ok"}` for `--live`. Pinned by C3, the smoke test, and
the system tests — same three layers as every target.

A second worked sketch — the schemas record with the decode
discipline, since records + presence checks are the Java-specific
pattern:

```java
// schemas/Order.java — Generated by CIaC.
public record Order(String id, double total) {
    public Order {
        Schemas.requireUuid("id", id);
    }
    /** Decode with absent/null/zero distinction (shared wire contract). */
    public static Order decode(JsonNode body) {
        Schemas.requireKeys(body, "id", "total"); // 400: absent required
        return new Order(body.path("id").asText(),
                         Schemas.requireNumber(body, "total"));
    }
}
```

And the statement-lowering table completing the `HostSyntax`
picture:

| HIR statement | Generated Java |
| --- | --- |
| `Let { name, value }` | `var name = <expr>;` |
| `Expr(value)` | `<expr>;` with an unused-var-tolerant shape mirroring the CIAC0045 posture |
| `Return(Some(v))` | `return <expr>;` |
| `Return(None)` | `return;` |
| `Fail { error, args }` | `throw new XException(args…);` (unchecked — the no-amendment decision) |
| `Publish { stream, value }` | `state.publish(subject, Json.bytes(<expr>));` |
| `Transaction { body }` | `state.tx().executeWithoutResult(tx -> { <body with JdbcClient-on-tx leaves> });` |

## Pillar 4 — The Spring-magic vs compiler-ownership pillar

Named as its own pillar because it is the real Java risk: Spring
auto-configuration DOES things — component scanning, bean
post-processing, classpath-triggered activation — and CIaC's
contract is that generated code is explicit, diffable, and does
nothing that isn't declared in the `.ciac` source. The decided
discipline, four rules:

1. **Explicit over scanned where it matters.** Routes, workers,
   clients, and state are registered via explicit `@Configuration`
   classes the compiler emits; component scanning is confined to the
   generated package so nothing outside the tree can be accidentally
   picked up, and the wiring for each concern is readable in one
   generated file.
2. **Starters trimmed to declared capabilities.** The pom's
   dependency list is exactly capability-driven — no redis starter
   without `cache`, no kafka without `queue Kafka`, no
   resource-server without `auth` — the same conditional-dependency
   discipline every backend's build-file template already follows,
   here doubly load-bearing because Spring activates behavior by
   classpath presence.
3. **No behavior from classpath accident.** Actuator exposure
   explicitly limited (prometheus endpoint only, only with
   `metrics`); Flyway pinned to CIaC's directory with
   repair/out-of-order disabled; explicit auto-config exclusions
   emitted wherever a transitively-present starter would otherwise
   activate an undeclared capability. The exclusion list is a
   template concern, golden-visible.
4. **The no-infra boot test is the magic detector.** A generated
   `NoInfraBootTest` boots the full application context with
   unreachable endpoints configured and asserts clean startup — if
   ANY bean eagerly connects (a classic Spring failure mode when a
   starter sneaks in), this test fails. It lands in M1 and runs in
   every `mvnw verify` forever, making lazy-init a permanently
   TESTED property — which is precisely what the scope suite (M6)
   and the sim seam (M9) structurally require, and the same bar
   v0.17 M11 retrofitted onto Rust, here built in from the first
   milestone.

## Pillar 5 — Database, transactions, migrations

**Placeholders.** JDBC is `?`-positional for ALL engines — Java has
the simplest placeholder story of any backend: the question-style
path of the shared `sqlph` filter plus the v0.13 M1
fields-first-id-last bind order applies uniformly with no per-engine
branching at all. The conformance topology assertion pins the SQL
text to the other targets.

**Verb lowering table** (M4's transcription target):

| Verb | Generated Java shape |
| --- | --- |
| `db.insert(T, v)` | `var row = v; state.jdbc().sql(INSERT_SQL).params(…).update(); row` — world-guarded in M9 |
| `db.get(T, id)` | `.query(RowMappers.T).optional()` → nullable |
| `db.update(T, id, v)` | fields-first-id-last; `updated == 0 ? null : row` |
| `db.delete(T, id)` | `updated > 0` |
| `db.query/count/delete_where` | shared predicate SQL + params |
| `cache.*` | Lettuce via `StringRedisTemplate` with the shared JSON codec |
| `object_store.*` / `email.send` / `search.*` / `http.call` | the generated wrappers of Pillar 7 |

**Transactions.** `TransactionTemplate` — programmatic, not
`@Transactional` — because the lowered `transaction {}` block is a
lexical scope the compiler emits code inside, and the annotation
approach carries the proxy self-invocation trap (a same-class call
skips the aspect silently: exactly the class of invisible-magic bug
Pillar 4 exists to exclude). Real atomicity; Python parity; standing
Rust-gap cross-reference.

**RowMappers.** Explicit per-model `RowMapper` constants with
explicit column order — deterministic, reflection-free, the
`FromRow`/Scan analog.

**Migrations via Flyway, CIaC-authored.** The differ's SQL, renamed
through `migration_filename`; the starter applies on boot (api
profile; the workers profile waits on ledger-current, matching the
cross-target decision recorded in the Go plan's M4 reconciliation).
The manifest/ownership and rename-replay machinery resolve the
mapped names through the trait hook — M2 includes the explicit
rename-replay proof because this is the hook's first non-identity
consumer, called out as such since 22UpdatePlan.md built it.

## Pillar 6 — Broker, workers, jobs, channels

**NATS:** jnats `Dispatcher` with queue-group subscriptions —
direct parity. **Kafka:** spring-kafka listener containers,
`groupId` = queue group, topic = subject, container concurrency =
worker `concurrency`, manual-immediate ack after successful handling
(at-least-once, the v0.11 M3 contract); record headers carry
traceparent both directions.

**Workers.** The uniform seam:

```java
public static final String SUBJECT = "sim_vertical_slice.order_created";
public static final String QUEUE_GROUP = "...";
public static final int MAX_RETRIES = 2;

public static void handleMessageOnce(AppState state, Order payload) { … }
static void handleMessage(AppState state, Order payload) { /* retry loop */ }
```

`handleMessageOnce` public for the M9 sim runner and attempt
counting — the preserved M1-finding seam, fifth backend in a row.

**Jobs.** `@Scheduled(cron=…)` — Spring cron is SIX-field
seconds-first but accepts weekday 0–7 natively, so the translation
is a `"0 "` prefix and nothing else. The Rust-specific weekday
rewrite is NOT reused; a `spring_cron` derivation lives in the Java
backend as a filter (the factory's mechanism for exactly this),
unit-tested against the same cron-equivalence cases the Rust
translation carries. `handleTickOnce` public; `catch_up` per the
shared contract.

**Channels.** Raw `TextWebSocketHandler` registration + `SseEmitter`
endpoints, each bridging a plain (non-group) subscription — fan-out
parity, probed by the unchanged generated system tests.

## Pillar 7 — Auth, scopes, ontology remainder, observability

**Auth.** The resource-server starter configured per provider: HS256
with the shared secret (JWT provider) via a `SecretKeySpec` decoder;
issuer/JWKS RS256 (OAuth2) via the starter's built-in lazily-cached
JWKS handling. Claims exposed uniformly (`sub`, `scopes`); the
generated `requireScope` check produces the same 403 semantics,
wired per scoped route from the shared scope collection.

**Scope tests.** MockMvc — full-context request injection with no
listener (the oneshot analog); the generated `ScopeTests` mints HS
tokens and asserts the 403-without/200-with pair per scope, JWT-only
with the standing OAuth2 exclusion sentence. Because MockMvc boots
the real context, this suite doubles as a second run of the Pillar 4
magic detector on every verify, forever.

**Ontology.** AWS SDK v2 S3 wrapper (endpoint override + path-style
for MinIO; same five config fields); starter-mail against Mailpit
(same six); opensearch-java (same one); `RestClient` wrappers for
external_http instances and generated `clients/` on the same
base-URL env convention.

**Observability.** Logback + logstash-encoder JSON with the shared
field conventions; Micrometer prometheus at `/actuator/prometheus`
when declared; the OTel starter when `tracing` is declared with OTLP
env parity and the broker header propagation of Pillar 6 — proven by
the cross-target trace test at five targets, the arc's final
extension of it.

**Deployment.** Dockerfile: `maven:3.9-eclipse-temurin-21` build
(`./mvnw -q -B -DskipTests package`, dependency layer cached first
via `dependency:go-offline` for image-build layer reuse) →
`eclipse-temurin:21-jre` with the fat jar.

**Scope-test sketch** (MockMvc as the oneshot analog, doubling as
the magic detector):

```java
// ScopeTests.java — Generated by CIaC.
@SpringBootTest(properties = NoInfra.PROPS) // unreachable endpoints
@AutoConfigureMockMvc
class ScopeTests {
    @Autowired MockMvc mvc;

    @Test void ordersWriteScopeEnforced() throws Exception {
        mvc.perform(post("/orders").contentType(APPLICATION_JSON)
                .header("Authorization", bearer(token(/* no scopes */)))
                .content(orderBody()))
           .andExpect(status().isForbidden());
        mvc.perform(post("/orders").contentType(APPLICATION_JSON)
                .header("Authorization", bearer(token("orders:write")))
                .content(orderBody()))
           .andExpect(result -> assertNotEquals(403,
                result.getResponse().getStatus())); // mechanism proof
    }
}
```

### The config/env surface

Same cross-target env contract; Java-specific rows only: datasource
URL/username/password assembled from the discrete vars (the JDBC
containment note); `spring.profiles.active` selecting api/workers;
everything else — `REDIS_URL`, `NATS_URL`/`KAFKA_URL`,
`JWT_SECRET`/`OAUTH_ISSUER`, `<SVC>_URL`, ontology instance fields,
`OTEL_*` — identical names and semantics per the shared contract
(plan 23's table; not repeated because being shared is the point).
`application.yml` maps env→properties with `${VAR}` placeholders so
the generated file is readable AND twelve-factor.

### Template inventory

Estimate: ~36 templates, ~3,000–3,300 lines (the largest of the
three — Spring's per-concern configuration classes add files),
checked at M5:

| Group | Templates |
| --- | --- |
| project | `pom.xml`, `Dockerfile`, `README.md`, `system-README.md`, `application.yml` |
| app core | `Application.java`, `state config classes` (per-capability `@Configuration`), `ConfigProps.java`, `Envelope.java`, `ErrorAdvice.java`, `Observability.java` |
| data | `record.java` (per record), `Enums.java`, `RowMappers.java`, `Db.java`, `ResourceStore.java`, `Schemas.java` (decode helpers) |
| http | `RouteController.java`, `ResourceController.java`, `ChannelHandler.java`, `SseController.java` |
| async | `Worker.java`, `Consumer.java`, `Job.java`, `Queue.java` (nats/kafka wiring) |
| handlers | `Logic.java` (compiler-owned), `Service.java` (seeded stub) |
| ontology | `Cache.java`, `ObjectStore.java`, `Email.java`, `Search.java`, `HttpClients.java`, `AuthConfig.java` |
| tests/sim | `ScopeTests.java`, `NoInfraBootTest.java`, `SmokeTest.java`, `SimRunner.java` (M9) |

Same no-novel-file-kinds parity check as plans 23/24: every row has
an audited Python/Rust analog.

## Implementation map

| Artifact | Content |
| --- | --- |
| `crates/ciac-backend-java/src/lib.rs` | `TargetInfo` (incl. the Flyway filename mapping), `java_type` + `spring_cron` filters, emission table, gating ladder |
| `crates/ciac-backend-java/src/lower.rs` | `HostSyntax for JavaSyntax` — leaves only |
| `crates/ciac-backend-java/templates/` | the ~36 templates above |
| `crates/ciac/src/commands.rs` | ONE registry line |
| `tests/tests/snapshots/` | `gen__java__*` goldens (registry-enumerated) |
| `.github/workflows/ci.yml` | `generated-java` (cached, M5-scoped) + system rows |
| docs | backends.md section, simulation.md column (M9), generated table rows |
| shared | NO lower_core changes expected (the no-amendment decision) |

## Capability parity checklist

Same matrix discipline as plans 23/24 (module / proving example /
milestone), with the identical example-to-milestone mapping (M1
core, M2 data incl. the Flyway mapping proof, M3 async, M4
handlers/transactions/relations, M6 auth/scopes, M7
ontology/clients/observability/system rows, M8 integration, M9 sim).
The signed-off copy lives in M8's notes with goldens and proofs
linked per row.

## Determinism and supply chain

Exact pins for every dependency AND every Maven plugin (plugin
versions float by default in Maven — pinning them is the
Java-specific determinism trap, named here and enforced by the
conformance pom lint); the wrapper pins Maven; `-B` (batch) mode
everywhere; no version ranges; generated pom + wrapper
golden-snapshotted. Spotless (google-java-format, pinned) makes
formatting canonical-and-asserted — the gofmt/prettier analog.
Dependency convergence enforced (`maven-enforcer` with
requireUpperBoundDeps) so transitive drift fails loudly at
generation-validation time rather than at runtime.

## Pillar 8 — Build hermeticity and the validate-latency budget

The named Java-specific operational risk, treated with numbers and a
pre-agreed decision rather than optimism. A cold `mvnw verify`
downloads the world; the JVM+plugin startup tax is real per
invocation. Mitigations, all decided now:

1. The committed wrapper pins Maven itself; the pom pins every
   dependency and plugin version exactly — no ranges, ever
   (determinism is a repo invariant; a version-range pom would be
   the only non-deterministic generated artifact in the fleet, so it
   is forbidden).
2. The validator is ONE invocation (`./mvnw -q -B verify` covering
   compile + Spotless + tests), not separate compile/lint/test
   commands — the startup tax paid once.
3. CI dependency caching in both this repo's `generated-java` job
   and the generated projects' own CI workflow (the `ci_test_steps`
   include the setup-java cache flag).
4. M1 records cold/warm validate wall-clock as the baseline; every
   milestone re-records it; the numbers live in this file's
   milestone notes.
5. The pre-agreed scoping decision: if the full 26-example matrix
   blows the CI budget at M5's measurement, `generated-java` narrows
   to the capability-covering subset (the same deliberate scoping
   the `generated-system` job already practices for Rust, disclosed
   in the workflow comment) — chosen at M5 from data, not improvised
   later.

## Pillar 9 — Simulation (gated) and the divergence ledger

| Row | Python | Rust | Java |
| --- | --- | --- | --- |
| sim | full, record/replay | narrow, no replay | narrow slice, M9 (gated); no replay, disclosed |
| scope tests | full | JWT-only | JWT-only, same reason |
| `transaction {}` | atomic | disclosed non-atomic | atomic (TransactionTemplate) |
| `Int` | arbitrary | i64 | long (i64) |
| `Option` decode | native | native | @Nullable component + presence check, boundary-tested |
| migrations executor | generated runner | sqlx::migrate! | Flyway on CIaC SQL (renamed), CIaC sole author |
| cron translation | none (croniter) | seconds-prefix + weekday rewrite | seconds-prefix only |
| deploy artifact | image + venv | stripped binary image | JRE image (~200MB); jlink/native disclosed future |
| validate latency | seconds | tens of seconds | slowest of five — budgeted, Pillar 8 |

M9 mirrors the v0.17 M11 continuation: `World.java` is a narrow
restatement (Java cannot vendor `ciac-sim`'s Rust source; Python's
disclosed position, same docstring discipline) — fake table map +
fake queue + occurrence-counted failure rules (`error` only).
`state.publish` and the `db.insert` leaf gain the world-guard.
A generated `SimRunner` (test-scoped main or `@SpringBootTest`
driver — decided at implementation against how MockMvc composes with
the child-protocol's one-line-JSON stdout contract, recorded when
decided) drives MockMvc for requests (real status codes),
`handleMessageOnce` retry budgets for drains, the `spring_cron`
due-instant computation for advances, world state for expects.
`ciac sim --target java` goes through `SimSupport::Narrow` with the
shared `unguarded_verbs` gate. Acceptance: both checked-in scenarios
reproduce `{"ProcessOrder":3}/{"Reconcile":1}` and
`{"ProcessOrder":100}/{"Reconcile":7}` exactly; the order-system
refusal names its reasons; sim-vertical-slice × java joins the
ratchet CI matrix.

## Diagnostics, gating, and docs impact

CIAC0011 gating per milestone via `supports()`; conformance harness
reports gated pairs as disclosed skips. No new error codes expected;
the standard code+docs procedure applies if implementation surfaces
a Java-specific diagnosable condition. Docs: generated provider
table flips rows per milestone; docs/backends.md gains the Java
section (deps + divergence ledger + the Spring-discipline summary of
Pillar 4); docs/simulation.md gains the Java column at M9; README
target list; and M9 additionally delivers the arc-closing artifacts
below.

Deployment-layer interaction, inherited like every target: k8s and
Terraform from the shared generators (image from the Dockerfile,
`--profile` sizing unchanged); the generated project's CI from
`ci_test_steps` (with the caching flag that Pillar 8 requires); the
keyed store's cache-aside path against Lettuce with the shared
TTL/key conventions, asserted by the existing capability
round-trips. Multi-service systems and `ciac new --target java`
follow plans 23/24's pattern verbatim (per-service directories,
scaffold in M8, registry-derived docs rows).

## Relationship to the forecast documents

Same posture as the rest of the arc: v0.19–v0.21 remain open;
nothing here consumes them. Two forward notes for the v0.19
planning pass, since this plan closes the arc that feeds it: the
`TransactionTemplate` leaf and the `state.publish` seam are Java's
outbox attachment points (as `sql.Tx`/`state.Publish` are Go's and
the drizzle transaction/`state.publish` are TS's — the same two
names on five targets, which is the factory's outbox dividend
banked in advance); and Micrometer/OTel wiring gives v0.20's
provenance work its Java join point without new instrumentation
surface.

## Milestones

1. **M1 — Reconcile + skeleton to ping-parity.** Reconcile against
   both prior arcs' actuals (recorded here). Copy the skeleton;
   register `TargetInfo` (one external line — factory assertion #3,
   the last). Emit pom+wrapper/Dockerfile/README/AGENTS.md/
   application.yml/config/state (all `@Lazy`)/observability/
   Envelope+advice/health/openapi-embed, plus `NoInfraBootTest`
   (Pillar 4's detector, from day one). ping verifies fully locally
   via `./mvnw -q -B verify` (Java toolchain present); cold/warm
   validate wall-clock recorded (Pillar 8 baseline). Goldens begin;
   `supports()` gated.
2. **M2 — Records, schemas, models, CRUD, keyed store, Flyway.**
   Records/enums/exceptions/decode helpers (presence-check
   discipline + boundary tests), RowMappers, JdbcClient wiring,
   typed CRUD + keyed store on all three engines through the
   uniform `?` path, Flyway with the non-identity filename mapping —
   including the explicit rename-replay proof (first real consumer
   of the factory hook). sqlite-notes fully local (xerial file
   engine, zero Docker); crud/mysql static-local, round-trips
   CI-delegated.
3. **M3 — Broker, workers, jobs, channels.** jnats + spring-kafka,
   retry + public `handleMessageOnce`, `@Scheduled` with the
   prefix-only `spring_cron` filter (equivalence-tested), WS/SSE.
   The four broker/schedule examples verify.
4. **M4 — Typed handlers: `HostSyntax` for Java.** All verbs per
   Pillar 5's table, TransactionTemplate atomicity, JsonNode paths
   with the missing-path check, switch-expression match where fixed,
   builtins. No contract amendment expected (Pillar 2's unchecked-
   exception decision); if falsified, goldens-first per the standing
   procedure. typed-handlers/typed-video/domain-orders/query-verbs/
   extras-verbs verify; equivalence test → five targets.
5. **M5 — CHECKPOINT.** The factory's final grade: measured cost vs
   the twice-updated model; conformance harness green across five
   targets (OpenAPI byte-equality ×5, topology, boundary decode);
   the Pillar 8 latency measurement and the pre-agreed CI-scoping
   decision taken from data. Go/no-go for the remainder.
6. **M6 — Auth, scopes, scope tests.** Resource-server both modes,
   requireScope, MockMvc `ScopeTests` green under zero
   infrastructure; order-system and oauth-echo verify.
7. **M7 — Ontology remainder + call clients + observability
   completion.** S3/mail/search wrappers, RestClient call clients,
   OTel end-to-end (five-target trace test), metrics endpoint.
   multi-service-media, inventory-system, ontology-growth,
   traced-checkout, dev-identity verify; `--system` CI rows
   (java × inventory-system, × mysql-notes, × sim-vertical-slice)
   with compose-build times recorded against the Pillar 8 ledger.
8. **M8 — Whole-repo integration.** Every example verifies or is
   reason-gated (target: zero gates); goldens complete; generated
   docs tables regenerate; `ciac dev` session test (jar rebuild
   loop); MCP exercised; evolution/rename-replay re-proven against a
   Java tree; `generated-java` CI job with the M5 scoping decision
   applied and stated in the workflow comment.
9. **M9 — Simulation slice (gated) + version + the five-backend
   retrospective.** Pillar 9's slice with exact-outcome acceptance
   and the refusal case; ratchet row; docs. Workspace version bump.
   Then the arc-closing deliverables: the consolidated five-backend
   cost model (the final, measured answer to 22UpdatePlan.md's
   question), the generated support matrix as the single source of
   truth, and the cross-target disclosed-gaps ledger (sim depth per
   target, record/replay, OAuth2 scope-testing, Rust's transaction
   atomicity catch-up, Java's image-size follow-up) — handed to
   whichever forecast track executes next.

### Per-milestone exit checklists

- **M1 exits when:** reconciliation notes committed; the registry
  line is the only external edit; ping passes `mvnw -q -B verify`
  live; `NoInfraBootTest` exists and passes (the magic detector is
  born before any capability exists to mask it); cold/warm validate
  times recorded; ping goldens committed.
- **M2 exits when:** three-engine goldens exist; sqlite-notes
  verifies live zero-Docker; the rename-replay proof through the
  Flyway filename mapping passes (the factory hook's first
  non-identity consumer — this plan's most load-bearing single
  test); C3 ×5 on M2-scope examples; boundary decode suite passes.
- **M3 exits when:** the four async examples verify; seam methods
  import-tested; `spring_cron` equivalence cases pass.
- **M4 exits when:** every verb row goldened; domain-orders rollback
  proof on local sqlite; equivalence suite ×5; the no-amendment
  claim confirmed (or the amendment landed goldens-first with the
  deviation recorded).
- **M5 exits when:** the final cost table is committed; C1–C5 ×5
  green; the latency data and CI-scoping decision recorded;
  go/no-go sentence recorded.
- **M6 exits when:** MockMvc suite green under zero infrastructure;
  textual parity of the OAuth2 exclusion.
- **M7 exits when:** ontology examples verify; trace test ×5; the
  three system rows merged with build-time data in the ledger.
- **M8 exits when:** zero unexplained gates; dev/MCP/evolution
  transcripts attached; `generated-java` green under the recorded
  scope.
- **M9 exits when:** canonical outcomes byte-exact; refusal reasons
  named; ratchet row merged; docs + version done; the three
  arc-closing artifacts published in this file and docs/backends.md.

## Open questions resolved at implementation (pre-registered)

1. **SimRunner packaging** (test-scoped main vs `@SpringBootTest`
   driver vs a `sim` profile on the main jar) — decided in M9
   against the child-protocol's one-line-stdout contract; recorded.
2. **Workers' migration posture** — inherits whatever cross-target
   decision plan 24's M4 reconciliation recorded; Java implements
   the same posture via a profile-gated Flyway strategy.
3. **Package-name override surface** (`java_package` GenOptions
   field) — final shape decided in M1 with scaffold/docs impact
   recorded; defaulted so no other target notices.
4. **Records vs classes for payloads with >254 components or other
   record edge cases** — records are the decision; if a real
   program hits a records limitation, the fallback (final class +
   generated equals/hashCode) is per-record, golden-localized, and
   recorded with the trigger.

## Verification strategy

Standard per-milestone discipline: fmt/clippy/test workspace green;
goldens reviewed diff-by-diff, never blind-accepted; live proofs as
named with Docker-delegation honesty. Java-specific standing checks:
`NoInfraBootTest` in every verify (lazy-init as a permanent tested
property and Spring-magic tripwire); Spotless check inside the
single `mvnw verify` (canonical formatting asserted, the gofmt
analog); exact-pin lint on the generated pom in the conformance
harness; validate wall-clock recorded per milestone (Pillar 8's
ledger is data, not anecdote). The generated pom + wrapper are
golden-snapshotted; dependency updates are deliberate,
golden-visible changes.

The proof ledger by layer (plan 24's format, Java's oracles):

| Claim | Oracle |
| --- | --- |
| generated code compiles/lints/tests | `mvnw -q -B verify` (live locally — temurin is a plain install) |
| wire contract equals other targets | C3 OpenAPI byte-equality ×5; C7 boundary decode/encode |
| topology equals other targets | C4 — including Flyway-renamed migration CONTENT equality, the anti-mutation tripwire |
| logic behavior equals other targets | the equivalence suite ×5 |
| broker/channels/capability round-trips real | system tests via the three `--system` rows (Docker-delegated) |
| lazy init + no Spring magic | `NoInfraBootTest`, every verify |
| scope mechanism, zero infra | MockMvc `ScopeTests` |
| sim outcomes match canon | M9 exact-outcome acceptance |
| fake≠real drift caught | ratchet row on sim-vertical-slice |
| build determinism | pinned pom + plugins + enforcer + snapshotted wrapper |

## Milestone dependencies and parallelism

M1→M2→M3→M4→M5 sequential; M6/M7 independent after M5; M8 needs
both; M9 last. No shared-code changes are expected at all (the
no-amendment decision), so this plan — uniquely in the arc — should
touch nothing outside its own crate, templates, goldens, CI rows,
and docs after M1's registry line; any deviation from that is
itself a factory finding recorded at M5. The slow validate loop
(Pillar 8) argues for batching golden-affecting work within
milestones rather than spreading it — an execution note, not a
plan change.

## Explicit cuts

No JPA/Hibernate mode. No Kotlin. No Gradle variant. No GraalVM
native-image or jlink runtime (recorded deployment-maturity
follow-up). No Quartz. No STOMP. No Spring Cloud anything — service
discovery, config servers, and gateways are exactly the
infrastructure CIaC itself owns and generates. No multi-module Maven
for multi-service systems (per-service directories, like every
target). No actuator surface beyond the prometheus endpoint. No sim
record/replay. No reactive variant, ever, for generated handler
code.

## Risks

- **Spring auto-config drift across Boot minor versions changes
  generated behavior.** Boot version pinned exactly in the pom;
  goldens catch template-visible drift; `NoInfraBootTest` catches
  behavioral drift; Boot upgrades become deliberate, versioned,
  golden-visible changes with a changelog note.
- **Validate latency makes the full matrix impractical.** Pillar 8's
  pre-agreed, data-driven narrowing keeps CI honest instead of
  slow-then-quietly-deleted.
- **Jackson/records/nullability edge cases.** The decided
  @Nullable-plus-presence-check approach is the boring, well-trodden
  one, and the shared boundary-decode suite runs against Java
  verbatim.
- **The jar-with-profile shape surprises compose assumptions.**
  Contained entirely in `TargetInfo.compose.workers_command`;
  the system tests that probe both containers validate it
  end-to-end.
- **Flyway asserts authority it shouldn't** (repair/out-of-order/
  checksum drift). Configuration pins it to executor-only; the
  conformance harness's topology assertion compares migration
  CONTENT across targets, so any Flyway-side mutation of CIaC's SQL
  would fail the matrix.
- **Spring pulls a transitive capability in by classpath.** Pillar
  4's exclusion discipline + the boot test; the pom's
  capability-driven dependency list is golden-visible.

## Confidence and handoff

High on coverage, medium on the two named engineering risks — both
carrying decided mitigations with tests, budgets, and pre-agreed
decision points rather than optimism. This plan ends the backend
arc: M9's retrospective delivers the consolidated five-backend cost
model, the generated support matrix, and the disclosed-gaps ledger
as the input to the next selection. The natural successor is named:
v0.19's outbox/idempotency content, because its dual-write machinery
must be designed once in the shared model and rendered five times in
templates — exactly the shape this arc's factory was built to make
cheap, and the first post-arc test of whether it actually did.
