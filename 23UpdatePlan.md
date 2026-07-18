# CIaC v0.23-file — The TypeScript Backend: Node at Absolute Parity (implementation plan)

> Implementation plan. Document number ≠ release number (see
> 22UpdatePlan.md's preamble for the naming-quirk precedent; the
> release version is assigned at execution time). Assumes
> 22UpdatePlan.md ("the backend factory") has shipped: `TargetInfo`,
> neutral naming/filters, `lower_core`/`HostSyntax`, the conformance
> harness, the emission-plan helper, and the skeleton crate. This plan
> is deliberately the FIRST of the three new-language plans because it
> doubles as the factory's acceptance test — its M5 checkpoint
> measures the factory's promised cost model against reality and
> gates the Go and Java plans on the result.
>
> **Parity contract.** "Absolute parity" means: every row of
> docs/language.md's capability/provider table; typed handlers (full
> HIR lowering, every verb, every statement form); typed CRUD + the
> keyed-document store; relations and transactions (v0.16); the
> shared migration pipeline; authorization scopes plus the
> no-live-infra scope-test suite; OpenAPI embedding; structured
> logging/metrics/tracing including traceparent propagation across
> broker hops; typed cross-service call clients; realtime channels;
> generated system tests; compose/k8s/Terraform/CI emission;
> AGENTS.md and the seeded/compiler-owned manifest discipline;
> `ciac verify` validators; the `ciac dev` loop; vocab/LSP/describe/
> MCP visibility; evolution/semantic-diff/rename-replay
> participation; and simulation. Where Python and Rust themselves
> diverge, parity means matching the better of the two, and each such
> row is named in Pillar 9's divergence ledger: sim (Python full /
> Rust narrow → this plan targets the Rust-narrow slice as a gated
> final milestone, full parity disclosed as future work), scope tests
> (Rust excludes OAuth2 for a stated cryptographic reason → same
> exclusion, same stated reason), transactions (Rust's
> `transaction {}` is disclosed non-atomic → this backend implements
> REAL atomicity, matching Python and exceeding Rust, with the Rust
> gap cross-referenced so its catch-up stays tracked).
>
> **Confidence:** high on everything except the sim slice (gated,
> exactly as v0.17 M11 was gated) and the `Int` fidelity decision
> (Pillar 2 — decided in this document, disclosed loudly, revisitable
> only with a concrete user need). The ecosystem choices are the most
> contested of the three language plans (JS churns), which is why
> every selection records its rejected alternatives and a named
> fallback where the choice is genuinely close.

## The gap this version closes

TypeScript/Node is the largest backend-adjacent developer population
CIaC does not serve. The compiler already generates a TypeScript
*client* (v0.15 M2, `ts_client.rs`) — the standing asymmetry that a
CIaC system can be consumed from TypeScript but not built in it is
the single most common shape of "can I use this?" that the current
target list fails.

A TS backend also closes a strategic loop no other target closes:
the same organization's frontend and backend can share CIaC-generated
types end to end — the generated client's request/response interfaces
and the generated server's zod schemas derive from the same
`RecordCtx`, so a full-stack TypeScript shop gets one type system
from database row to browser fetch, compiler-verified at both ends.

Two further reasons this target goes first among the three:

1. **It is the hardest test of the factory's template contract.**
   Node's ecosystem is the least "batteries included" of the three —
   the backend assembles more discrete libraries than Go (stdlib-
   heavy) or Java (Spring-unified), so it exercises more of the
   per-capability template surface and more `TargetInfo` fields than
   either. If the factory's authoring path survives TS cleanly, Go
   and Java are strictly easier consumers.
2. **The v0.21 forecast interaction.** v0.21's "full TypeScript
   backend" breadth candidate is hereby superseded by this plan for
   the backend-generation portion; v0.21's selection machinery
   survives for its other candidates (OpenAPI bridge, admin UI).
   This supersession is recorded in both documents' preambles so the
   forecast can't be double-spent.

## Pillar 1 — Ecosystem selection

Selection criteria, in order: (1) most widely accepted/utilized in
production TypeScript services today, (2) maintenance health and
governance, (3) fit with CIaC's generation model — the compiler owns
SQL, migrations, and schemas, so libraries that insist on owning
those fight the compiler, (4) TypeScript-first typing quality.
Every rejected alternative is named with the reason, so a future
re-evaluation argues against a recorded decision rather than a
vacuum. Where a choice is genuinely close, the fallback is named and
the seam that makes flipping cheap is identified.

A governance note before the table, because JavaScript is the one
ecosystem on this arc's list where "most accepted" is a moving
target: each selection below is additionally scored on whether the
project has institutional backing or a governance structure that
survives a maintainer's departure (Fastify: OpenJS Foundation;
undici/slog-equivalents: platform teams; AWS/OpenSearch/OTel:
vendors; zod/pino/ioredis/kafkajs: community — the four community
picks are exactly the four rows that carry named fallbacks). The
generated projects' exact-pin + snapshotted-lockfile discipline
(Determinism section) is what makes any future flip a deliberate,
reviewable event rather than an emergency.

| Concern | Choice | Rejected alternatives, with reasons |
| --- | --- | --- |
| Runtime / language | Node.js ≥ 20 LTS, TypeScript 5.x `strict`, ESM | Deno/Bun: real momentum but far smaller production footprints; revisit as *providers* someday, not defaults. CJS: legacy; ESM-only avoids dual-build complexity |
| Package manager | npm (lockfile v3) | pnpm/yarn: excellent, but npm is the zero-install-assumption default; generated projects must work on any Node box |
| HTTP framework | **Fastify 5** | Express: the largest install base but architecturally stagnant, untyped by default, no first-class validation — parity with FastAPI/Axum demands validation-first routing. NestJS: a full opinionated framework; generated code inside its DI container fights compiler ownership of wiring. Hono: excellent and rising, smaller server-side production base today; named fallback if Fastify governance falters |
| Schema/validation | **Zod 3** | class-validator: NestJS-coupled decorators. TypeBox: technically superb Fastify fit (JSON-schema native) but smaller mindshare; named fallback. Zod is the de facto standard and mirrors pydantic's role exactly: schema object + inferred static type |
| Database | **Drizzle ORM** table objects + `pg`, `mysql2`, `better-sqlite3` drivers | Prisma: the most popular ORM, rejected on three structural conflicts — it owns its own schema DSL, its own migration engine, and a generated client, all three of which CIaC already owns. Knex: query-builder in maintenance twilight. Raw drivers only: viable floor, but Drizzle's typed table objects play the exact role SQLAlchemy models / Rust `FromRow` structs play in the existing backends (a typed row shape the compiler emits per table) |
| Migrations | CIaC's sequential SQL + a small generated runner | drizzle-kit / Prisma migrate: a second migration authority, rejected on principle — the shared differ (v0.7 M5) is the only author |
| Cache | **ioredis** | node-redis v4 (official): fine and closing the gap; ioredis remains the production standard with the larger deployed base. Decision recorded; the cache module is one file, flipping is cheap |
| Queue: NATS | **nats** (nats.js, official) | none serious |
| Queue: Kafka | **kafkajs** | node-rdkafka: librdkafka binding — native build chain pain, the exact cmake tax Rust already pays and Python avoided via aiokafka; kafkajs is the pure-JS analog of aiokafka. Confluent's new JS client: official but too new to be "most accepted." kafkajs's slowed maintenance cadence is a named risk with confluent as the recorded fallback |
| Auth JWT/OAuth2 | **jose** | jsonwebtoken: most installed, but stagnant, callback-era API, no JWKS story; `jose` is the modern standard and its `createRemoteJWKSet` provides the lazy, cached JWKS lookup the other backends hand-built in v0.17 M11 — the laziness bar met by dependency choice |
| Object store (S3) | **@aws-sdk/client-s3** (v3, official) | minio-js: simpler, but "most accepted" is the official SDK; MinIO compose compatibility is an endpoint override either way |
| Email (SMTP/SES) | **nodemailer** | none serious — it is the standard |
| Search (OpenSearch) | **@opensearch-project/opensearch** (official) | — |
| External HTTP | **undici** (Node core team's client; also powers global fetch) | axios: huge install base, adds nothing over undici for generated code and carries interceptor magic; got: maintenance-mode; bare fetch: viable, undici's request API gives better timeout/pool control for generated clients |
| Logging | **pino** | winston: larger legacy base, weaker structure/performance; pino is Fastify-native — one logging story instead of two |
| Metrics | **prom-client** | — (it is the standard) |
| Tracing | **@opentelemetry/sdk-node** + auto-instrumentations (official) | — |
| Realtime | **@fastify/websocket** (wraps `ws`) + SSE via reply streams | socket.io: a protocol of its own, not plain WebSocket — would break cross-target channel parity (a Python SSE consumer and a TS socket.io producer don't interoperate) |
| Scheduler | **croner** | node-cron: larger install base but weaker correctness reputation; croner is dependency-free, actively maintained, and natively accepts the 5-field/0-7-Sunday grammar CIaC's sema validates — no translation layer at all (contrast Rust's seconds-first rewrite). Decision recorded |
| Testing | **vitest** + `fastify.inject()` | jest: larger legacy base, slower, ESM friction; vitest is the current standard and ESM-native |
| Lint/format | **eslint** (typescript-eslint) + **prettier** | biome: promising all-in-one, smaller base; named fallback — the validator seam makes swapping a one-line `TargetInfo` change |
| Docker base | `node:22-slim` multi-stage | alpine: musl native-module edge cases (better-sqlite3 prebuilds) — slim is the safe default; distroless/nodejs: attractive, revisit with the deployment-maturity pass |

`TargetInfo` values (consuming 22UpdatePlan.md Pillar 1):

- `project_marker`: `package.json`
- `migrations_dir`: `migrations/` (identity filename mapping)
- `validate`: `npm ci` → `tsc --noEmit` → `eslint .` → `vitest run`
  (in that order: install, types, lint, tests — mirroring the
  uv-sync/ruff/pytest and cargo-check/test sequences)
- `compose`: `db_url_scheme: "postgres"`, `mysql_url_scheme:
  "mysql"`, `sqlite_url_prefix: "file:data/"`, suffix ``,
  data dir `/app/data`, `workers_command:
  ["node", "dist/workers.js"]`
- `dev`: rebuild `npm run build` (tsc), restart node processes
- `ci_test_steps`: setup-node@v4 with npm cache + the validate
  sequence
- `sim`: `None { reason }` until M9, then `Narrow` with the shared
  `unguarded_verbs`-driven coverage function

## Pillar 2 — Type system mapping and the `Int` decision

The full mapping, with the wire form pinned because cross-target
equality of the wire is what the conformance harness asserts:

| CIaC | TypeScript (memory) | Wire (JSON) | zod schema | Notes |
| --- | --- | --- | --- | --- |
| `Str` | `string` | string | `z.string()` | |
| `Int` | `number` | number | `z.number().int()` | see the decision below |
| `Float` | `number` | number | `z.number()` | |
| `Bool` | `boolean` | boolean | `z.boolean()` | |
| `Uuid` | `string` | string | `z.string().uuid()` | matching pydantic's validation behavior; stored as TEXT like every target |
| `Timestamp` | `Date` | ISO 8601 string | `z.coerce.date()` out, `.toISOString()` in | SQL columns unchanged (shared migrations) |
| `Json` | `unknown` | any JSON | `z.unknown()` | handler indexing lowers to safe access (below) |
| `enum { A, B }` | string literal union | string | `z.enum([...])` | same wire form as both backends; a named type alias per enum mirrors Rust's generated enums |
| `Record` | `interface` + inferred | object | `z.object({...})` | schemas.ts is the pydantic/`schemas.rs` analog |
| `Option<T>` | `T \| null` | `null` | `.nullable()` | explicitly NOT `undefined`/`.optional()` — wire parity requires a present, explicit null, and absent-vs-null must distinguish exactly as pydantic/serde do |
| `List<T>` | `T[]` | array | `z.array(...)` | |
| error records | `class XError extends Error` with typed fields | — | — | thrown; the route layer maps to the same status/shape `AppError`/the Python handler produce |

**The `Int` decision, in full.** CIaC `Int` is arbitrary-precision in
Python and `i64` in Rust; JavaScript numbers are exact only to
2^53−1. Three options existed: `number` (exactness gap above 2^53),
`bigint` (breaks `JSON.stringify`, every driver's default row
mapping, and the generated client's types), or a string-carrying
codec (breaks wire parity with every existing target). The decision
is **`number`**, because the *wire* format is JSON `number` for every
backend already — the cross-target contract is unchanged — and the
in-memory exactness gap is the narrowest honest cost. It is enforced
visible: `z.number().int()` refuses non-integers at the boundary,
`docs/language.md`'s determinism section gains the 2^53 disclosure in
the same table row that already discloses Python/Rust `Int` width
differences, and the conformance harness gains a boundary-value
decode test (2^53±1) whose TS behavior is asserted-as-documented
rather than left to be discovered. Revisiting requires a concrete
user need, not taste.

**Expression semantics via `HostSyntax` leaves.** The factory's
walker handles structure; these are the TS leaf rules, specified now
so M4 is transcription:

- String `+` with mixed operands lowers to template literals
  (`` `${a}${b}` ``), matching the format!/f-string behavior the
  shared dispatch already routes to the string-concat special case.
- Float literals render through the shared must-contain-a-dot
  fidelity rule (`1` vs `1.0` matters to goldens, not to JS — kept
  for cross-target readability of generated code).
- Integer division: JS `/` is float division; `Int / Int` lowers to
  `Math.trunc(a / b)` for i64-truncation parity with Rust (and the
  conformance equivalence test asserts it against Python's `//`
  lowering — this exact discrepancy class is why the equivalence
  test exists).
- `Json` indexing (`payload["items"][0]`) lowers to optional-chained
  access with a runtime presence check that throws the same
  shaped error a Python `KeyError` path produces — decided over
  silent `undefined` propagation, which would diverge behavior.
- Record construction is object literal + spread for `..base`;
  the E0382-class value-semantics hook is a documented no-op (GC
  language, no moves) — noted so the lower_core hook is knowingly
  unused, not forgotten.
- `if`/`match`: TS is expression-capable via ternaries but generated
  code favors readability — `match` lowers to a `switch` statement
  in statement position and an IIFE-free extracted variable in
  expression position (the StatementOriented shaping Python already
  exercises; TS runs in ExpressionOriented mode only where it reads
  naturally — the mode choice per construct is fixed in M4 and
  golden-visible).

## Pillar 3 — Project shape and the HTTP layer

Generated tree (single service; multi-service systems repeat this
per directory exactly as the other targets do):

```text
package.json  package-lock.json  tsconfig.json  Dockerfile  README.md
AGENTS.md  openapi.json  docker-compose.yml  migrations/000N_*.sql
src/
  main.ts            # fastify bootstrap, health, openapi route, migrate-on-boot
  workers.ts         # all workers/jobs/consumers in one process (parity with
                     #   workers_main.py / workers_bin.rs)
  config.ts          # env-driven, lazy — no I/O at import time
  state.ts           # AppState: lazy pool/redis/nats/jwks/clients
                     #   (+ world seam, M9)
  schemas.ts         # zod schemas + inferred types + enums + error classes
  models.ts          # drizzle table objects (the SQLAlchemy/FromRow analog)
  db.ts              # engine-keyed pool construction + migration runner
  observability.ts   # pino, prom-client /metrics, OTel init
  routes/<api>.ts    # one Fastify plugin per api pipeline
  logic/<handler>.ts # compiler-owned typed handlers (lowered HIR)
  services/<h>.ts    # seeded, user-owned extern/classic handler stubs
  workers/<w>.ts     # subscribe loop + handleMessageOnce (exported)
  clients/<svc>.ts   # typed cross-service call clients
tests/scope.test.ts  # vitest + fastify.inject scope suite (M6)
tests/system/        # shared Python system suite (free, unchanged)
```

A worked example of the generated route shape, because the route
template is the single most-read generated file and its parity
properties are the plan's contract (envelope, validation, error
mapping, publish-through-state):

```typescript
// routes/place_order_api.ts — Request pipeline for `api PlaceOrderApi`.
// Generated by CIaC.
import { FastifyPluginAsync } from "fastify";
import { Order, OrderSchema } from "../schemas.js";
import { PlaceOrder } from "../logic/place_order.js";
import { RecordAudit } from "../logic/record_audit.js";

export const placeOrderApi: FastifyPluginAsync<{ state: AppState }> =
  async (app, { state }) => {
    app.post("/orders", async (request, reply) => {
      let result: Order = OrderSchema.parse(request.body); // 400 on failure
      result = await new PlaceOrder(state).handle(result);
      result = await new RecordAudit(state).handle(result);
      await state.publish(
        "sim_vertical_slice.order_created",
        JSON.stringify(result),
      );
      return { status: "accepted", data: result };
    });
  };
```

Parity properties pinned by this sketch: the same
`{"status":"accepted","data":…}` envelope; zod parse failures map to
400 with the same shape FastAPI/axum produce (a Fastify error handler
translates `ZodError`); thrown error-record classes and unknown
errors map exactly like `error.rs`'s `AppError` (500 + logged cause,
canonical-reason body — one `setErrorHandler` in `main.ts`, the
`@RestControllerAdvice`/`AppError` analog); `/health` and
`/openapi.json` (embedded string, same single-source-of-truth doc
comment `routes_mod.rs.j2` carries). Handler classes take `state` and
expose `handle(payload)` — the same class shape both existing
backends generate, so pipeline call sites are uniform and the
`HandlerRef` model needs nothing new.

