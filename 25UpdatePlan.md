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

   **Shipped (v0.25 M1):** `crates/ciac-backend-java` — `JavaBackend`
   with `TargetInfo` (`project_marker: "pom.xml"`, `migrations_dir:
   "src/main/resources/db/migration"`, `migration_filename` the
   plan's first non-identity consumer — `V{seq:04}__{slug}.sql` for
   Flyway — every other current target keeps the identity mapping;
   `validate`: one `./mvnw -q -B verify` step (compile + Spotless
   format check + test, Pillar 8's startup-tax-paid-once decision);
   `ci_test_steps` via `actions/setup-java@v4` (temurin 21, maven
   cache) then the same `verify` invocation; `dev.rebuild`: `./mvnw
   -q -B -DskipTests package` /`RestartStyle::Restart` (one-jar
   rebuild, no per-service hot reload); `sim: None` until M9).
   `supports()` gated to exactly `Component::Api` — Go's own M1
   finding holds here for the identical reason: `ping.ciac`'s
   `pipeline Echo: Return` binds no handler, so no `Service` node is
   in play, and claiming that kind before any template implements it
   would pass gating and fail on an undefined template variable
   instead of a clean `CIAC0011`. Registered in
   `crates/ciac/src/commands.rs::backends()` and
   `tests/src/lib.rs::backends()` per `docs/backends.md`'s recipe —
   the only two external edits outside the new crate itself.

   Package layout: `com.ciac.<module, underscores stripped>` (no new
   `GenOptions::java_package` override field — M1's disclosed scope
   reduction, since no reachable example needs a custom package name
   yet). Emitted: `pom.xml` (Spring Boot 3.3.5 parent,
   `spring-boot-starter-web` + `jackson-datatype-jsr310` +
   `spring-boot-starter-test`, `spotless-maven-plugin` with
   `googleJavaFormat` bound to the `verify` phase), the real Maven
   wrapper (`mvnw`/`mvnw.cmd`, live-generated once via `mvn -N
   wrapper:wrapper -Dmaven=3.9.11`'s modern "only-script" type and
   vendored verbatim — no `MavenWrapperDownloader.java` needed),
   `Dockerfile` (multi-stage, `maven:3.9-eclipse-temurin-21` build via
   `./mvnw`, `eclipse-temurin:21-jre` runtime), `README.md`,
   `application.yml` (`server.port: 8000`,
   `spring.threads.virtual.enabled: true`), `Application.java`,
   `AppState.java` (empty `@Component` marker — every provider client
   bean added in later milestones goes `@Lazy`, `NoInfraBootTest`
   proving the no-magic contract from day one, Pillar 4's detector),
   `HealthController` (`/health`, plus `/openapi.json` serving a
   classpath resource copy of the same doc `openapi.json`
   `build_document` produces at the project root), `Envelope`/
   `ErrorAdvice`/`BadRequestException` (the shared `{"status":
   "accepted"|"error", "data": ...}` wire contract, `@RestControllerAdvice`
   mapping `BadRequestException` -> 400 and any other `Exception` ->
   500), `Schemas.java` (one Jackson `ObjectMapper` with
   `JavaTimeModule` + `PropertyNamingStrategies.SNAKE_CASE` for
   camelCase-Java-field <-> snake_case-JSON-key mapping,
   `requireKeys`/`requireUuid` presence/format checks — Bean
   Validation deferred per the module doc's disclosed M1 scope note),
   one real Java `record` per CIaC record with a static `decode`
   method, one `@RestController` per api.

   **Real bug found live #1 — XML comments cannot contain `--`.**
   `pom.xml.j2`'s header comment (`<!-- pom.xml -- Generated by CIaC.
   -->`) broke Maven's POM parser (`Non-parseable POM ... in comment
   after two dashes (--) next character must be > not " "`) — a
   fundamental XML rule (no `--` substring anywhere inside a comment,
   not only adjacent to `-->`). A first fix attempt still failed
   because the *replacement* explanatory text itself quoted `"--"`
   inside the comment body; the working fix drops the double-hyphen
   entirely (`<!-- pom.xml — Generated by CIaC. -->`, em dash).

   **Real bug found live #2 — `google-java-format` is a required
   generation-time dependency, exactly like Go's `gofmt` precedent.**
   Hand-written 4-space-indented `.java` templates fail Spotless's
   `googleJavaFormat` check (2-space AOSP-like style, record-parameter
   collapsing, etc.) during a real `./mvnw -q -B verify` — confirmed
   live via the exact diff Maven printed. Unlike `gofmt`,
   `google-java-format` does not ship with the JDK, so it is
   **vendored**: `crates/ciac-backend-java/vendor/
   google-java-format-1.19.2-all-deps.jar` (the self-contained
   "all-deps" build off Maven Central — the plain jar alone throws
   `NoClassDefFoundError` for Guava at runtime), embedded via
   `include_bytes!` and materialized once to a stable temp path
   (`java -jar` needs a real file, not stdin bytes). A new
   `google_java_format()` function in `lib.rs` shells out to it with
   the `--add-exports`/`--add-opens` flags JDK 16+'s strong
   encapsulation requires to reach `com.sun.tools.javac.*`
   internals — modeled directly on Go's own `gofmt()` (piped
   stdin/stdout, `output.status.success()` check, clear error if
   `java` isn't on `PATH`). Every `.java`-suffixed emission in
   `emit_service` now routes through a `render_java` wrapper
   (`render` then `google_java_format`) instead of bare `render` —
   pom.xml/Dockerfile/README/`application.yml`/the wrapper properties
   file stay unformatted since they aren't Java source.

   **Live proof:** `ciac build examples/ping.ciac --target java`
   generated 22 files; `./mvnw -q -B verify` passes end to end
   (compile, Spotless check, `NoInfraBootTest` boot-and-shutdown) —
   exit 0, confirmed by direct exit-code capture, not just absence of
   "BUILD FAILURE" in quiet output. `./mvnw spring-boot:run` then
   answered real HTTP requests: `GET /health` ->
   `{"status":"ok"}`; `POST /echo` with a valid `Message` body ->
   `{"status":"accepted","data":{"id":...,"text":...}}`; the two
   negative paths -> `HTTP 400` with `{"status":"error","data":
   "field \"id\": invalid uuid4 value"}` and `{"status":"error",
   "data":"missing required field \"id\""}` respectively (the same
   presence/format-check wire contract Go/TS/Rust/Python already
   produce); `GET /openapi.json` served the real embedded document.
   `./mvnw -q -B verify` wall-clock (this sandbox, proxied Maven
   Central, not representative of CI hardware, and — like TS's own
   M1 `npm ci` disclosure — not a genuinely empty-cache measurement
   since `~/.m2` already held Spring's dependency tree from this
   milestone's own earlier iteration): ~10.6s with only this
   project's own `com.ciac` artifacts cleared from the local repo,
   ~6.3s on an immediate re-run with everything warm.

   Two pre-existing clippy findings in the freshly-scaffolded crate
   (`needless_borrows_for_generic_args` on `Value::from_serialize(&
   ())`, `needless_question_mark` in the `render` closure) were fixed
   as part of getting `cargo clippy --workspace --all-targets -- -D
   warnings` clean — not new code added by this milestone's own
   changes, but surfaced by them.

   **Real bug found live #3 — the classpath-resource `openapi.json`
   copy tripped C3's cross-target conformance check, the exact gap
   Go's own M1 already hit and fixed.** `cargo test --workspace`'s
   `c3_openapi_is_byte_identical_across_targets` failed once
   `docs/targets.json` was regenerated and the suite reached
   `ciac-integration-tests` (a prior partial run had stopped earlier,
   at `targets_cli`'s own now-fixed staleness failure, before ever
   reaching conformance — worth naming since it's why this wasn't
   caught in the very first full-suite pass). C3 matches files by
   `ends_with("openapi.json")`; the project-local classpath copy
   `HealthController` reads (needed because Spring, unlike a runtime
   `os.ReadFile`, wants a bundled resource, not a filesystem path)
   was originally named `src/main/resources/openapi.json`, making
   Java's own `openapi.json`-suffixed path set `{openapi.json,
   src/main/resources/openapi.json}` diverge from every other
   target's single-path set. Fixed identically to Go's own
   `cmd/api/apidoc.json` precedent: renamed to
   `src/main/resources/apidoc.json` (does not end with the literal
   substring `"openapi.json"`, so C3 never needs a Java-specific
   exception), `HealthController`'s `ClassPathResource` load updated
   to match. Live-reverified after the fix: `./mvnw -q -B verify`
   still exit 0, `GET /openapi.json` still served the identical
   document over a live `spring-boot:run` process.
2. **M2 — Records, schemas, models, CRUD, keyed store, Flyway.**
   Records/enums/exceptions/decode helpers (presence-check
   discipline + boundary tests), RowMappers, JdbcClient wiring,
   typed CRUD + keyed store on all three engines through the
   uniform `?` path, Flyway with the non-identity filename mapping —
   including the explicit rename-replay proof (first real consumer
   of the factory hook). sqlite-notes fully local (xerial file
   engine, zero Docker); crud/mysql static-local, round-trips
   CI-delegated.

   **Shipped (v0.25 M2):** `supports()` widened to
   `Component::Database` (all three engines uniformly — JDBC needs no
   per-engine gating, sharpening Go's own M2 finding further since
   Java's placeholder story has no per-engine branch at all, not even
   the `?`-vs-`$N` one) plus `Component::Service { signature: None }`
   (the crud-synthesized store marker — Go's own M2 finding held here
   too, found the identical way before reading the sema source).
   `Some(_)` (typed handlers) stays refused until M4.

   New per-resource templates (`ctx.resources`, entirely shared/
   precomputed already — `ResourceCtx`/`RecordCtx`'s `select_cols`/
   `insert_placeholders`/`update_assignments`/`update_where` needed
   zero new Rust-side computation, only Java spellings): `<Name>In`
   (create/update payload, presence-checked via `Schemas.requireKeys`
   for typed resources, plain decode for the keyed variant matching
   every other target's own `{"data": ..}`-wrapped-body contract);
   `<Name>Entity` (the keyed variant's generic row shape, deliberately
   *not* named `<Name>` so a keyed resource sharing a name with an
   unrelated wire record can never collide in the `schemas` package);
   `RowMappers` (one explicit, reflection-free `RowMapper<T>` constant
   per typed resource — Pillar 5's own decision, not
   `BeanPropertyRowMapper`/`DataClassRowMapper` reflection, matching
   Go's/Rust's own explicit-Scan-order discipline); `<Name>Store`
   (JdbcClient-backed CRUD, `?`-only SQL via the shared `sqlph`
   filter); `<Name>Controller` (raw-entity JSON responses, no
   `Envelope` wrapper — CRUD resources carry their own response
   contract, matching every other target's own resource routes, not
   the pipeline-`api` contract `ApiController` uses).
   `NotFoundException` (404, mirrors `BadRequestException`'s 400) is
   now emitted unconditionally alongside it, so `ping.ciac`'s own
   golden picked up the addition too — disclosed, not a scope leak:
   M4's own typed handlers will need it regardless, and duplicating
   the emission per-milestone would just be busywork.

   **The AppState/DataSource/Flyway design, arrived at only after
   live-testing two real risks rather than assuming either:**
   1. **Flyway does support SQLite in `flyway-core` directly** (no
      separate community plugin needed, unlike some historical Flyway
      versions) — verified live with a standalone `Flyway.configure()
      .dataSource(url, null, null).load().migrate()` run against a
      real sqlite file before writing a single line of the real
      integration, specifically to avoid discovering a hard blocker
      after building around it.
   2. **The JDBC embedded-credential URL problem, found by reasoning
      about the actual JDBC URL grammar, not by assumption:**
      docker-compose's own `DATABASE_URL` value (`jdbc:postgresql://
      user:pass@host:port/db`, the *userinfo* form every other
      target's own native driver already accepts) is not one the
      postgres/mysql JDBC drivers parse — userinfo credentials aren't
      part of the JDBC URL grammar either driver implements. Fixed
      with a new `DataSources.open(rawUrl)` (in `state/
      DataSources.java`) that strips `jdbc:` , re-parses the remainder
      as a real `java.net.URI` (hierarchical once the outer `jdbc:`
      layer is gone), and re-attaches the extracted `user`/`password`
      as discrete `HikariConfig` properties instead of guessing at a
      URL shape the driver would accept. Verified live in isolation
      against both the postgres- and mysql-shaped strings compose
      would actually emit (`jdbc:postgresql://postgres:postgres@db:
      5432/postgres`, `jdbc:mysql://root:root@db:3306/mysqlnotes`) —
      not live-tested end-to-end against a running server, since
      crud-notes/mysql-notes stay gated this milestone (below), but
      the parsing itself is proven correct on the exact strings it
      will see whenever M6/M7 unlock those examples.

   `AppState` hand-rolls `@Lazy @Bean DataSource`/`JdbcClient` beans
   per db instance (reading the raw URL via `@Value("${ENV_VAR:
   <engine-appropriate localhost default>}")`, mirroring every other
   target's own `envOr`-style fallback) rather than letting Spring's
   own `DataSourceAutoConfiguration` build one from `spring.datasource.*`
   properties — the same "compiler owns the wiring, no implicit
   magic" rule Pillar 4 already applies to provider clients, now
   applied to the connection pool itself. `spring.flyway.enabled:
   false` in `application.yml` stops `FlywayAutoConfiguration` from
   independently resolving those same `@Lazy` beans the moment it
   sees flyway-core + a `DataSource` bean on the classpath — Flyway
   instead runs from an explicit `CommandLineRunner` in `AppState`
   (`DataSources.migrate(..)`, applying whatever the shared differ
   wrote under `src/main/resources/db/migration` — Java needed zero
   new Rust-side migration-generation code, since that machinery is
   already target-neutral and `TargetInfo::migration_filename`/
   `migrations_dir` were already wired at M1), immediately followed
   by a `CREATE TABLE IF NOT EXISTS` bootstrap for every CRUD resource
   (typed and keyed) bound to that instance — CRUD resources
   synthesize no `Table` IR node (confirmed by reading
   `ciac_sema::build::crud`, not assumed), so they were never going to
   be covered by the shared differ; every other current target
   already treats CRUD-resource schema this same way, separately from
   the versioned-migration ledger, for the identical reason.

   **A real, disclosed architectural finding, not a hedge:** this
   `CommandLineRunner` runs unconditionally whenever `ctx.has_db`,
   which means it forces the `@Lazy` `DataSource` to resolve during
   every boot, `NoInfraBootTest` included — reasoned through carefully
   *before* writing the code (HikariCP validates a connection eagerly
   at pool construction, unlike Go's `sql.Open`/Rust's
   `PgPool::connect_lazy`, which never dial until first use) and then
   confirmed live: harmless for sqlite (a local file has no
   "infrastructure" to be unreachable — `NoInfraBootTest` passed
   clean against `sqlite-notes.ciac` with a real `HikariPool`
   constructed, a real Flyway run, and a real `CREATE TABLE`, all
   inside the boot-test's own context refresh), but a genuine forward
   risk for postgres/mysql once M6/M7 unlock `crud-notes`/
   `mysql-notes` — named here for that milestone to resolve with live
   evidence rather than silently inherited as an unexamined gap.

   **Live proof, sqlite-notes.ciac end to end (the only M2-reachable
   example — `crud-notes.ciac` needs `auth JWT`/`cache Redis`,
   `mysql-notes.ciac` needs `cache Redis`, neither of which Java
   supports before M6/M7, so both stay `CIAC0011`-refused exactly as
   before; the plan's own text anticipated "crud/mysql static-local"
   generation, but the actual gate keeps them fully refused, matching
   Go's own M2 precedent more strictly than the plan's optimistic
   read — recorded as the deviation it is):** `./mvnw -q -B verify`
   green (compile, Spotless, `NoInfraBootTest`); a full CRUD lifecycle
   against a real sqlite file via `spring-boot:run` — create (201,
   server-generated UUID echoed back), get (200), list (200), update
   (200, persisted), delete (204), a subsequent get (404 via the new
   `NotFoundException`); the zero-value/null boundary triple against
   real requests (missing `title` -> 400 `missing required field
   "title"`; explicit `"title":null` -> 400 `field "title" must not be
   null"`; `"title":""` -> 201, the legitimate zero value correctly
   accepted); data confirmed persisted on disk after the process
   exited (`data/sqlite_notes.db`, zero Docker throughout).

   **The rename-replay proof this milestone's own text calls the
   "most load-bearing single test"** — a new
   `out_replay_resolves_the_java_target_migrations_dir` in
   `crates/ciac/tests/rename_cli.rs`, modeled on TS's/Go's own M8
   replay tests but the first to exercise a genuinely *non-identity*
   `migration_filename` transformation rather than an identity one:
   builds a `table`-declaring program, confirms the migration lands
   at `V0001__migration.sql` (not `0001_migration.sql`), replays a
   field rename through `--out`, and confirms both the renamed
   schema file and the Flyway-transformed migration path survive the
   regeneration untouched. Passed on the first run.

   Full workspace verification: fmt/clippy clean, `cargo test
   --workspace` green (65 suites, zero failures) across three full
   passes (the middle one surfaced the one real gap below); two new
   golden snapshots (`gen__java__sqlite-notes`, and `gen__java__ping`
   picking up `NotFoundException`'s now-unconditional emission).

   **Real bug found live — the keyed CRUD path would have shipped
   with a dangling `Schemas` reference.** `Schemas.java`'s own
   emission was still gated on `!ctx.records.is_empty()` from M1 — a
   keyed `crud <Name>;` (no backing `record`) needs `Schemas.MAPPER`/
   `requireKeys` too but contributes nothing to `ctx.records`, so a
   keyed-only program would have generated code referencing a file
   that was never written. Fixed by widening the gate to `
   !ctx.records.is_empty() || !ctx.resources.is_empty()`. Not caught
   by any currently-reachable example (`sqlite-notes.ciac`/
   `mysql-notes.ciac` are both typed; `crud-notes.ciac`, the one
   keyed example, is auth/cache-gated) — found by reading the keyed
   branch's own template against what M1 actually gated, not by a
   failing build.

   **Second real bug found live — the classpath-resource `openapi.json`
   collision Go's own M1 already hit had a second latent form here:**
   caught and fixed at M1 itself (renamed to `apidoc.json`); recorded
   again here only because M2 was the first milestone to re-run C3
   against a second Java example (`sqlite-notes.ciac`) and confirm the
   fix generalizes, not because a new instance of the bug appeared.
3. **M3 — Broker, workers, jobs, channels.** jnats + spring-kafka,
   retry + public `handleMessageOnce`, `@Scheduled` with the
   prefix-only `spring_cron` filter (equivalence-tested), WS/SSE.
   The four broker/schedule examples verify.

   **Shipped (v0.25 M3):** `supports()` gained one wide OR-chain —
   `Component::Queue`/`Stream`/`Worker`/`Job`/`Channel`/`Scheduler`/
   `Realtime` — the same "engine-agnostic component, per-engine
   branch stays inside the template" shape M2 already established
   for `Database`. `events <Name>;` needed no separate gate, same as
   every earlier target: it lowers to the same `Component::Worker`
   node a plain `worker` declaration does, split into `ConsumerCtx`
   only at the codegen model layer.

   New templates: `_steps.java.j2` (shared macro, `{% import %}`-ed
   by `ApiController`/`Worker`/`Job`, simpler than Go's own equivalent
   since Java has real exceptions — no `if v, err := ..; err != nil`
   dance needed); `Service.java.j2` (seeded `@Component` stub for
   classic, `crud`-free handlers — `ctx.services`, genuinely new at
   M3); `Queue.java.j2` (standalone `@Component`, not one of
   `AppState`'s own `@Bean` factories — see the design note below);
   `Worker.java.j2`/`Consumer.java.j2` (jnats `Dispatcher` or
   `@KafkaListener`, retry loop, public `handleMessageOnce`);
   `Job.java.j2` (`@Scheduled`, public `handleTickOnce`, `CATCH_UP`
   constant); `Channel.java.j2` (`TextWebSocketHandler`+
   `WebSocketConfigurer` or `SseEmitter`, NATS-only this milestone —
   a Kafka channel needs a fresh consumer group per connection, which
   `@KafkaListener`'s declarative model can't express, Go's own
   precedent for the identical reason; no M3 example combines `queue
   Kafka` with a `channel`, so this is deferred rather than built
   against nothing reachable, disclosed in the template itself).
   Modified: `ApiController.java.j2` (rewritten to loop `api.steps`
   through the shared macro instead of the M1/M2 hardcoded
   decode-and-return shape); `Application.java.j2` (conditional
   `@EnableScheduling`/`@EnableWebSocket`); `pom.xml.j2` (jnats,
   spring-kafka, spring-boot-starter-websocket, all gated).

   **The AppState/Queue self-reference problem, found by trying the
   obvious shape first, not by reading ahead:** `Queue` was initially
   sketched as one of `AppState`'s own `@Bean` factory methods
   (matching `DataSource`/`JdbcClient`'s own M2 pattern) — but a
   `@Configuration` class cannot inject a bean its own `@Bean` method
   produces, which every consumer site (`Worker`/`Consumer`/`Channel`,
   plus `ApiController`/`Job` when they publish) would have needed to
   reach through `state.getQueue()`. Fixed by making `Queue` a
   standalone `@Component`, constructor-injected directly into every
   site instead of routed through `AppState` — a real design
   correction mid-milestone, not a hedge chosen up front.

   **Kafka's official Java client carries Go's own franz-go risk,
   applied proactively rather than rediscovered:** Go's own
   24UpdatePlan.md M3 found, via `goleak` against a real unreachable
   broker, that franz-go's client starts supervisory goroutines
   immediately on construction despite never dialing. The official
   `org.apache.kafka.clients.producer.KafkaProducer` carries the
   analogous risk (a background sender thread starts at construction),
   so `Queue`'s Kafka branch is guarded the identical way NATS's own
   `Connection` already has to be (`synchronized producer()`/
   `synchronized connection()`, connect-on-first-use) — reasoned
   through from Go's own finding before writing the code, not
   discovered by a failing test here (no local broker to fail against).

   **`spring_cron` is a literal `"0 "` prefix, nothing else — verified
   live, not trusted from the plan's own claim:** Spring's
   `CronExpression` is six-field, seconds-first, but (unlike Rust's own
   `cron` crate) accepts CIaC's weekday `0`-`7` convention natively; a
   standalone `CronExpression.parse("0 0 3 * * 0")`/`"0 0 3 * * 7"`
   both parsed as Sunday before the filter was written this simply.
   Equivalence-tested in `tests/tests/cron_vectors.rs` against the same
   `VALID_SCHEDULES` fixture the Rust-crate equivalence test already
   uses.

   **Five real bugs, found only by live-generating and building the
   four target examples plus the newly-reachable `audited-crud.ciac`,
   not by inspection:**
   1. `Queue.java.j2`'s NATS branch imported `io.nats.client.NatsMessage`
      — the real class lives in `io.nats.client.impl.NatsMessage`.
      Caught immediately by `./mvnw compile` on `event-pipeline.ciac`
      ("cannot find symbol"), fixed with the correct import.
   2. `Channel.java.j2` only imported `java.io.IOException` on the SSE
      branch, but both branches catch it — the websocket branch's own
      `afterConnectionEstablished` would have failed to compile the
      moment `realtime-progress.ciac` (M3's first websocket-channel
      example) tried it. Fixed by hoisting the import out of the
      `{% if %}` so both branches get it.
   3. **`schemas.go.j2`'s own M3 finding recurred here, independently,
      not copied from reading it:** no template ever emitted a
      *declaration* for a record's inline-`enum` field's named type —
      `filters::java_type_of` already returned the bare `VideoStatus`
      name (correct *within* `schemas`, where every sibling type is
      already in scope), but nothing wrote `public enum VideoStatus {
      Ready, Failed }` anywhere. `realtime-progress.ciac` (M3's first
      example with an inline-enum record field, exactly the same
      example that tripped Go's own identical gap at its own M3) failed
      `./mvnw compile` with "cannot find symbol: class VideoStatus".
      Fixed with a new `RecordEnum.java.j2` template, one file per
      `record.enums` entry, emitted alongside each record — Jackson
      serializes/deserializes an enum by its constant name by default,
      and every variant identifier is spelled exactly as the source
      declared it, so no `@JsonProperty` is needed for the wire shape
      to match (mirrors Go's own string-enum-type answer, Java's own
      idiomatic equivalent of the same closed set).
   4. **A latent M2 gap, invisible until M3's own live-postgres proof
      first exercised it:** Flyway 10 split per-database support out
      of `flyway-core` — only H2/SQLite still detect with core alone;
      Postgres/MySQL need the separate `flyway-database-postgresql`/
      `flyway-mysql` artifacts. M2's own live proof only ever ran
      Flyway against SQLite (`crud-notes.ciac`/`mysql-notes.ciac` were
      both `CIAC0011`-refused at M2), so this never had a chance to
      fail until `scheduled-cleanup.ciac` became M3's first
      Postgres-backed example actually booted against a real local
      Postgres: `migrateOnBoot` threw `FlywayException: Unsupported
      Database: PostgreSQL 16.13`. Fixed by adding both artifacts
      (BOM-managed, no explicit version) gated on `has_postgres_db`/
      `has_mysql_db` respectively — the MySQL half disclosed as
      unverified live (no local MySQL in this sandbox), added
      proactively once the Postgres half's own pattern was confirmed.
   5. **The most consequential live find this milestone — NATS's own
      `@EventListener(ApplicationReadyEvent.class)` boot-time
      subscribe was fatal, not merely a disclosed risk:** reasoned
      through as a forward risk in the first draft (matching M2's own
      Flyway `CommandLineRunner` disclosure), then actually tested by
      running `NoInfraBootTest` against `event-pipeline.ciac` with no
      NATS server reachable — and it genuinely failed:
      `IOException: Unable to connect to NATS servers` propagating out
      of an `@EventListener` method aborts Spring's own event
      multicaster, which aborts context startup entirely. This is
      *not* the same failure class as Kafka's own `@KafkaListener`
      (confirmed by the same live test against `kafka-pipeline.ciac`
      with no Kafka broker either: its listener container polls on a
      background thread, an unreachable broker blocks that thread, not
      context refresh, so boot succeeds). Fixed by wrapping
      `Worker`/`Consumer`'s NATS `start()` body in a try/catch that
      logs and returns instead of throwing — restoring the same
      graceful-degradation behavior Go's own goroutine-based consumer
      already has (the dial failure is caught and logged inside the
      goroutine, never propagated to `main`), rather than taking the
      whole app down over a broker outage. Re-ran `NoInfraBootTest`
      against `event-pipeline.ciac` after the fix: green.
   6. **A sixth, structural bug — not in a template, in the
      conformance harness's own newly-widened reach:** M3's wider
      `supports()` gate is what first makes the multi-service
      `audited-crud.ciac` reachable for Java at all (M1/M2's narrower
      gate never had one in the harness's supported set). `generate()`
      never wrote a root-level combined `openapi.json` index for
      multi-service systems — only each service's own — the exact bug
      Go's own M3 already found and fixed for itself, just never
      ported to Java's parallel code path. Caught by C3's own
      byte-identical-path-set check (`python: [accounts/openapi.json,
      catalog/openapi.json, openapi.json]` vs `java:
      [accounts/openapi.json, catalog/openapi.json]`). Fixed by adding
      the same `ciac_codegen::openapi::build_index(&model)` write
      Go's own `generate()` already has, at the same `model.multi`
      site.

   **Live-verified end to end, not just golden-generated,** all four
   target examples (`./mvnw -q -B verify` clean, Spotless/
   `NoInfraBootTest` included) plus the newly-reachable
   `audited-crud.ciac`:
   - `scheduled-cleanup.ciac` (jobs-only, Postgres): full `mvn verify`
     green against a real local Postgres (apt-installed, no Docker) —
     `NoInfraBootTest` passes with a real `HikariPool` constructed, a
     real Flyway run (bug 4's fix, live), and the app boots via
     `spring-boot:run` with `/health` responding. **The seam-import
     proof this milestone's own exit checklist names** — a throwaway
     JUnit test, not committed, directly `new`-ing `PruneExpired` and
     `CleanupJob` (no Spring context at all) and calling
     `handleTickOnce()` against the same real Postgres — passed clean,
     the identical "seam a future simulation runner drives directly"
     proof Go's own M3 ran for its own `HandleTickCleanupOnce`.
   - `event-pipeline.ciac` (Postgres + NATS): full `mvn verify` green;
     a live `spring-boot:run` round-trip — `POST /submit` with no NATS
     broker reachable decodes the body, runs the `Validate` step,
     attempts the publish, and returns a clean `{"status":"error",
     "data":"internal server error"}` via `ErrorAdvice`'s generic
     handler rather than crashing the process (bug 5's fix, confirmed
     live against a running binary, not just `NoInfraBootTest`).
   - `kafka-pipeline.ciac` (no db, Kafka): full `mvn verify` green with
     zero local Kafka broker — the live log confirms the disclosed
     reasoning for bug 5 empirically: `NetworkClient` retries on a
     background `kafka-1` thread, never blocking `NoInfraBootTest`'s
     own context refresh.
   - `realtime-progress.ciac` (no db, NATS, websocket channel): full
     `mvn verify` green — `Channel` never dials NATS at boot (only
     `afterConnectionEstablished`, per real connection), so it was
     never exposed to bug 5's risk class at all; the enum fix (bug 3)
     is what let this example compile in the first place.
   - `audited-crud.ciac` (multi-service, newly reachable): C3 green
     after bug 6's fix; both `accounts/` and `catalog/` generate their
     own `openapi.json`, plus the new root index.

   Full workspace verification: `cargo fmt`/`clippy -D warnings` clean
   three times; `cargo test --workspace` green three times (the first
   pass caught a stale M1/M2-era unit test —
   `tests::supports_apis`'s own `assert!(!backend.supports(&Component
   ::Queue{..}))`, obsolete the moment M3 widened the gate — replaced
   with `supports_broker_workers_jobs_channels_at_m3`, mirroring Go's
   own M3 test of the identical name; the second pass caught bug 6
   above via C3; the third pass was clean). Six new/updated golden
   snapshots (`ping` picking up the `ApiController` rewrite's uniform
   `throws Exception`; `event-pipeline`/`kafka-pipeline`/
   `scheduled-cleanup`/`realtime-progress`/`audited-crud` new), each
   reviewed before accepting, not blanket-accepted, matching Go's own
   M3 discipline.

4. **M4 — Typed handlers: `HostSyntax` for Java.** All verbs per
   Pillar 5's table, TransactionTemplate atomicity, JsonNode paths
   with the missing-path check, switch-expression match where fixed,
   builtins. No contract amendment expected (Pillar 2's unchecked-
   exception decision); if falsified, goldens-first per the standing
   procedure. typed-handlers/typed-video/domain-orders/query-verbs/
   extras-verbs verify; equivalence test → five targets.

   **Shipped (v0.25 M4):** `supports()` widened once more — the
   `Component::Service{signature: None, ..}` arm folded into a bare
   `Component::Service{..}`, so both classic and typed handlers now
   build — and a new `crates/ciac-backend-java/src/lower.rs`
   (~600 lines) implements every `HostSyntax` leaf for Java. Pillar 2's
   own unchecked-exception decision held exactly as predicted: **no
   error-idiom amendment override needed** — every "simple verb" leaf
   (`db_get`/`cache_*`/`object_store_*`/`email_send`/`search_*`/
   `http_call`) is a plain scalar expression, so `HostSyntax`'s own
   default `..._tail` wrappers apply `Dest` unchanged, the same shape
   Python already has (zero of Go's own `fallible_tail`/two-return-
   value machinery). `TransactionTemplate.executeWithoutResult`, not
   `@Transactional`, wraps a `transaction {}` block's body — chosen
   specifically to dodge the proxy self-invocation trap a same-class
   `@Transactional` call would hit silently; every db verb inside
   ignores its own `in_tx` flag and issues SQL through the same
   `JdbcClient` bean regardless, which participates in the ambient
   transaction transparently via Spring's own `DataSourceUtils`
   connection binding — a real, disclosed simplification over Go's
   explicit dual `*sql.Tx`/pool-handle scheme, since JDBC has no
   separate "transaction handle" type at all. `record_cons` zips the
   raw HIR record's surface field names against `context::build_record`'s
   own Java-facing (Reference-renamed) field list in lockstep by
   position to resolve both the lookup key and the fallback accessor
   name correctly. `RowMappers.java.j2` now takes a single, Rust-
   deduplicated `row_mapper_records` list covering both CRUD resources
   and `table` declarations (previously resource-only). Java 21
   arrow-case `switch` on bare, unqualified enum constants lowers
   `match` more simply than feared going in — no string-conversion of
   the scrutinee needed at all, Go's own "no explicit break" simplicity
   without even Go's string-keyed dispatch. Jackson's `Schemas.toJson`/
   `Schemas.fromJsonOrNull` serialize a Record, a `JsonNode`, or a
   boxed scalar uniformly through one call, eliminating the 3-way
   Record/Json/scalar branch every other backend's own `json_body`
   needs. All handler classes — classic and typed alike — expose one
   `handle` method, matching both the pre-existing classic-handler
   convention and the plan's own worked example, so `_steps.java.j2`
   needed no second call-shape branch, only a `payload_type`-typed
   cast for the typed case. `domain-orders.ciac`/`query-verbs.ciac`
   (db-only) are this milestone's reachable proving examples, per M1–
   M3's own `typed-handlers`/`typed-video`/`extras-verbs`-stay-refused
   precedent (they need `object_store`/`cache`/`auth`, still gated to
   M6/M7).

   **Eight real bugs, every one found only by live-generating,
   compiling, and running the actual examples against real
   Postgres/SQLite — none by inspection:**
   1. **A latent M2 bug, invisible until a Postgres-bound `table` was
      first live-tested this milestone:** `ResourceStore.java.j2` was
      converting placeholders via `sqlph(sql, resource.db_engine)`,
      which leaves literal `$1`/`$2` in the SQL text for a
      Postgres-bound resource — invalid for JDBC's `PreparedStatement`,
      which always uses `?` regardless of engine (Postgres's own `$N`
      is a libpq/psql-only convention, never part of the JDBC
      placeholder grammar any driver implements). Never caught before
      because M2's only live-tested example was SQLite (already
      `?`-style by coincidence). Fixed by adding
      `filters::jdbcph` (unconditional `?` conversion) and replacing
      every `sqlph(sql, resource.db_engine)` call site.
   2. **The `__row0;` invalid-statement bug, found via a
      `google-java-format` rejection on `domain-orders.ciac`'s first
      live generation:** `db_insert_tail`/`db_update_tail`/
      `db_delete_tail`/`query_tail`'s three arms all called the
      generic `apply_dest(self, dest, &value, ..)` unconditionally —
      correct for `Dest::Assign`/`Dest::Return`, but for
      `Dest::Discard` this emitted a bare local-variable-name
      statement (`__row0;`), which Java's `ExpressionStatement`
      grammar rejects outright (only Assignment/Increment/Decrement/
      MethodInvocation/ClassInstanceCreation qualify — unlike Go's
      `_ = x;` or Python's fully general expression-statement
      grammar). Fixed by special-casing `Dest::Discard` at all 6 call
      sites to emit no trailing line at all — safe because, unlike
      Go, Java has no "declared and not used" hard compile error.
   3. **A record.java.j2 whitespace bug, found via a `spotless:check`
      failure on `InvalidOrder.java` (the first `error` record ever
      live-verified for Java — no earlier milestone's examples had
      one):** the unconditional `import
      com.fasterxml.jackson.databind.JsonNode;` line, unused by the
      `is_error` branch, was silently stripped by `google-java-format`
      during generation but left a double blank line behind — `mvn
      verify`'s own independent `spotless-maven-plugin` invocation of
      google-java-format is stricter than the compiler and caught
      what the generation-time formatter didn't. Fixed by moving the
      import into the non-error `else` branch only, where it's
      actually used.
   4. **A `logic.java.j2` missing-import bug, found via a compile
      failure once a handler outside the `schemas` package first
      called a `Schemas.` static helper (`indexOrThrow`/`toJson`) —
      unexercised by `domain-orders.ciac`/`query-verbs.ciac`, which
      never index `Json` or serialize a payload:** every generated
      `logic`/`services` class is in a different package from
      `Schemas` itself, so a bare `Schemas.foo(..)` call needs an
      explicit cross-package import the template never emitted.
      Fixed by adding an unconditional `import ...schemas.Schemas;`
      to `logic.java.j2` (stripped by `google-java-format` when
      genuinely unused, so this is safe for every handler regardless
      of whether it touches `Schemas`).
   5. **A `RowMappers.java.j2` type-mismatch bug, found via a compile
      failure the moment a `table`-backed record's own `Json` field
      was first read back (`rs.getString(..)` returning `String`
      where the record component's Java type is `JsonNode`):** the
      template's own doc comment already disclosed this exact gap as
      "isn't exercised by any reachable example yet." Fixed for real
      (not merely left disclosed) by adding a `Schemas.
      fromJsonOrNull(rs.getString(..))` branch, gated on the
      pre-existing `FieldCtx.is_json` flag `ciac-codegen::model`
      already tracks (reused directly — no parallel `java_is_json`
      filter needed, an unnecessary duplication caught and reverted
      mid-fix).
   6. **A `bind_expr` bug, found live against a real Postgres
      `jsonb` column:** binding a `Json` field's raw record accessor
      (a `JsonNode`/`ObjectNode`) directly threw
      `IllegalArgumentException: Invalid positional parameter value of
      type Iterable (ObjectNode): Parameter expansion is only
      supported with named parameters` — `JsonNode` implements
      `Iterable<Map.Entry<String,JsonNode>>`, so `JdbcClient`'s own
      `.param(..)` mistook it for an IN-clause expansion candidate
      rather than a single scalar bind value. Fixed by serializing
      every `Json` field's bind expression through `Schemas.toJson(..)`
      first, binding a plain `String`.
   7. **A Postgres `jsonb` column-type bug, found immediately after
      fixing bug 6 — the very next live request:** even a correctly
      bound `String` value is rejected outright by a `jsonb` column
      (`column "extra" is of type jsonb but expression is of type
      character varying") unless the placeholder itself carries an
      explicit `::jsonb` cast — a JDBC-specific trap none of the
      other four targets' own drivers have (each already knows the
      bound value's JSON-ness from its own client-side type wrapper).
      Fixed by threading a new `table_db_engine`/`field_placeholder`
      pair through `db_insert_tail`/`db_update_tail`, replacing
      reliance on the shared, engine-agnostic `RecordCtx::
      insert_placeholders`/`update_assignments` strings with Java's
      own per-field, engine-conditional placeholder text (`?::jsonb`
      only for a `Json` field on `DbEngine::Postgres`; plain `?`
      everywhere else, matching MySQL's/SQLite's own JSON-as-text
      columns, which need no cast).
   8. **A conformance-harness bug in the cross-target contract itself,
      not in a template — found by `c4b_declared_topology_appears_in_
      every_target` the moment `domain-orders.ciac` became Java-
      reachable for the first time this milestone:** every other
      target names some generated identifier or doc comment literally
      after a table's own declared PascalCase name (Go's `type
      Customers struct`, Python's `class Customers(Base)`) — Java
      names everything after the singular *record* instead
      (`RowMappers.CUSTOMER`, `record Customer(...)`), so the literal
      string `"Customers"` never appeared anywhere in Java's own
      output, failing the shared cross-target "every declared
      topology fact appears somewhere" check. Fixed cosmetically, not
      behaviorally: `lib.rs`'s `row_mapper_records` now carries each
      table's own `class_name` alongside its `RecordCtx` (a new
      `RowMapperEntry` struct, `table_name: Option<String>`), and
      `RowMappers.java.j2` emits a one-line doc comment naming the
      table declaration directly above its row-mapper constant.

   **Live-verified end to end against real infrastructure, not just
   golden-generated or structurally asserted:**
   - `domain-orders.ciac` (Postgres, `transaction {}`): `./mvnw -q -B
     verify` green against a real local Postgres (apt-installed, no
     Docker) — Flyway migration, `NoInfraBootTest`, Spotless all
     clean. A live `spring-boot:run` round-trip proved the rollback
     contract for real: `POST /customers` then a valid `POST /orders`
     committed both the `orders` and `order_audits` rows inside
     `PlaceOrder`'s `transaction {}` block; a second `POST /orders`
     with a negative total threw `InvalidOrder`, returned HTTP 500,
     and left the database at exactly the same row counts as before
     the failed call — the transaction genuinely rolled back both
     inserts together, not just the one that failed.
   - `query-verbs.ciac` (SQLite, zero Docker): `./mvnw -q -B verify`
     green with no container at all. A live round-trip exercised the
     full extended db verb set over real HTTP: `db.query`/`db.count`
     with `where` predicates (`ListActive`/`CountActive` against
     hand-seeded rows), `db.update` (`Replace`, confirmed via a
     before/after `count active` delta), `db.delete` (`Remove`,
     confirmed both the found-and-deleted `true` case and the
     not-found `false` case), and `db.delete_where` (`DeleteByActive`,
     confirmed the deleted-row count matched and a follow-up `list`
     returned the correct remainder).
   - A throwaway `division-example` fixture (not committed — the seed
     for bugs 4/5/6/7 above, and now also the `DIVISION_EXAMPLE`
     equivalence-test addition below) proved the `Json`-field
     round-trip for real: `POST /compute` with `extra: {"label":
     "widget", "qty": 3}` stored and read back the identical JSON via
     a real `jsonb` column, and `Int / Int` truncated toward zero
     exactly like Go's/Rust's own native `/` (`17 / 5 = 3`).

   **Tests:** `tests/tests/typed_handler_equivalence.rs`'s
   `DIVISION_EXAMPLE` test extends from four targets to five —
   `python_rust_typescript_go_and_java_lower_the_same_handler_body_to_
   equivalent_shape` — pinning Java's own two divergence points: `Int
   / Int` needs no `Math.trunc`-style special case (Java's `long /
   long` already truncates toward zero, the same simplification Go's
   own leaf gets), and `Json` indexing throws via
   `Schemas.indexOrThrow` with the identical `KeyError: '<key>'`
   message text every other target's own leaf carries — plus a
   Java-specific assertion pinning the `?::jsonb` cast bug 7 fixed,
   the one placeholder wrinkle none of the other four targets need.

   Full workspace verification: `cargo fmt`/`clippy -D warnings`
   clean; `cargo test --workspace` green (including all five
   `conformance.rs` cross-target checks, `c4b`'s own topology check
   included after bug 8's fix). Golden snapshots regenerated and
   individually reviewed for eleven Java trees (the RowMappers
   rename/dedup fix, the `jdbcph` placeholder fix, and the new
   `TransactionTemplate` bean all touch every Java golden, not just
   this milestone's own new examples) — `audited-crud.ciac` picked up
   the M3-era `RowMappers.RESOURCE_VIDEO` → `RowMappers.VIDEO` rename
   as part of this same regeneration, confirming M3's own dedup-by-
   record-name fix was already correct, just never re-snapshotted
   until now.

5. **M5 — CHECKPOINT.** The factory's final grade: measured cost vs
   the twice-updated model; conformance harness green across five
   targets (OpenAPI byte-equality ×5, topology, boundary decode);
   the Pillar 8 latency measurement and the pre-agreed CI-scoping
   decision taken from data. Go/no-go for the remainder.

   **Shipped (v0.25 M5) — the measured cost table**, against
   `24UpdatePlan.md` M5's own Go actuals (M1–M4, the same milestone
   marker Java has just reached) as the primary baseline, plus
   Rust's/Python's mature full-arc figures and TS's own M1–M4 actuals
   for context exactly as Go's own table did:

   | | Rust (mature, full arc) | Python (mature, full arc) | TypeScript (M1–M4) | Go (M1–M4) | Java (M1–M4) |
   | --- | --- | --- | --- | --- | --- |
   | `lower.rs` (leaves + `render`) | 607 | 932 | 1,098 | 1,296 | **1,031** |
   | `lib.rs` (emission wiring) | 559 | 429 | 501 | 710 | **788** |
   | `filters.rs` (neutral-field mapping) | n/a | n/a | 206 | 139 | **139** |
   | templates | ~2,800 | ~2,800 | 5,608 across 28 files | 2,176 across 25 files | **1,824 across 31 files** |
   | edits outside the crate | 1 (registry line) | 1 (registry line) | 1 (`commands.rs`) | 1 + 2 disclosed amendments | **1** (`commands.rs`, single-line registration) **+ 0 amendments** |

   **`lower.rs`: the lowest of the four fully-typed-verb targets
   measured at this marker, confirming Pillar 2's own prediction
   rather than merely repeating it.** Java's 1,031 lines sit below
   TS's 1,098 and well below Go's 1,296 — the exact gap Pillar 2's
   unchecked-exception decision predicted going into M4: Go alone
   needed the error-idiom amendment (`if err != nil` blocks, a
   closure-wrapped `fallible_tail` for `db.update`); Python/TS/Java
   all propagate failure through exceptions and needed zero
   `HostSyntax` contract changes. That Java still lands below TS's
   own exception-propagating figure, not merely near it, is a second,
   independent factor: JDBC's single placeholder story (unconditional
   `?`, no `$N`/`?`-family branch at all, sharpened further at M4 once
   `jdbcph` replaced the shared `RecordCtx::insert_placeholders`/
   `update_assignments` strings for the two write-verb tails that
   needed engine-conditional `::jsonb` casts) needs no per-engine
   `sqlph`-style dispatch inside `lower.rs` itself the way TS's three
   structurally different driver APis do. Slightly above Python's 932
   and Rust's 607: Java's own `record_cons` needs the raw-surface/
   Java-facing field-list zip (Reference-renaming resolution) neither
   Python's kwargs-splat nor Rust's field-by-field struct literal
   needs, and the `TransactionTemplate`/`collect_branching_lets`
   block-scoping machinery Go's own `lower.rs` doc already named as a
   real, shared Java/Go cost (`var`-declared locals inside `if`/
   `switch` blocks don't escape their block in either language).

   **`lib.rs`: a real, disclosed overrun — the highest of the five —
   with a concrete, non-idiom cause.** Java's 788 lines exceed even
   Go's own 710 (which M4's own retro already flagged as a 42% premium
   over TS's 501). Two factors, neither about verb-lowering
   difficulty: (1) **file count.** Java emits 31 distinct templates
   at this milestone marker against Go's 25 and TS's 28 — each
   `project.add_file(..., render_java("X.java.j2", ...))` call is a
   few lines of wiring on its own, and Spring's own idiom (one
   `@Component`/`@Configuration` class per concern — `RowMappers`,
   `DataSources`, `Schemas`, `Envelope`, `ErrorAdvice`, two exception
   base classes, `AppState`, `Queue`, per-resource `In`/`Entity`/
   `Store`/`Controller` quartets — split more finely than Go's own
   fewer, denser files) multiplies that cost more than any other
   target's own file layout does. (2) **the `google-java-format`
   subprocess pipeline** (vendored-jar materialization,
   stdin/stdout piping, exit-code handling, the `CIAC_DEBUG_JAVA_SRC`
   diagnostic hook this milestone's own bug-hunting needed and kept)
   is roughly 50 lines of wiring no other target's `lib.rs` carries
   at all — Python/TS/Go/Rust format at the template-authoring level
   (consistent indentation baked into the `.j2` files themselves,
   `rustfmt`/`gofmt`/`prettier` are non-blocking dev-time tools, never
   invoked from inside `generate()`); Java's own commitment to a
   canonical, Spotless-asserted format (Pillar 5) means the formatter
   is a hard dependency of `generate()` itself, priced directly into
   `lib.rs`.

   **Templates: a second confirmation of Go's own M5 retro, not a
   contradiction of it.** Go's checkpoint found that a target whose
   database layer has one unifying driver interface (Go's
   `database/sql`, identical method sets across drivers) lands far
   below TS's per-engine-branching figure. JDBC is an even more
   direct instance of the same lens — `PreparedStatement`/`ResultSet`
   are the *standard library* interface every JDBC driver implements
   identically, not merely a popular convention the way Go's
   `database/sql` is — and Java's 1,824 lines across 31 files lands
   below even Go's own 2,176/25, despite six *more* files, each
   individually smaller (Spring's own per-concern class-splitting
   convention, the same factor `lib.rs`'s own overrun traces to,
   nets out favorably here: more files, but each one is a thinner,
   more repetitive shape — a `RowMapper` constant, a `JdbcClient`
   call chain — than Go's own denser per-file logic).

   **Zero shared-crate amendments — the cleanest factory-fidelity
   result of any target measured at this marker.** `git diff --stat
   4c7d9be^..20cfa33 -- crates/ciac-codegen crates/ciac-ir
   crates/ciac-sema crates/ciac-syntax` returns nothing: the factory
   held for Java's entire M1–M4 arc with no `HostSyntax`/`Needs`/
   `RecordCtx` changes at all — better than Go's own precedent (two
   disclosed, narrow amendments: the error-idiom amendment and
   `HandlerRef::is_typed_handler`), matching Python's/Rust's/TS's own
   original "factory held without amendment" pattern. The "edits
   outside the crate" Java's own arc did touch —
   `Cargo.toml`/`Cargo.lock`/`crates/ciac/Cargo.toml`/`tests/Cargo.toml`
   (workspace member registration), `crates/ciac/src/commands.rs`
   (the single-line target registration, identical in shape to every
   prior target's own), `tests/src/lib.rs` (the `backends()` list),
   `docs/targets.json` (mechanical, test-enforced regeneration),
   `crates/ciac/tests/rename_cli.rs` (M2's own required rename-replay
   proof, 67 lines — routine per-target scaffolding, not a factory
   change), and `tests/tests/cron_vectors.rs`/`tests/tests/
   typed_handler_equivalence.rs` (the cron-schedule and typed-handler
   equivalence suites' own per-target extension, expected at M1 and
   M4 respectively) — are the same routine registration/test-extension
   set every prior target's own checkpoint already named, not
   additional factory surface.

   **Conformance harness, run for real across all five targets:**
   `cargo test --workspace` is green with Java included (confirmed
   this same milestone, immediately before writing this retrospective
   — not a stale claim); `tests/tests/conformance.rs`'s
   `c3_openapi_is_byte_identical_across_targets`,
   `c4a_migration_sql_is_byte_identical_across_targets`, and
   `c4b_declared_topology_appears_in_every_target` all pass — the
   last one only after M4's own bug 8 fix (a `RowMappers.java.j2` doc
   comment naming each table's own declared PascalCase name, since
   Java otherwise names everything after the singular record, not
   the table, everywhere in its own output). `tests/tests/golden.rs`
   is green across eleven Java trees. `ciac targets --json` lists
   `"id": "java"` alongside python/rust/typescript/go, confirmed
   live this same session. The boundary-case decode suite this
   checkpoint's own text names is M1's/M2's own live-verified
   absent/explicit-null/legitimate-zero triple (already passing, not
   new work this milestone).

   **Pillar 8 latency, re-measured at this milestone marker (not
   merely carried forward from M1):** M1's own ping-parity baseline
   (this sandbox, proxied Maven Central, `~/.m2` not genuinely
   empty) was ~10.6s cold (only `com.ciac` artifacts cleared) / ~6.3s
   warm. Re-measured against `domain-orders.ciac` — M4's own larger,
   Postgres-backed, `transaction {}`-bearing example, a real
   `./mvnw -q -B verify` against a live local Postgres, not the
   ping-parity project's own zero-dependency shape — after clearing
   `~/.m2/repository/com/ciac`: **9.8s**, and **9.9s** on an immediate
   warm re-run. Materially flat against M1's own figure despite a
   genuinely larger, infrastructure-backed project: the "startup tax
   paid once" mitigation (Pillar 8 #2, one `./mvnw -q -B verify`
   invocation covering compile + Spotless + tests) is holding as
   designed, not degrading as the example matrix grows in complexity.

   **The pre-agreed CI-scoping decision, taken from this data:** the
   26-example matrix does *not* blow the CI budget at this milestone's
   measurement — ~10s per project, dominated by JVM+Maven-plugin
   startup rather than compile/test volume, means even the full
   matrix stays comfortably inside typical CI job budgets (the same
   conclusion Go's own `generated-system` job already reached for a
   comparable per-project cost). Decision: `generated-java` runs the
   *full* example matrix, unscoped — the narrowing clause in Pillar 8
   #5 is not exercised this arc, disclosed here as a decision taken
   from real numbers, not deferred or improvised later.

   **Go/no-go verdict: GO.** `lower.rs` came in below every other
   fully-typed-verb target measured at this marker, confirming
   Pillar 2's own unchecked-exception prediction with real numbers,
   not just repeating it; `lib.rs`'s real overrun has a concrete,
   disclosed, Spring-idiom-shaped cause (file-count multiplication
   plus the formatter-subprocess pipeline), not a capability or
   correctness gap; templates land favorably for the identical
   unifying-driver reason Go's own M5 retro named; the factory needed
   *zero* amendments across the entire M1–M4 arc, a cleaner result
   than Go's own two-amendment precedent; and Pillar 8's own latency
   budget holds flat under a materially larger, infrastructure-backed
   example than M1's own baseline. Nothing measured here is a
   structural blocker. `25UpdatePlan.md`'s remaining milestones
   (M6–M9) proceed without pausing to amend the factory further.

6. **M6 — Auth, scopes, scope tests.** Resource-server both modes,
   requireScope, MockMvc `ScopeTests` green under zero
   infrastructure; order-system and oauth-echo verify.

   **Shipped (v0.25 M6):** `supports()` widened once more —
   `Component::Auth { .. }` (both JWT and OAuth2 through the same
   `spring-boot-starter-oauth2-resource-server` mechanism, per Pillar
   7's own table). New `SecurityConfig.java.j2` (`@Configuration`,
   emitted only when `c.has_auth`): a deliberately *permissive*
   `SecurityFilterChain` bean (`anyRequest().permitAll()`) plus a
   `JwtDecoder` bean (`NimbusJwtDecoder.withSecretKey` for JWT,
   `withJwkSetUri` for OAuth2). The permissive filter chain is load-
   bearing, not incidental: merely adding the resource-server starter
   as a Maven dependency triggers Spring Boot's own autoconfiguration
   of a *default* security filter chain that authenticates every
   request — exactly the framework-magic-versus-compiler-ownership
   tension this whole target's own design note opens with. Neutralizing
   it explicitly, then calling `Auth.verifyToken`/`requireScope`
   (new `Auth.java.j2`, a plain static-method class in `routes`)
   inline at the top of each generated route body, restores the
   identical "no middleware chain, every check is an explicit call
   the generated code makes" discipline every other target's own
   `auth.go`/`auth.py`/`auth.rs`/`auth.ts` already has —
   `NimbusJwtDecoder.withJwkSetUri`'s own JWKS fetch is lazy (first
   `decode()` call only), so constructing it never blocks boot, the
   same laziness bar every other provider client already clears.
   `ApiController.java.j2`/`ResourceController.java.j2` both gained a
   conditional `HttpServletRequest request` parameter and `JwtDecoder`
   constructor field, gated on `api.has_auth_step`/`resource.has_auth`
   respectively — a typed handler's own `Auth` pipeline step needed no
   change at all (it was already a silent no-op in `_steps.java.j2`'s
   `if`/`elif` chain, the identical shape Go's own `_steps.go.j2` has,
   confirmed by reading it before assuming). Two new exception types,
   `UnauthorizedException`(401)/`ForbiddenException`(403), matching
   the existing `BadRequestException`/`NotFoundException` pattern
   exactly — `ErrorAdvice.java.j2`'s own doc comment had already
   named "scoped-auth rejections (M6)" as a foreseen extension point
   before this milestone touched it.

   **A real, RFC-level finding, confirmed live before writing a
   single line of template code around it:** every other target's own
   `JWT_SECRET` default is the shared `"change-me"` (9 bytes). Nimbus
   JOSE — the resource-server starter's own JWT engine, wrapped by
   both `NimbusJwtDecoder` and the `ScopeTests`' own token-minting
   code — enforces RFC 7518 §3.2's HS256 minimum key length (256
   bits/32 bytes) at sign *and* verify time. A standalone Nimbus
   `MACSigner` test against the literal 9-byte `"change-me"` string
   confirmed this directly: `KeyLengthException: The secret length
   must be at least 256 bits` — a failure every other target's own
   JWT library (golang-jwt, PyJWT, jsonwebtoken, the `jsonwebtoken`
   Rust crate) silently accepts despite being equally insecure by the
   same spec, since none of them enforce it client-side. Fixed by
   giving Java's own `JWT_SECRET` a longer default
   (`"change-me-please-use-a-real-32-byte-secret-key"`, 46 bytes,
   confirmed signing successfully against the same standalone
   harness) — a real, disclosed, Java-specific deviation from the
   cross-target convention, not a stylistic choice, and found before
   it could silently break every JWT-scheme project's own boot.

   **A real bug, found only by the golden-snapshot suite's own
   cross-example diff, not by inspection or by either live-tested
   example:** `ErrorAdvice.java.j2`'s two new `@ExceptionHandler`s
   referenced `UnauthorizedException`/`ForbiddenException`
   unconditionally, but those two exception *source files* are only
   emitted when `ctx.has_auth` (mirroring every other conditional
   file in `lib.rs`) — meaning every non-auth Java project (the large
   majority: `domain-orders.ciac`, `query-verbs.ciac`, and eleven of
   the twelve Java goldens) would have failed to compile with "cannot
   find symbol: class UnauthorizedException" the moment this landed.
   Neither `oauth-echo.ciac` nor the `jwt-scope` scratch example
   caught this — both declare `auth`, so both `.java` files always
   existed for them. Caught instead by `cargo test --workspace`'s own
   golden-snapshot diff against `audited-crud.ciac` (auth-free),
   which showed the two new handlers appearing in a project that
   should never reference those classes at all. Fixed by wrapping
   both handlers in `{% if c.has_auth %}` inside `ErrorAdvice.java.j2`
   — `c` (the full `Ctx`) is always in scope for every `render`/
   `render_java` call regardless of the `extra` context passed at the
   call site, confirmed by reading the closure's own definition rather
   than assumed. Re-verified live after the fix: `domain-orders.ciac`/
   `query-verbs.ciac`/`oauth-echo.ciac`/`jwt-scope` all `./mvnw -q -B
   verify` clean; the `audited-crud` golden's own diff after the fix
   showed only the doc-comment rewording, no handler code — direct
   confirmation the gate now works.

   **Disclosed scope gap vs. the plan's literal text, the identical
   shape Go's own M6 already hit:** `order-system.ciac`, this
   milestone's own named verification target, also declares `cache
   Redis` (`Component::Cache`), which stays refused for Java until M7
   — unlike TS, where Cache landed at TS's own M2. `order-system`
   therefore still returns `CIAC0011` for Java this milestone
   (confirmed live, same reason: `backend java does not support cache
   default Redis`); it will verify for real once M7 lands Cache, as
   M7's own milestone text already names it. In its place:
   `oauth-echo.ciac` (cache-free, OAuth2-only, the plan's other named
   example) verifies for real — newly Java-reachable this milestone,
   its own golden is new, not modified — and a throwaway scratch
   example (not committed — `jwt-scope.ciac`: `db Postgres; auth
   JWT;` plus a scope-gated `crud Note` and a scope-gated `api
   PingApi`) supplied the live JWT+scope proof `order-system` would
   otherwise have provided.

   **Live-verified end to end, not just golden-generated or
   MockMvc-asserted:**
   - `oauth-echo.ciac`: `./mvnw -q -B verify` green (`NoInfraBootTest`
     boots cleanly with zero JWKS server reachable — the lazy
     `NimbusJwtDecoder.withJwkSetUri` bean never touches the network
     at construction, confirmed live, not merely by reading the
     javadoc). A live `spring-boot:run` round-trip: `/health` returns
     200 (the permissive `SecurityFilterChain` genuinely doesn't gate
     it); `POST /echo` with no `Authorization` header returns 401
     `{"data":"missing bearer token"}`; with a syntactically-invalid
     bearer token returns 401 `{"data":"invalid or expired
     token"}` — both via the real `JwtDecoder.decode` call attempting
     (and failing) against the configured, unreachable issuer, caught
     by `Auth.verifyToken`'s own `catch (JwtException e)`.
   - `jwt-scope` (scratch, not committed): `./mvnw -q -B verify` green
     against real local Postgres, including all six generated
     `ScopeTests` (three scope-gated routes × missing/present pairs)
     passing against the real Spring context via MockMvc. A live
     `spring-boot:run` round-trip with HS256 tokens minted externally
     (plain HMAC-SHA256, no library, against the same 46-byte secret)
     confirmed the full mechanism: `POST /notes` with the wrong scope
     → 403 `{"data":"missing required scope: notes:write"}`; with the
     correct scope → 201 and the created row; `GET /notes` with no
     token → 401; with the correct read scope → 200 with the list
     (including the row created moments earlier, still in the same
     database — direct confirmation the store itself works
     end-to-end, not just the auth gate).
   - `domain-orders.ciac`/`query-verbs.ciac` (both auth-free,
     M4's/M5's own examples): re-verified live after the
     `ErrorAdvice` fix specifically to confirm the conditional gate
     didn't regress anything M4/M5 already proved — both `./mvnw -q
     -B verify` clean.

   Full workspace verification: `cargo fmt`/`clippy -D warnings`
   clean; `cargo test --workspace` green (first pass caught the
   `ErrorAdvice` bug via the golden suite; second pass, after the
   fix, fully green). Twelve Java goldens reviewed and accepted —
   eleven updated (the conditional `ErrorAdvice` handlers touch every
   Java golden, auth or not, since the doc-comment rewording is
   unconditional even when the two handlers themselves are gated
   out) plus one genuinely new: `oauth-echo`.

7. **M7 — Ontology remainder + call clients + observability
   completion.** S3/mail/search wrappers, RestClient call clients,
   OTel end-to-end (five-target trace test), metrics endpoint.
   multi-service-media, inventory-system, ontology-growth,
   traced-checkout, dev-identity verify; `--system` CI rows
   (java × inventory-system, × mysql-notes, × sim-vertical-slice)
   with compose-build times recorded against the Pillar 8 ledger.

   **Shipped (v0.25 M7):** `supports()` widened a final time —
   `Component::Cache`/`ObjectStore`/`Email`/`Search`/`ExternalHttp`/
   `Metrics`/`Tracing`/`Users` all join the OR-chain (`Logging` stays
   out: unreachable by any checked-in example, the same "don't build
   what nothing exercises" discipline this whole arc has kept).

   **A real M4-era bug found before any new code was written:**
   `lower.rs`'s own `cache_field`/`object_store_field`/`email_field`/
   `search_field`/`http_field` (written at M4 as stand-ins, unexercised
   until this milestone) stored the *raw* Rust-style field name
   (`cache_media`, snake_case) and spelled it verbatim into the leaf
   expression — but every other Java field this codegen emits is
   `java_camel`-filtered at the template layer (`AppState.java.j2`'s
   own `{{ inst.state_field | java_camel }}`). Reading `AppState.java.j2`
   before writing a single wrapper class caught the mismatch: the
   fields these M4 leaves would call had never actually been declared
   under that name anywhere. Fixed by threading the pre-existing plain
   `java_camel(&str) -> String` helper (already used for
   `java_tx_field`) through all five leaf call sites in `lower.rs`,
   matching Go's own identical precedent (`go_pascal` applied at each
   use site in `GoSyntax`, not baked into the stored field once) —
   confirmed by reading Go's own `lower.rs` side by side before fixing
   Java's.

   **Wrapper classes** (`crates/ciac-backend-java/templates/`,
   package `com.ciac.<pkg>.state`, one shared class per capability
   *kind*, one `@Lazy` `AppState` bean per named *instance* — the same
   one-struct-many-beans shape Go's own `objectstore.ObjectStore`/
   `email.Email`/`search.Search`/`httpclients.ExternalHttp` already
   established):
   - `Cache`: no wrapper needed at all — `StringRedisTemplate` (Spring
     Data Redis) is a direct match for the M4 leaf's own
     `.opsForValue().get/set()`/`.delete()` shape.
     `LettuceConnectionFactory` is built from a hand-parsed `redis://`
     URL (`java.net.URI`, mirroring `DataSources.open`'s own userinfo-
     stripping for JDBC) since Spring Data Redis has no single-string-
     URL constructor; confirmed live (see below) that
     `LettuceConnectionFactory.afterPropertiesSet()` does not dial —
     Lettuce's own client only connects on first command, the same
     "parses the URL and returns immediately" contract `redis.NewClient`
     already carries for Go.
   - `ObjectStore.java.j2`: AWS SDK v2 `S3Client`, `forcePathStyle(true)`
     for MinIO compatibility, built lazily inside a `synchronized`
     getter (never at construction). `get()` catches `NoSuchKeyException`
     and returns `null` rather than throwing — the M4 leaf's own
     `field.get(key) == null ? null : ...` ternary already assumed this
     miss contract, so the wrapper honors it instead of the leaf being
     rewritten. `software.amazon.awssdk:s3` is the one dependency this
     whole milestone needed an explicit `<version>` pin for (verified
     live against a standalone scratch Maven project before writing the
     real template: Spring Boot's own dependency management does *not*
     cover the AWS SDK, unlike Micrometer/OpenTelemetry, which it does
     — `opentelemetry-exporter-otlp` needed no version at all,
     confirmed the same way).
   - `Email.java.j2`: Spring's `JavaMailSenderImpl`, built fresh per
     `send()` call (config-only fields held, matching every other
     wrapper's own lazy discipline) — `spring-boot-starter-mail`
     against Mailpit by default, per Pillar 7's own text.
   - `Search.java.j2`: dependency-free `java.net.http.HttpClient`
     against OpenSearch's REST API directly, **a deliberate deviation
     from Pillar 7's literal "opensearch-java" text**, disclosed here:
     mirrors Go's own settled choice (`search.go.j2`'s own doc comment:
     "no official client is needed for three verbs") rather than
     vendoring a client library for `index`/`search`/`delete`. A single
     hardcoded `INDEX = "documents"` constant carries the index name
     the `search.index`/`search.query` verb signatures don't themselves
     carry — the identical `SEARCH_INDEX_NAME` shape Go's own `lower.rs`
     already uses.
   - `ExternalHttp.java.j2`/`Client.java.j2`: Spring's `RestClient`,
     per Pillar 7's own text — `RestClient.create(baseUrl)` opens no
     connection. Call clients (one `{Class}.java` per `CallTargetCtx`,
     mirroring Go's own `client.go.j2`) unwrap the `{"data": ..}`
     envelope and need **no manual OTel header injection** the way
     Go's own client does (`otel.GetTextMapPropagator().Inject`) —
     Micrometer Observation auto-instruments every `RestClient` request
     the moment `micrometer-tracing-bridge-otel` is on the classpath, a
     real simplification over Go's own explicit-injection shape,
     disclosed rather than assumed (confirmed by the same live
     `traced-checkout` verification below never needing to touch
     `Client.java.j2` for tracing at all).

   **Constructor injection was missing in two places, not one** —
   found live via `inventory-system.ciac`'s own `ComputePrice` classic
   handler, whose generated stub declared `// - cache: default` in its
   own doc comment but never actually received a `StringRedisTemplate`
   field: `logic.java.j2` (M4's own typed-handler template) already had
   `needs_cache`/`extras` plumbed through from `LogicFileCtx`, but
   `Service.java.j2` (the *classic*, seeded, pre-M4 handler stub) never
   consumed the identical `needs_cache`/`rust_cache_field`/`extras`
   fields `ServiceCtx` has carried since M2/M3 — a real, disclosed gap
   predating this milestone, fixed by mirroring the exact same
   `@Qualifier`-based constructor-injection shape into `Service.java.j2`
   that `logic.java.j2` already had for typed handlers.

   **Broker-hop trace propagation** (`Queue.java.j2`, Pillar 6): inject
   on `publishJson` (Micrometer's `Tracer.currentSpan()` +
   `Propagator.inject` into the same `Map<String,String> headers`
   parameter `publish` already accepted, unused until now), extract-
   and-span on consume (`Queue.traced(name, headers, Runnable)`, called
   from both the NATS `Dispatcher` callback and the Kafka
   `@KafkaListener` — the latter needed its own signature change,
   `byte[] raw` → `ConsumerRecord<String, byte[]>`, to reach the
   message's own headers at all). **A real generic-inference bug found
   live, not caught by the standalone API-surface scratch test that
   preceded it:** `propagator.extract(headers == null ? Map.of() :
   headers, ..)` compiled fine in isolation (a concrete `Map<String,
   String>` object passed directly) but failed exactly this shape once
   generated for real (`cannot infer type-variable(s) C,K,V`) — the
   ternary's own type inference produces an unrelated capture that
   defeats `Propagator.extract`'s generic signature. Fixed by hoisting
   the null-check to a separate statement
   (`Map<String,String> safeHeaders = headers == null ? Map.<String,
   String>of() : headers;`) before the call — the scratch test that
   verified `Tracer`/`Propagator`'s own API surface (`Span.Builder`,
   `Tracer.SpanInScope`, `Span.name(..)`) was real and load-bearing (it
   caught the Micrometer API shape correctly), but a scratch test only
   proves the API it actually exercises; the ternary shape needed the
   real generated file to surface.

   **Metrics/tracing config** (`pom.xml.j2`/`application.yml.j2`):
   `spring-boot-starter-actuator` gated on `has_metrics or has_tracing`
   (both autoconfiguration surfaces live in `spring-boot-actuator-
   autoconfigure`, confirmed live against a scratch project before
   committing to the template); `micrometer-registry-prometheus` +
   `management.endpoints.web.exposure.include: [prometheus]` when
   `has_metrics`; `micrometer-tracing-bridge-otel` +
   `opentelemetry-exporter-otlp` + `management.tracing.sampling.
   probability: 1.0` + `management.otlp.tracing.endpoint` when
   `has_tracing`.

   **`Component::Users` needed zero Java-specific code** — found live
   exactly as Go's/TS's own M7 already recorded: `dev-identity.ciac`
   generates and `mvn -q -B -DskipTests compile`s clean the moment
   `Users` joins the `supports()` OR-chain, confirming
   `ciac-codegen::model`'s own dev-Keycloak-issuer-default computation
   is target-neutral, not something any backend's own `lower.rs`/
   `lib.rs` has to implement.

   **Live-verified end to end, not just golden-generated or
   compile-checked:**
   - `extras-verbs.ciac` (all five ontology capabilities at once,
     cache/object_store/email/search/external_http, no db): `./mvnw -q
     -B verify` green with **zero** of Redis/S3/Mailpit/OpenSearch
     reachable — `NoInfraBootTest` genuinely proves the lazy-init
     discipline holds for every new bean this milestone added, not
     merely for the pre-existing db/queue ones. A live `spring-boot:run`
     round-trip against a real local Redis: seeded `evict-me` key in
     Redis directly, `POST /evict-cache-api {"key":"evict-me"}` →
     `{"status":"accepted","data":true}`, then confirmed via `redis-cli
     get evict-me` the key is genuinely gone — the `cache.delete` verb
     reaches real Redis at runtime, not just at compile time. The other
     three ontology routes (`index-doc-api`/`notify-user-api`/
     `call-upstream-api`, hitting genuinely unreachable OpenSearch/
     Mailpit/an external host) returned the expected 500 envelope with
     real network-layer exceptions in the server log (`MailConnect
     Exception: Couldn't connect to host, port: localhost, 1025`;
     `ResourceAccessException`/sandbox-proxy tunnel rejection) — proof
     each wrapper reaches the real client library correctly and fails
     only at the network, not from a code bug.
   - `inventory-system.ciac` (db + cache + a cross-service `call`):
     both services' `./mvnw -q -B verify` green against real local
     Postgres with Redis *unreachable* (the cache bean stays
     unconstructed — nothing in this example's own typed handlers
     calls a cache verb, only the classic `ComputePrice` stub's now-
     injected field) and again with Redis reachable. `GatewayController`
     `POST /quote` → `CatalogClient.price(..)` → unwraps the real HTTP
     envelope, confirmed via generated-source inspection matching
     `_steps.java.j2`'s own `call` arm exactly.
   - `ontology-growth.ciac` (2 db instances + cache + object_store +
     email + search + external_http + queue, the single densest
     capability set in the whole example suite): `./mvnw -q -B verify`
     green for the full trio (main/analytics Postgres reachable,
     everything else unreachable) — two workers' own NATS connection
     failures logged and caught exactly per the graceful-degradation
     discipline `Worker.java.j2`'s own doc comment already describes,
     never propagating out to fail the boot.
   - `multi-service-media.ciac` (5 services, object_store + email + a
     cross-service `call`): all five `mvn -q -B -DskipTests compile`
     clean.
   - `traced-checkout.ciac` (tracing + queue + `call`, the one example
     that first found the `Component::Tracing` `supports()` gap and
     the `Propagator.extract` ternary bug): both services' `./mvnw -q
     -B verify` green after both fixes — no live OTel collector in
     this sandbox (disclosed, the same "no Docker daemon" exclusion
     every other target's own OTel milestone already carries); what
     this milestone proves is structural: real `mvn` compilation of the
     full wiring (inject on publish, extract-and-span on consume for
     both NATS and Kafka), the actuator/Micrometer dependency graph
     resolving and autoconfiguring without error, `management.tracing.*`
     properties present in the generated `application.yml`.
   - `dev-identity.ciac`: `mvn -q -B -DskipTests compile` clean, zero
     Java-specific code needed (see above).
   - `mysql-notes.ciac`/`sim-vertical-slice.ciac` (the two remaining
     named `--system` CI targets): both `mvn -q -B -DskipTests compile`
     clean.

   **A large, disclosed side effect of finally widening `supports()`
   past `Cache`:** every example this arc's own M2–M6 milestones had
   left `CIAC0011`-refused for Java specifically because it declared
   `cache Redis` newly un-gates this milestone — `order-system.ciac`
   (the v0.14 flagship, refused since Java's own M6 exactly per that
   milestone's own disclosed-gap note), `crud-notes`, `routed-media`,
   `typed-video`, `video-platform`, `typed-handlers` (object_store/
   email/search/external_http), `sim-broker-slice`. All eleven golden
   trees are genuinely new (not modified), reviewed via `cargo insta
   test --accept` and spot-checked, not blindly trusted: `order-system`
   and `typed-handlers` specifically confirmed to compile via the same
   live `mvn -q -B -DskipTests compile` pass covering every newly-
   reachable example.

   **CI:** `actions/setup-java@v4` (temurin 21, maven cache) added to
   `generated-system`'s job (previously Rust/Python/Go/uv-only);
   `java × inventory-system`, `java × mysql-notes`, `java ×
   sim-vertical-slice` rows added to the `--system` matrix, mirroring
   TS's/Go's own M7 additions — `sim-vertical-slice` here exercises the
   real generated app against real Postgres/NATS via `--system` compose
   (the fidelity-ratchet block's own established shape), not `ciac sim
   --target java` (`SimSupport::None` until M9) — unaffected by
   simulation support landing later, the same precedent Go's own M7
   already established a milestone ahead of its own sim support.

   Full workspace verification: `cargo fmt`/`clippy --all-targets -D
   warnings` clean; `cargo test --workspace` green (21 suites, 0
   failures) after `cargo insta test --accept` — twenty-six Java
   goldens total, all regenerated (`AppState.java.j2`'s doc-comment
   reflow alone touches every one; eleven are genuinely new trees per
   the widened-`supports()` effect above).

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
