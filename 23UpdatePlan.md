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
2. **M2 — Records, schemas, models, CRUD, keyed store, migrations.**
   schemas.ts (zod + enums + error classes), models.ts (drizzle
   tables), db.ts (engine-keyed pools + migration runner), typed
   CRUD and the keyed-document store across Postgres/MySQL/SQLite
   with the shared placeholder/bind discipline. Live proof:
   sqlite-notes fully local with zero Docker (file-backed engine —
   the same zero-infra proof v0.13 M3 used); crud-notes/mysql-notes
   verify statically local, capability round-trips CI-delegated as
   always.
3. **M3 — Broker, workers, jobs, channels.** nats.js + kafkajs,
   worker retry loop delegating to exported `handleMessageOnce`,
   croner jobs with `handleTickOnce`, WS/SSE channels, publish sites
   through `state.publish`, traceparent headers. event-pipeline,
   kafka-pipeline, scheduled-cleanup, realtime-progress verify
   (static local; broker delivery CI-delegated per the standing
   v0.11 M3 disclosure).
4. **M4 — Typed handlers: `HostSyntax` for TypeScript.** Implement
   the ~30 leaves per Pillar 2's specs and Pillar 4's verb table;
   real transactions; builtins (`crypto.randomUUID()`, `new Date()`);
   enum use-site resolution through the shared dispatch. typed-
   handlers, typed-video, domain-orders, query-verbs, extras-verbs
   verify; the cross-backend behavioral equivalence test extends to
   three targets (including the Int-division and Json-indexing cases
   Pillar 2 flags).
5. **M5 — CHECKPOINT: factory acceptance + go/no-go.** Measure
   non-template LOC and template LOC against 22UpdatePlan.md M6's
   cost model and Pillar 8's template estimate; run the conformance
   harness across python/rust/typescript (OpenAPI byte-equality,
   topology equality, validators). Publish the comparison in this
   file. If the factory's numbers missed materially, STOP — fix
   22's deliverables before Go/Java consume them; "pause and amend
   the factory" is a valid, planned outcome, and this checkpoint
   existing is the reason TS goes first.
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