**HTTP behavior parity, itemized** — the small semantics that make
two targets' services actually interchangeable behind one client,
each asserted somewhere concrete:

- Content type `application/json` on all generated endpoints
  (OpenAPI equality pins the declaration; the smoke test pins the
  runtime header).
- 400 on malformed/invalid bodies with a JSON error body; 404/405
  from the router's defaults for unknown path/method — matching the
  framework defaults the other targets ship, which the generated
  system tests already exercise at the HTTP layer.
- 401 vs 403 split: missing/invalid token → 401, valid token
  lacking scope → 403 (the scope suite pins 403; the smoke test
  pins 401 — same split the existing auth modules implement).
- Method+path routing table byte-pinned by C3 (OpenAPI equality);
  trailing-slash behavior left at framework default and therefore
  asserted equal in the system tests rather than legislated here.
- Health returns `{"status":"ok"}` (the shared probe contract
  `--live` reads).

**Multi-service systems and `ciac new`.** Multi-service projects
repeat the tree per service directory with the shared system compose
at the root — the shared `SystemModel.multi` path needs nothing new
from this backend beyond its per-service emission being
prefix-relative (the emission-plan helper already is). `ciac new`
gains `--target typescript` scaffolding for free once the registry
lists the target; the scaffold templates' seeded-handler example is
the one target-specific `ciac new` artifact, added in M8 alongside
the docs. LSP/vocab visibility (hover showing per-target provider
support) derives from the registry — rows flip as `supports()`
un-gates, with no vocab edits (the factory's derived tables).

Ownership discipline is unchanged and free: `logic/` compiler-owned
and regenerated every build; `services/` seeded user-owned;
`AGENTS.md` explains the split; the shared manifest/sidecar layer
enforces it — zero new mechanism, asserted by re-running the existing
ownership tests against a TS tree in M8.

A second worked sketch — the schemas file — because it is where the
type mapping of Pillar 2 becomes visible bytes and where the
generated-client type-sharing claim is grounded:

```typescript
// schemas.ts — Generated by CIaC. Compiler-owned.
import { z } from "zod";

export const OrderSchema = z.object({
  id: z.string().uuid(),
  total: z.number(),
});
export type Order = z.infer<typeof OrderSchema>;

export const VideoStatusSchema = z.enum(["pending", "ready"]);
export type VideoStatus = z.infer<typeof VideoStatusSchema>;

export class InvalidOrderError extends Error {
  readonly status = 422 as const;
  constructor(readonly reason: string) { super(reason); }
}
```

And the worker sketch, pinning the seam the sim runner depends on:

```typescript
// workers/process_order.ts — Generated by CIaC.
export const SUBJECT = "sim_vertical_slice.order_created";
export const QUEUE_GROUP = "sim-vertical-slice-process-order";
export const MAX_RETRIES = 2;

export async function handleMessageOnce(
  state: AppState, payload: Order,
): Promise<void> {
  let result = payload;
  result = await new HandleOrderCreated(state).handle(result);
  void result;
}

export async function handleMessage(state: AppState, payload: Order) {
  for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) {
    try { await handleMessageOnce(state, payload); return; }
    catch (err) {
      if (attempt >= MAX_RETRIES) throw err;
      state.log.warn({ err, attempt }, "message processing failed; retrying");
    }
  }
}

export async function run(state: AppState): Promise<void> {
  const nc = await state.nats();
  const sub = nc.subscribe(SUBJECT, { queue: QUEUE_GROUP });
  for await (const msg of sub) {
    const payload = OrderSchema.safeParse(JSON.parse(sd.decode(msg.data)));
    if (!payload.success) { state.log.warn("discarding malformed message"); continue; }
    try { await handleMessage(state, payload.data); }
    catch (err) { state.log.error({ err }, "message processing failed after retries"); }
  }
}
```

Every structural beat mirrors `worker.py.j2`/`worker.rs.j2` on
purpose: malformed-message discard (not crash), retry loop hidden
behind `handleMessage`, per-attempt entry exported. The templates
are new text; the architecture is transcription.

### The statement-lowering table

The Pillar-2 leaf specs cover expressions; statements complete the
`HostSyntax` picture (all through the shared `StatementOriented`
shaping where noted):

| HIR statement | Generated TS |
| --- | --- |
| `Let { name, value }` | `const name = <expr>;` (or `let` when reassigned by sink shaping) |
| `Expr(value)` | `<expr>;` with `void` discard where the value is intentionally unused (mirrors the CIAC0045-tolerant `#[allow(unused)]` posture) |
| `Return(Some(v))` | `return <expr>;` |
| `Return(None)` | `return;` |
| `Fail { error, args }` | `throw new XError(args…);` |
| `Publish { stream, value }` | `await state.publish(subject, JSON.stringify(<expr>));` |
| `Transaction { body }` | `await state.db.transaction(async (tx) => { <body with tx-threaded verb leaves> });` |

## Pillar 4 — Database: CRUD, keyed store, relations, transactions, migrations

**Placeholders and bind order.** `pg` takes `$N` natively; `mysql2`
and `better-sqlite3` take `?`. The shared `sqlph` filter and the
v0.13 M1 bind-order discipline (UPDATE assignments first, id last)
apply completely unchanged — this backend adds zero new placeholder
logic, and the conformance harness's topology assertion verifies the
emitted SQL text matches the other targets byte-for-byte.

**Typed CRUD + keyed-document store.** Same SQL from the shared
`RecordCtx` fragments; drizzle table objects in `models.ts` provide
the typed row mapping for reads (the `FromRow`/SQLAlchemy analog);
the keyed store's column is `JSONB`/`JSON`/`TEXT` per engine exactly
as v0.13 M3 settled, with the store module template
(`resource_store` analog) following the Python template's
cache-aside shape where `cache_ttl` is configured.

**Verb lowering table** (the M4 transcription target — each row is a
`HostSyntax` leaf):

| Verb | Generated TS shape |
| --- | --- |
| `db.insert(T, v)` | `const row = {...v}; await state.db.query(INSERT_SQL, binds(row)); row` — world-guarded in M9 exactly like Rust's block |
| `db.get(T, id)` | `query → rows[0] mapped through the model type or null` |
| `db.update(T, id, v)` | fields-first-id-last binds; `rowCount === 0 ? null : row` |
| `db.delete(T, id)` | `rowCount > 0` |
| `db.query(T, pred)` / `count` / `delete_where` | shared predicate SQL + binds from the model's predicate terms |
| `cache.get/set/delete` | ioredis get/set with the same JSON codec both backends use |
| `object_store.*` | S3 client calls behind the generated `ObjectStore` wrapper (same wrapper shape as `object_store.rs`) |
| `email.send` | nodemailer via the generated `Email` wrapper |
| `search.index/query` | OpenSearch client via the generated wrapper |
| `http.call` | undici request via the generated `ExternalHttp` wrapper |

**Transactions.** REAL atomicity: the `transaction {}` block lowers
to a dedicated-connection transaction (`pg` client checkout /
drizzle `db.transaction`), commit on fall-through, rollback on
throw — semantics matching Python's session exactly, explicitly
exceeding Rust's disclosed non-atomic gap, with the standing
cross-reference so Rust's catch-up remains a tracked item rather
than a forgotten asymmetry.

**Migrations.** CIaC's sequential SQL applied by a generated runner
in `db.ts`: a `_ciac_migrations` ledger table, apply-in-order inside
a transaction per file where the engine allows, idempotent re-run —
the same runner shape Python generates, executed from `main.ts` on
boot like `sqlx::migrate!`. The regen/rename-replay machinery works
through `TargetInfo.migrations_dir` with identity filenames — no
special casing.

**Relations (v0.16).** Enforced by the same shared migration SQL (FK
constraints); the backend surfaces constraint violations as plain
driver errors, matching the disclosed unmapped-exception behavior of
both existing backends (mapping them nicely is a cross-target future
item, deliberately not solved unilaterally here — parity includes
parity of gaps).

**Lazy pools, from day one.** `state.ts` constructs everything
lazily: `pg.Pool` is lazy by design; ioredis with
`lazyConnect: true`; NATS behind a memoized connect promise; JWKS
via jose's remote set (lazy+cached); undici pools on first use. The
v0.17 M11 bar — constructing AppState touches zero infrastructure —
is met at M1 and TESTED at M1 (a vitest case constructs state with
unreachable config and asserts no rejection), because the sim seam
(M9) and the scope suite (M6) both depend on it structurally.

## Pillar 5 — Broker, workers, jobs, channels

**NATS.** `nats.js` `subscribe(subject, { queue: group })` — direct
queue-group parity with both backends' semantics (competing
consumers within a group, fan-out across groups).

**Kafka.** kafkajs consumer with `groupId` = queue group, topic =
subject — the v0.11 M3 mapping verbatim; producer with explicit
acks; headers carried for tracing.

**Workers.** The generated worker preserves the load-bearing seam
both prior arcs proved matters:

```typescript
// workers/process_order.ts (shape)
export const SUBJECT = "sim_vertical_slice.order_created";
export const QUEUE_GROUP = "sim-vertical-slice-process-order";
export const MAX_RETRIES = 2;

export async function handleMessageOnce(
  state: AppState, payload: Order,
): Promise<void> { /* lowered pipeline steps */ }

export async function handleMessage(state: AppState, payload: Order) {
  for (let attempt = 0; attempt <= MAX_RETRIES; attempt++) { /* retry loop */ }
}

export async function run(state: AppState) { /* subscribe loop */ }
```

`handleMessageOnce` is exported precisely because the sim runner
(M9) and attempt-counting depend on that exact entry point — the M1
finding of the v0.17 arc, preserved by construction.

**Jobs.** croner parses the source 5-field expression directly — the
first backend needing NO schedule translation (recorded in the model
docs: `cron_crate_schedule` is Rust-specific by name from here on).
The run loop sleeps to next fire; `handleTickOnce` exported for the
sim runner; `catch_up` semantics matching the shared contract.

**Channels.** `@fastify/websocket` handler / SSE reply stream, each
bridging a plain (non-group) subscription so every connected client
receives every message — mirroring `channel.py.j2`/`channel.rs.j2`
fan-out semantics, asserted by the existing generated system tests
(which already probe channels over real websockets, target-neutrally).

**Traceparent propagation.** Publish sites inject W3C context into
NATS/Kafka headers; consume sites extract and continue the trace —
byte-for-byte the header contract v0.15 M3/M4 established, proven by
extending the existing cross-target trace test to three targets.

## Pillar 6 — Auth, scopes, and the scope-test suite

`jose` verifies HS256 (JWT provider, shared secret) and RS256 via
`createRemoteJWKSet` (OAuth2 provider — lazy, cached, kid-matched;
the library does what v0.17 M11 hand-built for Rust). The generated
auth module exposes the same claims shape (`sub`, `scopes`) and the
same `requireScope` check-and-403 the other backends emit, driven by
the shared scope collection (`Ctx::scopes`) — one preHandler hook
per scoped route.

The scope-test suite uses `fastify.inject()` — Fastify's built-in
no-listener request injection, the exact analog of
`tower::ServiceExt::oneshot` — so `tests/scope.test.ts` runs with
zero live infrastructure: per scope, the 403-without/200-with
assertion pair, tokens minted with the test secret, the same suite
shape `scope_tests.rs.j2` generates. OAuth2 is excluded from the
no-infra suite for the same stated reason as Rust (real RS256
verification requires a real issuer's JWKS regardless of laziness),
with the same comment at the gate — a parity-of-disclosure item, and
the standing cross-target candidate for a future fake-issuer
mechanism, named not solved.

The suite's shape, sketched (one pair per scope from `Ctx::scopes`,
same dummy-body generation the Rust suite uses — including the
snake_case `FieldTypeKind` tag matching whose absence was the v0.17
M11 scope-test bug, cited in the template comment so the lesson
travels):

```typescript
// tests/scope.test.ts — Generated by CIaC.
import { buildApp } from "../src/main.js";

test("orders:write is enforced", async () => {
  const app = await buildApp(testConfig()); // no infra touched: lazy state
  const denied = await app.inject({
    method: "POST", url: "/orders",
    headers: auth(token({ scopes: [] })),
    payload: { id: UUID, total: 1.0 },
  });
  expect(denied.statusCode).toBe(403);
  const allowed = await app.inject({
    method: "POST", url: "/orders",
    headers: auth(token({ scopes: ["orders:write"] })),
    payload: { id: UUID, total: 1.0 },
  });
  expect(allowed.statusCode).not.toBe(403); // mechanism proof, per v0.14 M6
});
```

The `not.toBe(403)` (rather than `toBe(200)`) mirrors the existing
suites' claim boundary exactly: this proves the *scope mechanism*,
not the handler's happy path, which may legitimately fail on the
unreachable fake infrastructure behind it.

## Pillar 7 — Ontology remainder and call clients

- **S3:** official v3 client with `forcePathStyle` + endpoint
  override for MinIO — the compose layer already emits the endpoint/
  credential env vars target-neutrally, so the wrapper consumes the
  same five config fields `object_store.rs` does.
- **Email:** nodemailer transport against compose's Mailpit; same
  six config fields.
- **Search:** official OpenSearch client; same URL field.
- **External HTTP + call clients:** undici. Generated
  `clients/<svc>.ts` follow the same base-URL env convention
  (`BILLING_URL` etc. — emitted by the shared compose/k8s layer
  already). The v0.15 M2 generated TS *client* is reused as the call
  client where its shapes align (same `CallApiCtx` inputs); measured
  at implementation — if server-side needs diverge (error mapping,
  tracing hooks), the backend emits its own client template and the
  divergence is documented in the template header rather than
  forced. Either way the generated *browser* client artifact is
  unchanged.

## Pillar 8 — Observability, deployment, and the standard file set

pino JSON logs with the field conventions the other targets'
structured logging documents (service, level, timestamp, message,
context fields); prom-client default registry + `/metrics` route
when `metrics Prometheus` is declared; OTel SDK init in
`observability.ts` gated on `tracing OpenTelemetry`, OTLP exporter
env conventions identical (`OTEL_EXPORTER_OTLP_ENDPOINT` etc. — the
shared collector config file already exists), Fastify/undici/pg
auto-instrumentations plus the manual broker-hop propagation of
Pillar 5.

Dockerfile: multi-stage `node:22-slim` — `npm ci` → `tsc` → runtime
stage with `dist/` + production-only `node_modules` (`npm ci
--omit=dev`); `.dockerignore` carries `node_modules`/`dist` the same
way the Rust template carries `/target` (same image-bloat rationale,
same comment). README/AGENTS.md from the shared emission plan.
Compose: the `TargetInfo.compose` values above; k8s/Terraform/CI:
free from the shared layer, with `ci_test_steps` supplying the
setup-node sequence for generated projects' own CI.

### The config/env surface

Pinned because it is cross-target contract, not per-language choice —
the compose/k8s layer already emits these names for every service,
and `config.ts` merely reads them (parity table shown for the core
set; ontology instances follow the same established per-instance
suffix conventions):

| Env var | Consumed as | Notes |
| --- | --- | --- |
| `DATABASE_URL` (+ `_MAIN` etc. per instance) | pg/mysql2 pool config, sqlite path | schemes per `TargetInfo.compose` |
| `REDIS_URL` (per instance) | ioredis constructor | `lazyConnect: true` |
| `NATS_URL` / `KAFKA_URL` | nats.js / kafkajs | first-use connect |
| `JWT_SECRET` / `OAUTH_ISSUER` (+ audience) | jose HS key / remote JWKS URL | |
| `<SVC>_URL` per call target | client base URLs | shared default-port conventions |
| S3/email/search instance vars (5/6/1 fields) | wrappers | same fields as `object_store.rs`/`email.rs`/`search.rs` |
| `OTEL_EXPORTER_OTLP_ENDPOINT` etc. | OTel SDK | identical across all five targets |

### Template inventory

Sized against the audit's essential-cost model — ~33 templates,
estimated ~2,700–3,000 lines, checked at M5:

| Group | Templates |
| --- | --- |
| project | `package.json`, `tsconfig.json`, `Dockerfile`, `README.md`, `system-README.md` |
| app core | `main.ts`, `workers.ts`, `config.ts`, `state.ts`, `observability.ts`, `error-handler.ts` |
| data | `schemas.ts`, `models.ts`, `db.ts`, `resource_store.ts` |
| http | `route_api.ts`, `resource_api.ts`, `channel.ts`, `client.ts` |
| async | `worker.ts`, `consumer.ts`, `job.ts`, `queue.ts` |
| handlers | `logic.ts` (compiler-owned), `service.ts` (seeded stub) |
| ontology | `cache.ts`, `object_store.ts`, `email.ts`, `search.ts`, `http_clients.ts`, `auth.ts` |
| tests/sim | `scope.test.ts`, `smoke.test.ts`, `sim_runner.ts` (M9) |

Every row has a named analog in the Python (31) or Rust (37)
inventory listed in 22UpdatePlan.md's audit — nothing here is a
novel file kind, which is itself a parity check: a template with no
cross-target analog would mean this backend invented behavior.

## Pillar 9 — Simulation (gated) and the divergence ledger

The ledger this plan commits to resolving or disclosing, row by row:

| Row | Python | Rust | This backend |
| --- | --- | --- | --- |
| sim | full fakes, record/replay | narrow (db.insert + publish/consume + cron), no replay | **narrow slice, M9 (gated)**; full = disclosed future |
| scope tests | full | JWT-only | JWT-only, same stated reason |
| `transaction {}` | atomic | disclosed non-atomic | **atomic** |
| `Int` | arbitrary precision | i64 | number (2^53), disclosed + boundary-tested |
| sim record/replay | yes | no | no (matches Rust), disclosed |
| schedule translation | library-native | seconds-first + weekday rewrite | none needed (croner) |

M9's sim slice mirrors v0.17 M11's Rust continuation step for step,
with one structural difference disclosed up front: TS cannot vendor
`ciac-sim`'s Rust source the way the Rust backend does via
`include_str!`, so `world.ts` is a *narrow restatement* — exactly
the position Python's `sim/pyrunner/world.py` already occupies, with
the same docstring discipline saying so. Contents: fake table map +
fake queue + occurrence-counted `(effect, subject)` failure rules
(`error` action only); `state.publish` and the `db.insert` leaf gain
the same world-guard branch Rust's templates gained; a generated
`sim_runner.ts` drives `fastify.inject` for requests (real HTTP
status codes, like Rust's oneshot — exceeding Python's
"200 means didn't throw" disclosure), worker `handleMessageOnce`
retry budgets for drains, croner due-instants for advances, and
`SimWorld` state for expects; `ciac sim --target typescript` goes
through `SimSupport::Narrow` with the shared `unguarded_verbs`
coverage gate; the same one-line-JSON `SimScenarioOutcome` child
protocol. Acceptance is exact: both checked-in scenarios reproduce
`{"ProcessOrder":3}/{"Reconcile":1}` and
`{"ProcessOrder":100}/{"Reconcile":7}`, and the refusal case
(order-system: auth + unguarded verbs) names its reasons.
Fidelity-ratchet row: sim-vertical-slice × typescript joins the
`generated-system` CI matrix.

The runner's step dispatch, sketched to fix its architecture (the
same closed vocabulary every runner in the fleet interprets):

```typescript
// sim_runner.ts (generated) — one scenario, one JSON line, exit.
for (const step of scenario.steps) {
  if ("request" in step) {
    const res = await app.inject({ method, url, payload: step.request.json });
    if (step.request.save_as) saved.set(step.request.save_as,
      { status: res.statusCode, json: res.json() });
  } else if ("drain" in step) {
    for (const [subject, raw] of world.queue.takeAll()) {
      if (subject === workers.processOrder.SUBJECT) {
        attempts.ProcessOrder += await driveWithRetries(
          workers.processOrder.handleMessageOnce, raw,
          workers.processOrder.MAX_RETRIES);
      } else throw new Error(`no worker for subject ${subject} (disclosed scope)`);
    }
  } else if ("advance" in step) {
    for (const fire of dueInstants(schedule, nowMs, nowMs + ms(step.advance.by)))
      { await workers.reconcile.handleTickOnce(state); runs.Reconcile++; }
    nowMs += ms(step.advance.by);
  } else if ("expect" in step) { assertExpect(step.expect, world, saved,
      attempts, runs); }
  else if ("publish" in step) throw new Error("publish steps: disclosed scope");
}
console.log(JSON.stringify({ scenario: name, passed, error,
  worker_attempts: attempts, job_runs: runs }));
```

Line-for-line this is `sim_runner.rs.j2`'s architecture in TS
spelling — deliberate, so a reader who audits one runner has audited
the fleet's shape, and so a scenario-semantics fix is recognizably
the same edit in five templates.

## Implementation map

| Artifact | Content |
| --- | --- |
| `crates/ciac-backend-ts/src/lib.rs` | `TargetInfo`, filter registration (`ts_type` over `FieldTypeKind` + `NameForms`), emission table, `supports()` gating ladder |
| `crates/ciac-backend-ts/src/lower.rs` | `HostSyntax for TsSyntax` — the ~30 leaves of Pillars 2/4, nothing else (the factory's walker does the rest) |
| `crates/ciac-backend-ts/templates/` | the ~33 templates above |
| `crates/ciac/src/commands.rs` | ONE line: the registry entry |
| `tests/tests/snapshots/` | `gen__typescript__*` goldens per example (registry-enumerated, no test edits) |
| `.github/workflows/ci.yml` | `generated-typescript` job + system-matrix rows |
| docs | backends.md section, simulation.md column (M9), generated provider table rows |
| conformance | zero edits — registry-driven (C1–C7 pick the target up automatically) |

The map's brevity IS the factory's acceptance test: compare it with
what the audit says the Rust backend touched historically.

## Capability parity checklist

The definition-of-done matrix M8 signs off — each row names the
implementing module, the proving example, and the milestone that
un-gates it:

| Capability/feature | Module | Proving example | M |
| --- | --- | --- | --- |
| api pipelines + envelope + errors | routes/, error-handler | ping, typed-video | 1/4 |
| records/enums/errors | schemas.ts | typed-handlers | 2 |
| db Postgres/MySQL/SQLite CRUD | db.ts, models.ts | crud-notes, mysql-notes, sqlite-notes | 2 |
| keyed-document store | resource_store.ts | ontology-growth | 2 |
| migrations | db.ts runner | crud-notes | 2 |
| queue NATS / Kafka | queue.ts, workers/ | event-pipeline, kafka-pipeline | 3 |
| workers + retries | workers/ | sim-vertical-slice | 3 |
| jobs (cron, catch_up) | workers/ | scheduled-cleanup | 3 |
| channels WS/SSE | channel.ts | realtime-progress | 3 |
| typed handlers (all verbs) | logic/, lower.rs | typed-handlers, query-verbs, extras-verbs | 4 |
| transactions (atomic) | db.ts + Transaction leaf | domain-orders | 4 |
| relations (v0.16) | shared SQL + driver errors | domain-orders | 4 |
| auth JWT/OAuth2 + scopes | auth.ts | order-system, oauth-echo | 6 |
| scope tests (no infra) | scope.test.ts | order-system | 6 |
| object store / email / search / external http | wrappers | ontology-growth, extras-verbs | 7 |
| call clients | clients/ | multi-service-media, inventory-system | 7 |
| logging/metrics/tracing + propagation | observability.ts | traced-checkout | 1/7 |
| users Keycloak wiring | shared compose | dev-identity | 7 |
| system tests | shared | inventory-system (--system, CI) | 7 |
| compose/k8s/terraform/ci emission | shared | all | 1 |
| AGENTS.md + ownership | shared manifest | all | 1 |
| dev loop / MCP / evolution / rename | shared via TargetInfo | scripted sessions | 8 |
| sim (narrow) | world.ts, sim_runner.ts | sim-vertical-slice, sim-broker-slice | 9 |

## Determinism and supply chain

The repo invariant (same input → byte-identical output) extends to
this target's ecosystem risks explicitly: generated `package.json`
pins exact versions (no `^`/`~`); the generated `package-lock.json`
is part of the emitted project AND part of the goldens, so the full
transitive tree is pinned and diffable; `npm ci` is the only install
verb in validators, CI, and Dockerfiles; dependency upgrades are
deliberate template changes that show up as golden diffs with a
reasoned commit message. The generated project's own CI (via
`ci_test_steps`) inherits the same posture. No postinstall scripts
from generated code; better-sqlite3's build script is the one
allowed native compile, executed at `npm ci` time and cached — its
failure mode (missing build tools) is an honest validator error,
documented in the generated README's requirements line.

## Diagnostics, gating, and docs impact

- **Gating:** `supports()` starts narrow (M1 scope) and un-gates per
  milestone — the same CIAC0011 discipline every backend arc used;
  the conformance harness treats a gated example/target pair as a
  disclosed skip, exactly as CI's generated-rust job does today.
- **No new error codes expected.** Existing codes cover the surface
  (CIAC0011 gating, CIAC0035 stale-file warnings, CIAC0003
  path-collision checks are all target-neutral). If implementation
  surfaces a TS-specific diagnosable condition, it lands with a code
  + docs/errors.md entry through the standard procedure rather than
  a bare anyhow error.
- **Docs:** `docs/language.md` provider table rows flip via the
  generated table (factory M1); `docs/backends.md` gains the TS
  section (deps table + divergence ledger); `docs/simulation.md`
  status table gains the TS column at M9; README target list;
  `docs/authoring.md` editor notes unchanged (LSP is
  target-neutral).

## Milestones

1. **M1 — Skeleton to ping-parity.** Copy
   `backends/skeleton-internal`; register `TargetInfo` (the ONLY
   edit outside the crate — the factory's first acceptance
   assertion, checked in review). Emit package.json (+ committed
   lockfile), tsconfig, Dockerfile, README, AGENTS.md, config.ts,
   state.ts (with the no-infra construction test), observability.ts,
   main.ts with `/health` + `/openapi.json`. `examples/ping.ciac`
   verifies end-to-end through the real validator sequence (npm ci,
   tsc, eslint, vitest smoke — Node toolchain present locally, so
   this proof is live, not delegated). Goldens begin; `supports()`
   gated to M1 scope; cold/warm `npm ci` times recorded for the CI
   budget ledger.

   **Shipped (v0.23 M1):** `crates/ciac-backend-ts` — `TsBackend`
   with `TargetInfo` (`project_marker: "package.json"`, `validate`:
   `npm ci` → `tsc --noEmit` → `eslint .` → `vitest run`,
   `ci_test_steps` via `actions/setup-node@v4`, `dev.rebuild: npm run
   build`, `sim: None` until M9). `supports()` is gated to exactly
   `Component::Api` — the single-construct scope `examples/ping.ciac`
   exercises; every other component (db/cache/queue/service/worker/
   job/channel/auth/...) stays `CIAC0011`-refused until its own
   milestone. `ciac build examples/ping.ciac --target typescript`
   live-verified end to end against the real toolchain: `npm ci`
   (0 vulnerabilities), `npx tsc --noEmit`, `npx eslint .`, `npx
   vitest run` (1/1) all pass on the actually-generated project, and
   the built server (`npm run build && node dist/main.js`) answers
   real HTTP requests — `/health` → `{"status":"ok"}`, `/openapi.json`
   → the real embedded doc, `POST /echo` → `{"status":"accepted",
   "data":{...}}`, the identical envelope shape Python/Rust already
   produce. `npm ci` timing (this sandbox, not representative of CI
   hardware): ~3.8s with an empty npm cache, ~3.3s with npm's package
   cache warm from a prior install.

   Ecosystem picks actually pinned (real current-as-of-execution
   versions, not the plan's illustrative ones): Fastify 5.10.0, pino
   10.3.1, TypeScript 5.9.3 (latest stable 5.x — 7.x exists but is a
   different, Go-ported compiler outside this plan's stated "5.x"
   decision), eslint 10.7.0 + `@eslint/js` 10.0.1 +
   typescript-eslint 8.64.0, vitest 4.1.10 (not the initially-tried
   3.2.4: `npm audit` found a real critical CVE, GHSA-5xrq-8626-4rwp,
   in vitest <3.2.6's UI server — disclosed and avoided, not shipped).
   `@types/node` pinned to 22.20.1 specifically to satisfy vite's
   (vitest's own dependency) peer-range floor of `>=22.12.0` cleanly
   — `npm ci` reports zero warnings, not just zero vulnerabilities.

   Two real bugs the live proof caught, not hypothesized: (1) the
   route template's `state` plugin argument was unconditionally
   unused at M1 scope (no db/handler/publish step exists yet to read
   it) but only suppressed for bodyless apis — `eslint`
   (`no-unused-vars`) failed on `Echo`'s typed-body case; fixed to
   suppress unconditionally, with a comment explaining exactly when
   that stops being true (M2 onward). (2) `tests/src/lib.rs`'s
   `backends()` — the registry `conformance.rs`/`golden.rs`/
   `targets_cli.rs` correctly iterate registry-agnostically — was
   *also* being reused by `gating.rs`'s six `..._on_both_backends`
   tests and by three unguarded `generate()` loops in `blueprints.rs`/
   `determinism.rs`/`modules.rs` that never called `check_support`
   first. Both are real, disclosed fixes, not TS-specific patches:
   added `full_parity_backends()` (Python+Rust only) for the tests
   whose names and intent are genuinely about those two mature
   backends, and added the missing `check_support` guard to the three
   loops that were silently relying on "every backend supports
   everything" — true only by accident before a narrowly-gated third
   target existed, and exactly the kind of latent gap 22UpdatePlan.md's
   own conformance harness was built to catch structurally rather than
   ad hoc. `docs/targets.json` regenerated (new `typescript` entry,
   `capabilities: {}` — correctly empty, since `vocab::PROVIDERS`
   only lists python/rust as of 22 M4's own disclosed disposition). A
   new golden (`gen__typescript__ping`) is the only accepted snapshot
   diff. Full verification green: `cargo fmt --check`, `cargo clippy
   -D warnings`, `cargo test --workspace` (all suites, zero
   failures).
2. **M2 — Records, schemas, models, CRUD, keyed store, migrations.**
   schemas.ts (zod + enums + error classes), models.ts (drizzle
   tables), db.ts (engine-keyed pools + migration runner), typed
   CRUD and the keyed-document store across Postgres/MySQL/SQLite
   with the shared placeholder/bind discipline. Live proof:
   sqlite-notes fully local with zero Docker (file-backed engine —
   the same zero-infra proof v0.13 M3 used); crud-notes/mysql-notes
   verify statically local, capability round-trips CI-delegated as
   always.

   **Shipped (v0.23 M2):** `schemas.ts` (zod schemas + `z.infer` types
   + error classes, from `c.records`), `models.ts` (Drizzle table
   objects for CRUD resources and `table` declarations, column
   builders called through a real Rust filter — `drizzle_column` in
   `filters.rs` — rather than picked in Jinja, mirroring the
   Postgres/MySQL/SQLite `FieldTypeKind` mapping decided in Pillar 2),
   `db.ts` (hand-written per-engine `CREATE TABLE IF NOT EXISTS` DDL
   for CRUD resources reached through Drizzle's `$client` escape hatch
   — there is no declarative-sync API outside drizzle-kit — plus a
   hand-rolled `_ciac_migrations`-ledger runner for `table` decls,
   mirroring Python's own runner shape since none of the three drivers'
   own migration tooling matches CIaC's plain-numbered-`.sql`-file
   contract), one `stores/<resource>.ts` class per CRUD resource (typed
   and untyped/keyed-document, Redis cache-aside where `has_cache`,
   using Drizzle's query builder directly rather than raw placeholder
   SQL — sidestepping the `sqlph`/bind-order machinery entirely for
   this pre-built REST layer, since Drizzle already generates correct
   per-engine SQL from a single portable call), and one
   `routes/<resource>.ts` Fastify plugin per resource (create/list/
   get/update/delete, Zod validation via `{{Name}}Schema.omit({id:
   true})` reused straight from `schemas.ts` rather than a second
   generated schema). `state.ts` gained one `AppState` field per named
   db/cache instance — `pg.Pool`/`mysql2`'s pool are lazy by
   construction (no connection until a query runs), `ioredis` is
   built with `lazyConnect: true`, and `better-sqlite3` only ever
   touches a local file — so the M1 "AppState touches zero
   infrastructure" bar (tested since M1) still holds with real
   capability instances present. `TsBackend::supports()` widened to
   `Database` (all 3 engines), `Cache`, and a classic binding-style
   `Service { signature: None }` — CRUD/keyed-store resources compile
   to exactly that component triple; a *typed* `service` still stays
   refused until M4.

   Two real, non-hypothetical type errors the live `tsc --noEmit`
   proof caught, both fixed by *not* trusting a plausible-looking
   type annotation: (1) `ReturnType<typeof drizzle>` on an AppState
   field looked right but is wrong for `drizzle-orm/mysql2`
   specifically — being a generic function with a defaulted type
   parameter, `ReturnType` resolves the *default* instantiation
   (`$client: <callback-style mysql2 Pool>`), not the one the real
   call site narrows to when handed an actual `mysql2/promise` `Pool`
   — so the field type and the constructed value structurally
   disagreed. Fixed by spelling the field type as the concrete
   intersection (`MySql2Database & { $client: mysql.Pool }`) at every
   one of the three engines, not just the one that happened to break
   first. (2) `import Database from "better-sqlite3"; ...: Database`
   — the default import is a namespace, not a usable type in this
   project's module setup (`Cannot use namespace 'Database' as a
   type`); fixed by importing the constructor and the `Database`
   *type* under separate names (`import SqliteDatabase, { type
   Database } from "better-sqlite3"`). Both fixes are disclosed
   because they'd have shipped silently wrong if the M2 gate had been
   "renders without a template error" instead of a real `tsc`
   pass — exactly the trust-the-tool-not-the-plausible-guess
   discipline the M1 vitest-CVE and eslint fixes already established.

   Live proof, real toolchain, zero Docker: `sqlite-notes` — `npm ci`
   (0 vulnerabilities), `npx tsc --noEmit`, `npx eslint .`, `npx
   vitest run` (1/1) all pass on the generated project; `npm run
   build && node dist/main.js` answers a real HTTP CRUD round-trip
   against an actual SQLite file — `POST /notes` → 201 with a
   server-generated id, `GET /notes` lists it, `GET /notes/:id`
   fetches it, `PUT /notes/:id` updates it, `DELETE /notes/:id` → 204,
   a subsequent `GET /notes/:id` → 404. `mysql-notes` (db + cache, no
   auth) and the multi-service `inventory-system` golden verify
   statically (`npm ci`, `tsc --noEmit`, `eslint .` all clean); a live
   MySQL/Redis round-trip is CI-delegated per the plan's own stated
   allowance (no local MySQL/Redis daemon in this sandbox). As a
   disclosed, real deviation from this milestone's own checklist row —
   not a silent gap — `crud-notes.ciac` (needs `auth JWT`) stays
   `CIAC0011`-refused this pass: auth is genuinely out of scope until
   M6, verified by a real `ciac build --target typescript` run against
   it that fails with exactly that code, not by inspection. Full
   verification green: `cargo fmt --check`, `cargo clippy -D
   warnings`, `cargo test --workspace` (61 suites, zero failures); 4
   new/updated goldens accepted (`ping` — trivial, from the M2
   `state.test.ts` engine-aware env-var fix below — plus new
   `sqlite-notes`/`mysql-notes`/`inventory-system` trees).

   One more real fix, found by the same live proof rather than
   guessed: M1's `state.test.ts` unconditionally set
   `DATABASE_URL=postgres://unreachable-host:1/db` regardless of which
   engine(s) the generated project actually declares. Harmless while
   no backend had a real db instance to construct against it, but once
   `sqlite-notes` generated a real `better-sqlite3` `AppState` field,
   that Postgres-shaped URL got handed straight to
   `new Database(path)` as a literal (non-existent) filesystem path,
   failing the "never touches the network" test for a reason that had
   nothing to do with the network. Fixed by generating one
   engine-appropriate unreachable URL per declared instance instead of
   a single hardcoded one.
3. **M3 — Broker, workers, jobs, channels.** nats.js + kafkajs,
   worker retry loop delegating to exported `handleMessageOnce`,
   croner jobs with `handleTickOnce`, WS/SSE channels, publish sites
   through `state.publish`, traceparent headers. event-pipeline,
   kafka-pipeline, scheduled-cleanup, realtime-progress verify
   (static local; broker delivery CI-delegated per the standing
   v0.11 M3 disclosure).

   **Shipped (v0.23 M3):** `queue.ts` (a `Queue` class per broker —
   kafkajs `Kafka`/`Producer`/`Consumer` or `@nats-io/transport-node`'s
   `NatsConnection`, both genuinely lazy — plus the shared
   `publish(state, subject, payload)` free function every generated
   call site goes through, the seam a future simulation runner (M9)
   can intercept), `service.ts.j2` (seeded, classic binding-style
   `handle()` methods pipeline steps invoke — `crates/
   ciac-backend-python`/`-rust`'s own `service.py.j2`/`service.rs.j2`
   shape), `client.ts.j2` (typed HTTP clients for `call` steps, using
   the Node-global `fetch` rather than pulling in a dependency,
   unwrapping the `{status, data}` envelope and validating the
   response through the same Zod schema `schemas.ts` already
   generates), `worker.ts.j2`/`consumer.ts.j2` (NATS queue-group vs.
   Kafka consumer-group branching, the exported `handleMessageOnce`
   seam, a retry loop with no backoff — matching Python's/Rust's own
   choice exactly), `job.ts.j2` (croner, which — like Python's
   croniter and unlike Rust's `cron` crate — accepts the source
   5-field expression verbatim and owns its own scheduling loop, so
   `handleTickOnce` needs no hand-rolled sleep-until-next code at
   all), and `channel.ts.j2` (`@fastify/websocket` or a hand-rolled
   SSE stream; fan-out is broker-native — a plain NATS subscription
   naturally delivers a copy of every message to every subscriber, and
   Kafka gets a fresh per-connection consumer group instead of the
   shared queue-group workers use). `TsBackend::supports()` widened to
   `Queue`, `Stream`, `Worker` (both pipeline-bearing workers and bare
   `events X;` consumers — the same `Component::Worker` shape,
   distinguished only downstream in `ciac-codegen::model`), `Job` +
   `Scheduler`, and `Channel` + `Realtime`.

   Pipeline-step codegen (`_steps.ts.j2`, a shared minijinja macro
   `{% import %}`-ed by `route_api.ts.j2`/`worker.ts.j2`/`job.ts.j2` —
   a deliberate difference from Python's/Rust's own per-file
   duplication of the same macro, made possible and simple by
   minijinja's `import`) is real M3 work, not carried over from M1/M2:
   M1's `route_api.ts.j2` only ever emitted a hardcoded `{status:
   "accepted", data: result}` echo, correct only because `ping.ciac`'s
   pipeline is the trivial one-step `Return`. Auditing the actually-
   exercised surface before calling M3 done surfaced a real gap this
   pass had to close, not defer: `examples/inventory-system.ciac` (in
   the registry-agnostic golden suite since M2, since `crud`/`db`/
   `cache` alone were already enough to generate it) has a real `call
   Catalog.Price` pipeline step that M1/M2's stub silently ignored —
   accepted `Component::Api` unconditionally but never implemented
   `call`. Fixed by implementing the shared step macro's full
   vocabulary (`handler`/`publish`/`call`/`match`, with `return`
   deliberately a no-op — matching Python's own `emit_steps`, which
   has no arm for it either, since the envelope wrap-and-return
   happens once, unconditionally, after the whole step list) and
   `client.ts.j2`, rather than leaving `call` silently wrong now that
   `Api`/`Worker`/`Job` are gated broadly by component kind, not by
   which step kinds a given pipeline happens to use.

   Two real, live-caught bugs, both disclosed rather than smoothed
   over: (1) `client.ts.j2` imported a record's Zod schema
   (`ItemSchema`) but not the inferred TypeScript *type* (`Item`) the
   method's own return signature needed — `tsc --noEmit` on the
   generated `gateway` service failed with `Cannot find name 'Item'`;
   fixed by importing both under one line
   (`import { ItemSchema, type Item } from "../schemas.js"`). (2) Five
   real `eslint` findings across the four new examples on first run,
   not zero: `@typescript-eslint/no-explicit-any` on the `result as
   any` cast `_steps.ts.j2` needs at each step's untyped input
   boundary (a real, load-bearing rule in this project's
   `tseslint.configs.recommended`, not assumed absent — fixed with a
   documented `eslint-disable-next-line`); `no-useless-assignment` +
   `@typescript-eslint/no-unused-vars` on a worker's/job's final
   `result` reassignment, since unlike an api route a worker has no
   envelope to return it into (fixed with a trailing `void result;`);
   `prefer-const` on pipelines whose steps never reassign `result`
   (e.g. `publish X -> Return` alone) — fixed with a new Rust filter,
   `reassigns_result`, recursing into `match` arms to decide `let` vs.
   `const` per pipeline rather than always guessing `let`; and an
   unused `state` parameter in `buildApp` for `scheduled-cleanup`
   (which declares no `api`/`crud`/`channel` at all) — fixed with the
   same conditional `void state;` pattern M1 already established.
   `nats`, the plan's own illustrative NATS package name, turned out
   to be deprecated as of a recent registry check ("Package moved.
   Use @nats-io/transport-node") — pinned the real current successor
   instead of the stale name, the same real-current-versions
   discipline M1's vitest-CVE avoidance already established.

   Live proof, real toolchain: all four new examples (`event-pipeline`,
   `kafka-pipeline`, `scheduled-cleanup`, `realtime-progress`) plus
   `inventory-system` and `audited-crud` (both now exercising real
   `db`/`cache`/`service`/`call` content for the first time) pass
   `npm ci` (0 vulnerabilities), `npx tsc --noEmit`, and `npx eslint .`
   clean on the generated output. Two genuinely live, zero-mocking
   proofs beyond static verification, using the sandbox's real local
   Postgres/Redis servers (started directly, no Docker) rather than
   stopping at "it type-checks": `scheduled-cleanup`'s built
   `dist/workers/cleanup.js` — `handleTickOnce` executes the real
   seeded `PruneExpired` service through the classic-binding call path
   without throwing, and `run()` constructs a real `croner` `Cron`
   reporting the correct next 03:00 fire time; and
   `inventory-system`'s `call` step — with a real `catalog` server
   running against live Postgres/Redis, `gateway`'s compiled
   `CatalogClient.price()` made a genuine HTTP round trip, unwrapped
   the `{status, data}` envelope, and validated the response through
   `ItemSchema.parse()`. Broker delivery itself (an actual NATS/Kafka
   server publishing and being consumed) stays CI-delegated as
   disclosed — this sandbox has no local broker binary, matching the
   standing v0.11 M3 precedent exactly (Postgres/Redis do have local
   binaries here, so those *are* live-proofed rather than
   CI-deferred). Traceparent propagation is genuinely deferred, not
   silently skipped: none of the four new examples declare
   `tracing OpenTelemetry`, and Pillar 8/M7 is where TS tracing lands
   for the first time (mirroring Python's/Rust's own asymmetry —
   worker pipelines and the publish path propagate it, `channel`/bare
   `events` consumers never have). Full verification green: `cargo fmt
   --check`, `cargo clippy -D warnings`, `cargo test --workspace` (61
   suites, zero failures); 9 new/updated goldens accepted (4 new M3
   examples, `inventory-system` and `audited-crud` now generating real
   content for the first time, plus trivial `ping`/`sqlite-notes`/
   `mysql-notes` diffs from the `main.ts.j2` `buildApp` signature
   change).
4. **M4 — Typed handlers: `HostSyntax` for TypeScript.** Implement
   the ~30 leaves per Pillar 2's specs and Pillar 4's verb table;
   real transactions; builtins (`crypto.randomUUID()`, `new Date()`);
   enum use-site resolution through the shared dispatch. typed-
   handlers, typed-video, domain-orders, query-verbs, extras-verbs
   verify; the cross-backend behavioral equivalence test extends to
   three targets (including the Int-division and Json-indexing cases
   Pillar 2 flags).

   **Shipped (v0.23 M4):** `crates/ciac-backend-ts/src/lower.rs` — a
   full `TsSyntax` (`Orientation::Statement`, the same mode Python
   exercises, since a `{}` block isn't an expression in TS the way it
   is in Rust) implementing every `HostSyntax` leaf: scalar/literal
   leaves near-verbatim from Python's shapes; `if_tail` a plain
   `if {} else {}`; `match_tail` a real `switch` statement (Pillar 2's
   decision, not Python's if/elif-chain transcription); the four
   statement-shaped `db.*` verbs (`db.insert/update/delete`,
   `db.query`/`count`/`delete_where`) as raw parameterized SQL reached
   through Drizzle's `$client` escape hatch — following the *Rust*
   backend's bind-order/`sqlph` discipline, not Python's ORM-chain
   shape, per Pillar 4's explicit "adds zero new placeholder logic";
   `transaction_stmt` with **real atomicity**, exceeding Rust's
   disclosed non-atomic gap — Postgres/MySQL check out a dedicated
   connection and run `BEGIN`/`COMMIT`/`ROLLBACK` by hand (a pool's
   `.query()` alone is not transactional), SQLite uses better-sqlite3's
   native synchronous `.transaction()` wrapper. `crates/
   ciac-backend-ts/templates/logic.ts.j2` (compiler-owned
   `src/logic/<h>.ts` / seeded `src/services/<h>.ts`, mirroring
   `service.ts.j2`'s single-`state`-constructor-parameter shape rather
   than Python's/Rust's per-dependency constructor injection — already
   established since M2/M3, not a new decision). `TsBackend::supports()`
   widened `Component::Service` to accept `signature: Some(..)`
   (dropping the M1–M3 `signature: None` restriction) — `db`/`cache`
   (already open since M2) are the only capabilities a typed handler
   can actually reach this milestone.

   **Scope boundary, disclosed rather than silently narrowed:**
   `Component::ObjectStore`/`Email`/`Search`/`ExternalHttp`/`Auth` stay
   `CIAC0011`-refused. This is not an oversight — 23UpdatePlan.md's own
   capability-parity checklist (line ~759) places the S3/email/search/
   external_http *wrapper clients* at M7 and auth at M6; M4's job per
   Pillar 4's verb table is the `HostSyntax` *leaf* lowering for every
   verb (so `object_store.put`/`email.send`/`search.index`/`http.call`
   compile correctly — implemented to the trait's letter, verified by
   `cargo build`/`clippy`, using a forward-compatible `this.state.
   <camelCase>` access pattern the M7 wrapper wiring is expected to
   land under), not standing up the wrapper modules themselves. Verified
   live, not by inspection: `ciac build --target typescript` against
   `typed-handlers.ciac` (needs `object_store`), `typed-video.ciac`
   (needs `auth`), and `extras-verbs.ciac` (needs `object_store`) each
   fail with exactly `CIAC0011` and the correct capability name — the
   same disclosed-deviation pattern M2 used for `crud-notes.ciac` and
   M3 used for traceparent. `domain-orders.ciac` and `query-verbs.ciac`
   (both db-only) are this milestone's real proving examples instead,
   plus three more examples that turned out to be in-scope once typed
   handlers un-gated (`modular-video.ciac`, `sim-vertical-slice.ciac`,
   `sim-broker-slice.ciac` — none declare `object_store`/`auth`/etc.,
   discovered by running the golden suite, not hand-picked).

   Five real, live-caught bugs, all disclosed and fixed rather than
   patched around:

   1. **Duplicate `const` declarations in one block scope.** The first
      generated `domain-orders.ciac` output failed with a real
      `SyntaxError`: `PlaceOrder`'s `transaction {}` block calls
      `db.insert` twice (`Orders` then `OrderAudits`), and the initial
      `db_insert_tail` leaf declared `const __row = ..;` at the same
      scope both times. Fixed with a per-handler `Cell<u32>` fresh-name
      counter (`__row0`, `__row1`, ...) for every temp a leaf declares
      directly into the caller's block (not inside its own IIFE, which
      already has its own scope).
   2. **`import type` erasing a value used at runtime.** `PlaceOrder`'s
      `fail InvalidOrder(..)` lowers to `throw new InvalidOrder(..)` —
      a real value construction — but every `schemas.ts` import was
      blanket-emitted as `import type { X }`, which TypeScript erases
      entirely at compile time; `tsc` stayed clean (structural typing
      never needed the name) but the class reference would have been
      `undefined` at runtime. Fixed by splitting `schema_imports` into
      error records (plain `import { X }`, since `fail`/`throw new`
      needs the value) versus everything else (`import type { X }`,
      the common case).
   3. **Hardcoded `../services/` import path for typed-handler call
      sites.** `route_api.ts.j2`/`worker.ts.j2`/`job.ts.j2` imported
      every pipeline handler from `../services/<module>.js`
      unconditionally — correct for classic/`extern` handlers, wrong
      for the new compiler-owned `src/logic/` package `db_insert_tail`
      needs `checkout.ts` etc. to actually import
      `PlaceOrder`/`CreateCustomer`/`AddLineItem` from — a real
      `TS2307: Cannot find module` on the very first generated
      project. Fixed by using the already-shared, target-neutral
      `HandlerRef.handler_package` field (`"services"` vs `"logic"`,
      the exact field Python's `app.<package>`/Rust's
      `crate::<package>` already key off) instead of a literal.
   4. **A pre-existing `schemas.ts.j2` bug, latent since M2, only now
      exercised.** Every error record's `super(..)` call built its
      message from adjacent template-literal fragments joined by `+`
      — except the *last* field's fragment, which had no trailing `+`
      before the closing `` `)` `` literal. Two adjacent template
      literals with nothing between them is valid JS syntax with a
      different meaning (a tagged template: the first literal used as
      a *function* tagging the second) — `tsc` caught it as `Type
      'String' has no call signatures`, not a syntax error. Latent
      since M2 (`schemas.ts.j2` has generated `is_error` classes since
      then) because no earlier TS-supported example had ever declared
      an `error` record with fields until `domain-orders.ciac`'s
      `InvalidOrder`. Fixed by joining every field fragment with `+`
      unconditionally, including the last.
   5. **Structural typing silently dropping an import.** `OrderAudit`
      (a record-construction target inside `PlaceOrder`, never bound
      to a typed local) triggered `@typescript-eslint/no-unused-vars`
      even though `record_cons` genuinely constructs one: an
      *unannotated* object literal never spells its record's name
      anywhere, so the import genuinely went unused by TS's structural
      typing. Fixed by wrapping every `record_cons` result in
      `satisfies {record_name}` (not `as`, which would widen away the
      literal field types) — a real correctness improvement (the
      constructed shape is now checked against its declared record),
      not just an eslint placation.

   One more real design point, resolved rather than assumed: a HIR
   `Let` bound to an `if`/`match` expression threads the same
   `Dest::Assign(name)` into every branch, and unlike Python (whose
   `if`/`else` share the enclosing scope), TS's `if {} else {}`
   introduces a real block scope — a `let`/`const` declared *inside*
   each branch would be invisible after the block. Resolved by having
   `render()` scan the HIR once for exactly the `Let`s whose value is
   an `if`/`match` (`collect_branching_lets`), hoisting only those into
   one `let` declaration above the branch and using a bare `name =
   value;` at each branch's own assignment site; every other `Let`
   (the common case — a straight-line value with no branching) gets a
   plain `const name = value;` at its one assignment site instead.
   Getting this wrong either way is real, live-caught: an unconditional
   hoist tripped `eslint`'s `prefer-const` on `query-verbs.ciac`'s
   `Replace` (`let n = Note {...}` never branches), the naive
   unconditional non-hoisted form would have shipped a real scoping bug
   for the branching case (unexercised by any of this milestone's own
   examples, but exercised for real once `typed-handlers.ciac`'s
   `let ready = if inserted.status == Pending {...} else {...};`
   un-gates at M7).

   A sixth fix, one line, in the shared (not TS-only) `models.ts.j2`:
   conformance's C4b (every declared topology fact — including a raw
   `table` declaration's own name — must appear verbatim in every
   supporting target's output) failed for `domain-orders.ciac` once TS
   newly supported it, because `table` declarations were exported as
   `{{ table.snake }}Table` (e.g. `customersTable`) rather than the
   declared name itself. Fixed to export `{{ table.class_name }}`
   (`Customers`), matching Python's `class Customers(Base):`/Rust's
   `struct Customers` naming exactly — CRUD resources' own `<snake>
   Table` convention is untouched (C4b only checks `table` decls, and
   CRUD resource names were never part of this fact set).

   Live proof, real toolchain and real local infrastructure, not
   assumed: `domain-orders` (Postgres) and `query-verbs` (SQLite) both
   pass `npm ci` (0 vulnerabilities), `npx tsc --noEmit`, `npx eslint .`,
   `npx vitest run` clean; `modular-video`/`sim-vertical-slice`/
   `sim-broker-slice` verify statically the same way. Two genuinely
   live, zero-mocking proofs beyond static verification:

   - **`domain-orders`' transaction rollback, against a real local
     Postgres server.** `POST /orders` with a negative `total` returns
     500 (`InvalidOrder`), and both the `Orders` write that happened
     *before* the failing `if` and the never-reached `OrderAudits`
     write are absent afterward (`SELECT count(*)` on both tables:
     `0`) — the dedicated-connection `BEGIN`/`ROLLBACK` genuinely
     undoes a write that already succeeded, exactly the "real
     atomicity, exceeding Rust's disclosed gap" claim, not just parsed
     syntax. A second `POST /orders` with a positive total commits
     both writes (`1` row each); a direct `DELETE FROM customers`
     against the referenced customer fails with a real Postgres FK
     violation (`on_delete: restrict`), proving v0.16's relations
     still enforce correctly through raw SQL.
   - **`query-verbs`' full verb set, against a real local SQLite
     file.** Two rows seeded directly into the file, then `db.query`/
     `db.count` (both with a `Bool` predicate — the sqlite `0/1`↔
     `boolean` coercion `bind_expr`/`map_row_field` add is what makes
     `active: true`/`false` round-trip as real JS booleans, not raw
     integers), `db.update` (renaming a row and flipping its `active`
     flag), `db.delete_where` (bulk delete, returns the correct
     affected-row count), and `db.delete` (single-key, `false` on a
     nonexistent id) all round-trip correctly over real HTTP requests
     against the real file.

   Explicitly not attempted this pass, disclosed rather than silently
   skipped: a generated per-handler behavioral unit test analogous to
   Python's `render_test` (mocked-dependency assertions on call
   counts) — real, substantial additional scope with its own mocking
   machinery; the live HTTP round-trips above cover the same ground at
   the system level instead. `MySQL`-engine typed handlers compile
   (every `db_engine` branch is written and type-checked) but aren't
   live-proofed this pass — no locally-tested example binds a typed
   handler to a MySQL instance; matches the standing MySQL
   live-proof-deferred-to-CI precedent from M2/M3.

   The cross-backend equivalence test (`tests/tests/
   typed_handler_equivalence.rs`) extends to three targets via a
   second canonical example (`DIVISION_EXAMPLE`, `db`-only — the
   original `CANONICAL_EXAMPLE` needs `object_store`, which TS can't
   join this milestone without changing what it tests) asserting the
   two named divergence cases directly: `Int / Int` (Python's `/`
   stays true division; Rust's native `/` and TS's `Math.trunc(a / b)`
   both truncate toward zero) and `Json` indexing (Python's bare
   `base[key]`; TS's optional-chained access plus an explicit thrown
   `KeyError`-shaped error, since JS has no equivalent built-in) — a
   disclosed, pragmatic scope reduction from the "specified" section's
   full JSON-fixture/three-runner-mechanism design, which doesn't
   exist yet even for the two established targets; growing today's
   real, working structural-parity mechanism to a third target is the
   concrete step available now.

   Full verification green: `cargo fmt --check`, `cargo clippy -D
   warnings`, `cargo test --workspace` (all suites, zero failures,
   including conformance's C3/C4a/C4b now checking TS alongside
   Python/Rust for every example all three support); 5 new/updated
   goldens accepted (`domain-orders`, `query-verbs` new; `modular-video`,
   `sim-vertical-slice`, `sim-broker-slice` newly generating real TS
   content now that typed handlers are open).
5. **M5 — CHECKPOINT: factory acceptance + go/no-go.** Measure
   non-template LOC and template LOC against 22UpdatePlan.md M6's
   cost model and Pillar 8's template estimate; run the conformance
   harness across python/rust/typescript (OpenAPI byte-equality,
   topology equality, validators). Publish the comparison in this
   file. If the factory's numbers missed materially, STOP — fix
   22's deliverables before Go/Java consume them; "pause and amend
   the factory" is a valid, planned outcome, and this checkpoint
   existing is the reason TS goes first.

   **Shipped (v0.23 M5) — the measured cost table**, against
   `docs/backends.md`'s own "What a backend costs today" baseline
   (Python's/Rust's post-factory numbers) and this plan's own Pillar 8
   template estimate (~33 templates, ~2,700–3,000 lines):

   | | Rust (post-factory) | Python (post-factory) | TypeScript (measured, M1–M4) |
   | --- | --- | --- | --- |
   | `lower.rs` (leaves + `render`) | 577 | 869 (incl. ~320-line `render_test` family) | **1,098** |
   | `lib.rs` (emission wiring) | ~509 | ~374 | **501** |
   | `filters.rs` (neutral-field mapping) | n/a (folded into `lower.rs`) | n/a | **206** |
   | templates | ~2,800 (audit baseline) | ~2,800 (audit baseline) | **5,608** across 28 files (Pillar 8 estimated ~33 files / ~2,700–3,000 lines for the *full* arc, M1–M9) |
   | edits outside the crate | 1 (registry line) | 1 (registry line) | **1** (`crates/ciac/src/commands.rs:25`, held by the same grep-fence test) |

   **The `lower.rs` overrun, measured and explained, not hand-waved:**
   159 of TS's 1,098 lines are doc comments (vs. 67 Python / 71 Rust —
   this session's disclosure-heavy commenting style, consistent with
   every prior milestone's notes, accounts for a real but partial
   share); net of comments and blank lines, TS's leaf code is still
   ~909 lines against Python's ~802 and Rust's ~506. The remaining gap
   has one concrete, structural cause rather than a design mistake:
   Rust's `sqlx` gives every engine (`Postgres`/`MySQL`/`SQLite`) the
   *same* call shape (`sqlx::query(..).bind(..).execute(self.db)
   .await?`) and only the SQL *text* varies (`sqlph`'s placeholder
   rewrite); the Node ecosystem has no `sqlx`-equivalent unifying
   driver, so `pg`/`mysql2`/`better-sqlite3` each have genuinely
   different call shapes at every verb site (sync `.prepare().run()`
   vs. async `.query()`, `[rows]` array destructuring vs. `.rows`,
   `.changes` vs. `.rowCount` vs. `affectedRows`) — a real 3-way branch
   Pillar 4's own text anticipated ("adds zero new placeholder logic")
   but that undersold the call-shape divergence specifically. On top
   of that, TS's raw-driver reads/writes need explicit per-field
   type coercion for SQLite (`boolean`↔`0/1`, `Date`↔`TEXT`,
   `object`↔`TEXT`-as-JSON — live-verified necessary: better-sqlite3
   rejects a bare JS `boolean` bind param outright) that neither
   Python's ORM (SQLAlchemy maps types itself) nor Rust's `sqlx`
   (compile-time-checked `FromRow`/bind traits) need hand-written.
   Both are genuine, disclosed, ecosystem-shaped costs — not scope
   creep, and not something a future milestone should try to "fix"
   away, since the alternative (a bespoke query-builder abstraction
   unifying the three drivers) would be strictly more code and more
   risk than the current straightforward-if-verbose per-engine
   branching.

   **The templates overrun, measured and explained:** 5,608 lines
   across 28 files already exceeds Pillar 8's ~2,700–3,000-line
   estimate for the *entire* arc (M1–M9, ~33 files) at only 28/33
   files landed. Comments/blank lines account for a small share (130
   comment lines + 116 blank lines, ~4.4% of the total) — this is a
   real, measured estimate miss, not a comment-padding artifact. Read
   against `docs/backends.md`'s own established baseline (~2,800
   lines/backend for Python's/Rust's *complete*, mature template sets)
   rather than Pillar 8's own pre-registered guess, TS's 5,608 lines
   for 28 files is proportionally still ahead of pace (Python/Rust
   land their full provider surface — auth, all five ontology
   capabilities, sim — in ~2,800 lines total; TS is at double that
   with auth/ontology/sim still unbuilt) — a genuine cost-model miss
   worth flagging plainly rather than reconciled away with a
   different denominator.

   **Conformance harness, run for real across all three targets:**
   `cargo test --workspace` (including `tests/tests/conformance.rs`'s
   `c3_openapi_is_byte_identical_across_targets`,
   `c4a_migration_sql_is_byte_identical_across_targets`, and
   `c4b_declared_topology_appears_in_every_target`) is green with
   TypeScript registered — the moment M1 added the registry line, C3/
   C4 began checking TS's OpenAPI/migration-SQL/topology output
   against Python's and Rust's for every example all three support,
   catching two real bugs this same milestone (the `models.ts.j2`
   table-naming gap C4b caught, and the transitively-exercised handler
   import paths C3's byte-identity would have caught downstream had
   `tsc`/`eslint` not already caught it first). C1/C2/C5 (this plan's
   own numbering, inherited from 22UpdatePlan.md's prose) have no
   dedicated named test functions the way C3/C4a/C4b do — they're
   satisfied by the pre-existing golden/gating/blueprint/determinism/
   modules suites, all green with TS included since M1's `full_parity_
   backends()` split (documented in M1's own shipped notes) correctly
   scopes the Python/Rust-only assertions away from TS-inclusive ones.
   `ciac targets --json` lists `typescript` with `capabilities: {}`
   (unchanged from M1 — still correctly empty, since `vocab::PROVIDERS`
   lists only python/rust per 22 M4's own disclosed disposition, not
   yet extended this arc).

   **Go/no-go verdict: GO.** Nothing measured here is a capability gap,
   a correctness gap, or a structural blocker — every miss is a line-
   count overrun with a concrete, disclosed, ecosystem-shaped cause
   (no `sqlx`-equivalent unifying driver in Node; more explicit
   per-engine branching than either existing backend needs). The
   factory's *structural* promise — `TargetInfo`, the backend-owned-
   filter pattern, the shared scanner, the shared `HostSyntax`
   dispatcher, the conformance harness, `Emit`/skeleton — held exactly
   as `docs/backends.md`'s handoff section described: a third backend
   really did consume all of it unchanged, and the map's brevity (one
   registry line) really did stay the acceptance test. What the
   factory's *line-count* promise underestimated is real and should be
   carried forward as a corrected budget for 24/25UpdatePlan.md (Go/
   Java), not discovered again the hard way: expect a `lower.rs`
   noticeably larger than Rust's 577-line figure whenever the target
   language's database ecosystem lacks a `sqlx`-equivalent unifying
   driver (true for Go's `database/sql` + per-engine driver split too
   — worth Go's own author checking this before, not after, writing
   its `lower.rs`), and expect the full template set to land closer to
   ~5,500–6,000 lines than the ~2,800-line Python/Rust baseline once
   auth/ontology/sim are in. 24UpdatePlan.md may proceed; its own cost
   table should cite these corrected figures, not Pillar 8's original
   estimate, as its starting budget.
6. **M6 — Auth, scopes, scope tests.** jose HS/RS + JWKS,
   requireScope hooks, generated `tests/scope.test.ts` via
   fastify.inject (JWT-only, standing OAuth2 exclusion comment).
   order-system verifies with the suite green under zero
   infrastructure; oauth-echo verifies statically with the
   documented OAuth2 posture.
7. **M7 — Ontology remainder + call clients + observability
   completion.** S3/email/search/external_http wrappers, typed call
   clients (reuse-or-fork decision recorded), OTel end-to-end with
   the three-target trace test, metrics endpoint. multi-service-media,
   inventory-system, ontology-growth, traced-checkout, dev-identity
   verify; `--system` CI rows added (typescript × inventory-system,
   × mysql-notes, × sim-vertical-slice) with the standing
   Docker-delegation note.
8. **M8 — Whole-repo integration.** Every example either verifies or
   is explicitly reason-gated (target: zero gates — TS supports the
   full provider table); golden suite complete; generated docs
   tables regenerate; `ciac dev` session test against a TS project;
   MCP verify/build exercised; evolution/rename-replay exercised
   against a TS tree (migrations_dir path resolution — identity
   here, but the test guards the factory's mapping for Java);
   `generated-typescript` CI job (npm-cached).
9. **M9 — Simulation slice (gated bet) + version + retrospective.**
   Pillar 9's slice: world.ts, world-guards, sim_runner.ts template,
   `SimSupport::Narrow` wiring, both canonical scenario outcomes
   reproduced exactly, refusal case verified, ratchet CI row,
   docs/simulation.md + backends.md updated. Workspace version bump;
   whole-arc analysis including the factory-cost verdict that gates
   24UpdatePlan.md.

### Per-milestone exit checklists

- **M1 exits when:** the registry entry is the crate's only external
  edit (asserted in review against the implementation map); ping
  passes the full validator sequence live; the no-infra AppState
  test exists and passes; goldens for ping committed; the grep-fence
  test still passes.
- **M2 exits when:** all three engines' CRUD/keyed-store goldens
  exist; sqlite-notes verifies live with zero Docker; conformance C3
  (OpenAPI) passes for every M2-supported example ×3 targets; the
  boundary-decode cases for zod (absent vs null vs zero, 2^53±1)
  pass.
- **M3 exits when:** the four broker/schedule examples verify;
  `handleMessageOnce`/`handleTickOnce` exports exist (a unit test
  imports them — the seam is load-bearing, so it is tested, not
  assumed); traceparent headers appear in publish goldens.
- **M4 exits when:** every verb row of Pillar 4's table has a golden
  exercising it; the equivalence suite passes ×3 including the
  division/Json-indexing/Option cases; domain-orders' transaction
  rollback proof passes against real sqlite locally.
- **M5 exits when:** the measured-vs-promised cost table is
  committed in this file; conformance C1–C5 green ×3 on all
  supported examples; an explicit go/no-go sentence for 24 is
  recorded.
- **M6 exits when:** the scope suite passes with zero infrastructure
  on order-system; the OAuth2 exclusion comment matches the Rust
  template's sentence (parity of disclosure is checked textually).
- **M7 exits when:** all ontology examples verify; the trace test
  passes ×3; the three `--system` CI rows are merged (execution
  Docker-delegated, honestly noted).
- **M8 exits when:** zero gated examples remain (or each gate names
  its reason in `supports()` comments); dev/MCP/evolution session
  transcripts are attached to the milestone notes;
  `generated-typescript` is green in CI.
- **M9 exits when:** both scenarios reproduce the canonical outcomes
  byte-for-byte in `SimScenarioOutcome`; the refusal case names auth
  + each unguarded verb; docs tables updated; version bumped;
  retrospective written.

## The behavioral equivalence suite, specified

Referenced by M4 and inherited by plans 24/25, so its shape is fixed
once: for a curated set of handler bodies (drawn from
typed-handlers, query-verbs, extras-verbs, and the boundary cases
this arc adds), the suite runs the *same logical inputs* through
each target's generated handler and asserts the same observable
outcomes — return values (as JSON), thrown/raised error identity,
and effect sequences against each target's test doubles. Python
executes via the existing pyrunner harness; Rust via its generated
tests; TS via vitest against the generated logic modules. The
curated case list is data (a JSON fixture), so adding a case adds it
for every target at once. Known-divergence cases (Int width, float
formatting at extremes) are asserted-as-documented rather than
skipped — the suite encodes the divergence ledger, so an undocumented
divergence is a failure and a documented one is pinned. This suite
is deliberately narrow (logic layer only); wire-level equivalence
belongs to C3/C4 and HTTP-behavior parity to the smoke/system
tests — three layers, no overlap, named coverage.

## Verification strategy

Per milestone: `cargo fmt --check` / `clippy -D warnings` /
`cargo test --workspace` green; goldens reviewed diff-by-diff, never
blind-accepted; live proofs as named per milestone with the repo's
standing honesty rule — anything needing Docker is CI-delegated and
said so in the milestone's shipped notes. Standing new assertions
this arc adds: OpenAPI cross-target byte-equality ×3, the
three-target behavioral equivalence suite for typed handlers, the
no-infra AppState construction test, the Int boundary-value decode
test, and M9's exact-outcome sim equality. Determinism: generated
package-lock.json is golden-snapshotted (the lockfile is part of the
generated artifact, pinning the full dependency tree); `npm ci` (not
`install`) everywhere; exact versions (no `^`) in generated
package.json — the same exact-pin rule every other backend's build
file follows.

## Open questions resolved at implementation (pre-registered)

The v0.17 arc's discipline: name the decisions deliberately deferred
to implementation-time evidence, so they are reconciled-then-decided
rather than improvised. This plan pre-registers four:

1. **`workers_command` dev-vs-dist shape** — `node dist/workers.js`
   assumes the image build ran tsc; decided against the final
   Dockerfile stage layout in M1, recorded in the template comment.
2. **Call-client reuse vs fork** (Pillar 7) — decided in M7 by
   diffing the v0.15 client's generated methods against the server
   call sites' needs; either outcome is documented in the template
   header.
3. **Drizzle table objects vs plain row mappers for reads** — if
   Drizzle's type inference fights the generated `models.ts` shape
   in practice, the fallback is plain typed row-mapping functions
   over the raw drivers (the Go RowMapper shape); decided in M2
   with the reason recorded. The SQL text is identical either way,
   so goldens localize the decision to one file.
4. **Fastify plugin-vs-closure state injection** — the sketch passes
   `state` via plugin options; if encapsulation friction appears at
   M1, the alternative is module-level factory functions (the Go
   shape). Golden-localized, decided once.

## Explicit cuts

No Deno/Bun providers. No Prisma/NestJS compatibility layer. No
`bigint` Int mode. No CJS output or dual builds — ESM only. No
socket.io. No sim record/replay for TS (matches Rust's disclosed
gap). No monorepo/workspace layout for multi-service TS systems
beyond the per-service-directory shape all targets share. No
GraphQL, no tRPC — the API surface is the declared REST contract,
same as every target. No attempt to unify the generated browser
client and server call client if their needs diverge (fork is
allowed, documented).

## Risks

- **npm supply-chain / version drift breaks goldens or CI.**
  Lockfiles generated and snapshotted; exact pins; `npm ci` only.
  Dependency upgrades become deliberate, golden-visible changes —
  the same posture as Cargo.toml pins.
- **kafkajs maintenance risk.** Named at selection; the queue module
  is one file behind the same seam as Rust's `Queue`; confluent's
  client is the recorded fallback.
- **`Int` fidelity complaints.** The disclosure is loud, tabled in
  docs/language.md, boundary-tested, and the decision record names
  its revisit condition.
- **Fastify major-version churn** (v5 is current; the ecosystem
  moves). Exact pins + goldens make upgrades deliberate; Hono is the
  recorded fallback if governance falters.
- **Factory shortfall discovered at M5.** By design — that is the
  checkpoint doing its job, and the outcome path (amend 22, then
  resume) is pre-agreed rather than improvised.

## Confidence and handoff

High. Every hard problem in this plan has a solved twin in the repo:
placeholder discipline (v0.13 M1), broker mapping (v0.11 M3), JWKS
laziness (v0.17 M11, here met by `jose`), scope-test injection
(v0.14 M6 + the oneshot precedent), the sim slice (v0.17 M11's
continuation, mirrored step by step), and the checkpoint-gated
rollout shape itself (v0.17 M5). The handoff to 24UpdatePlan.md (Go)
is the M5 checkpoint's measured cost report plus any `HostSyntax`
contract notes TS forced (none are expected from TS; Go pre-declares
its error-idiom amendment) — Go begins by reconciling against those
actuals, exactly as v0.17 M1 reconciled against real v0.16 output
rather than its own plan's prose.
