# CIaC v0.19 — The Correctness Version: Outbox, Idempotency, Ownership Policies, Architecture Lints (roadmap forecast)

> Forecast document. Assumes v0.16 (domain — this version's outbox
> rides its transactions, its policies ride its relations), v0.17
> (simulation — every guarantee here gets a deterministic fast
> proof), and v0.18 (evolution — policy and idempotency changes are
> contract changes the semantic diff must classify) have landed.
> Direction-setting; the v0.19 planning pass finalizes the policy
> surface syntax and the outbox relay semantics. **Confidence
> labels**: outbox and idempotency are *structural* — the bugs they
> eliminate demonstrably exist in today's generated output; ownership
> policies are *structural* for the multi-tenant audience;
> architecture lints are a *cheap experiment* riding existing
> machinery.

## The gap this version closes

Ciac's founding pitch is moving failure modes to compile time. Three
failure modes that define "production-grade" remain not just
unchecked but, in two cases, actively *generated*:

1. **Ciac generates the dual-write bug today.** `pipeline Upload:
   StoreVideo -> publish Uploaded -> Return` compiles to a database
   write in the handler followed by a broker publish — two systems,
   no transactional link. Crash between them: the video exists and no
   consumer will ever hear about it. Publish first instead: consumers
   hear about a video that doesn't exist. This is the textbook
   distributed-systems footgun, it is present in essentially every
   example in the repo, and v0.16 deliberately made it *loud*
   (publish-inside-transaction is rejected with a diagnostic that
   names this version). No framework can fix it because no framework
   sees both the write and the publish; ciac sees the whole pipeline.
2. **At-least-once delivery corrupts non-idempotent handlers.** The
   generated broker machinery (NATS queue groups, Kafka consumer
   groups) redelivers on failure — that's correct — and the generated
   worker retry loop (`max_retries`, v0.3) multiplies it. A worker
   that charges a card or increments a counter processes duplicates
   today, and nothing in the language lets the author even *declare*
   that it must not.
3. **Scopes answer the wrong question for multi-tenant data.** v0.14
   M6's `scope`/`read_scope`/`write_scope` answer "may this token
   call this route" — enforced, tested, done. Nothing answers "may
   this token touch *this row*". `GET /orders/{id}` with
   `orders:read` returns anyone's order to any authenticated
   customer. Every real SaaS hits this wall in week one, and the
   workaround (hand-written filters in seeded handler code, per
   route, per backend) is exactly the kind of repeated, mechanical,
   security-critical code a compiler should own. Honestly noted in
   planning: this is arguably *domain expressiveness* and could have
   ridden v0.16 — it sits here only to keep that version shippable,
   and it is the first candidate to pull forward if usage screams.
4. **The graph knows about design mistakes it never mentions.**
   An unindexed `where` predicate (O(n) scan), a cross-service `call`
   inside a worker's retry loop (retry storms), a publish fan-out
   chain five hops deep — all are visible in the `SystemGraph` today
   and diagnosed never. The reachability/cycle/auth-placement passes
   (v0.1) established compile-time architecture review; it simply
   stopped growing.

**v0.19 theme: classes of production bugs become inexpressible or
loudly declared — correctness by construction, which only the
component that sees the whole graph can offer.**

## Pillar 1 — The transactional outbox

`publish` inside a `transaction { }` (and, with an explicit opt-in,
the pipeline-level write-then-publish shape) compiles to the outbox
pattern instead of a raw broker call:

- **Mechanics, both backends**: an `_outbox` table (migration-managed
  like any other; id, subject, payload, created_at, published_at
  nullable) in the same database instance as the transaction; the
  publish verb inside the transaction lowers to an INSERT into
  `_outbox` — atomic with the business writes by construction. A
  generated **relay task** in the existing workers process
  (`app/workers.py` / the `workers` bin — the process every deployable
  with async work already runs) polls unpublished rows in insertion
  order, publishes to the real broker, marks published. At-least-once
  end to end; pairs with Pillar 2 on the consumer side.
- **Semantics, stated honestly in docs**: this is at-least-once with
  idempotent consumers, not exactly-once; ordering is per-outbox
  insertion order, not global. The relay's poll interval, batch size,
  and retention are generated config (env-overridable like
  everything in `config.py.j2`/`config.rs.j2`).
- **Surface**: no new syntax. The v0.16 diagnostic arm ("publish
  inside a transaction requires the outbox — planned v0.19") flips
  from *reject* to *lower*. Pipeline-level adoption is an attribute
  (`api Upload { publish: transactional; }`-shaped, final syntax in
  the planning pass) so existing programs' behavior never changes
  silently — the semantic diff (v0.18) classifies turning it on as
  `internal`, turning it off as `breaking`.
- **Proof bar**: a sim (v0.17) scenario — crash injected between
  business write and relay publish, restart, assert the event
  arrives exactly the consumers expect; and a live `verify --system`
  check: kill the app container mid-request (compose `kill`),
  restart, assert delivery. The sim's deterministic crash point is
  the test Docker could never express precisely.

## Pillar 2 — Idempotency

- **API side**: `api Charge: Payment { idempotent: true; }` — the
  generated route requires an `Idempotency-Key` header, stores
  key→response in the service's db instance (or cache with TTL when
  the service has one and the author opts in), and replays the stored
  response for a repeated key. 409 on a key reused with a different
  payload (the Stripe semantics, which are the de-facto standard and
  say so in docs).
- **Worker side**: `worker Ship on Placed { idempotent: true; }` —
  the generated consume loop records processed message ids
  (broker-assigned or payload-derived, per-engine detail behind the
  existing engine switch) and skips duplicates *before* invoking the
  handler; composes with `max_retries` (a retried failure is not a
  duplicate; a redelivered success is).
- Sema: `idempotent` on a worker whose service has neither db nor
  cache is a missing-capability diagnostic with a structured fix
  (v0.15 M7 machinery: "add `db Postgres;`…"). The generated
  scope/behavioral suites gain the replay tests (same key twice →
  identical response, one side effect — provable against the sim's
  fake store in microseconds).

## Pillar 3 — Ownership policies

- **Surface** (final syntax in the planning pass; the shape):

  ```ciac
  crud Order: Order {
      read_scope: "orders:read";
      owner: customer;          // a ref (or Uuid) field on Order
  }
  ```

  `owner:` names a field on the resource's record; the generated
  routes then enforce **row ownership against the token subject**:
  list returns only the caller's rows (the filter is compiled into
  the SQL, not applied after fetch), get/update/delete on another
  owner's row returns **404, not 403** (no existence oracle —
  decided, documented, tested), create stamps the owner from the
  token, and the owner field is not client-writable.
- **Compile-time checks**: `owner` requires an `auth` capability
  (same gate as scopes); the field must exist and be `Uuid`/`ref`
  typed to match the token subject; **exhaustiveness is the point** —
  every generated route on the resource is covered by construction,
  and the v0.14 typed-handler verbs (`db.get`/`db.query`/…) against
  an owned record inside that service's handlers get the filter
  injected in verb lowering too, so seeded logic can't accidentally
  bypass what the routes enforce. An owned record reachable through
  an *unowned* crud on the same data is a new diagnostic (one record,
  one ownership story).
- **Admin escape hatch**: `owner_bypass_scope: "orders:admin"` — a
  declared scope that lifts the filter, so back-office routes are a
  declaration, not a hand-carved hole.
- **Proof bar — the v0.15 M6 payoff**: the `users Keycloak` dev
  realm already ships two users (`dev-admin`/`dev-user`) and
  `scripts/token.sh`. The generated system suite gains cross-tenant
  isolation checks: user A creates an order, user B's token gets 404
  on it, B's list doesn't contain it, admin-scoped token sees both —
  real tokens, real IdP, real rows, in `verify --system`; the same
  assertions run against the sim's fake JWKS in the inner loop.
- Semantic diff (v0.18): adding `owner` to an existing resource is
  `breaking` (visible rows narrow); the classification table gains
  the policy rows.

## Pillar 4 — Architecture lints

Advisory warnings (never errors) from graph analysis, riding the
existing pass infrastructure (`ciac-sema/src/passes/`) and the v0.15
M7 fix machinery where the remedy is mechanical:

- **unindexed-predicate**: a v0.14 `where` predicate or `db.query`
  filter on a field with no v0.16 index → warning with a structured
  fix inserting the `index` declaration.
- **call-in-retry**: a cross-service `call` reachable inside a
  worker pipeline with `max_retries > 0` → retry-storm warning
  (suggests idempotency on the callee or moving the call).
- **fanout-depth**: publish→worker→publish chains beyond a
  documented depth → amplification warning.
- **write-then-publish**: the Pillar 1 shape *not* opted into the
  outbox → the dual-write warning, with a fix adding the attribute.
  (The lint is the migration path: existing programs warn, new
  programs opt in, a future major version can flip the default.)
- **missing-idempotency**: a worker with `max_retries > 0` and
  side-effecting verbs, not marked `idempotent` → duplicate-effects
  warning.
- **Suppression is declared, not configured**: `allow:
  call_in_retry;` as an attribute on the construct — visible in the
  source and the snapshot, greppable, diffable. No lint config file.
- Every lint ships with: a fixture pair (fires / doesn't), an
  `explain` entry, and a false-positive budget — a lint that annoys
  in practice gets demoted to off-by-default rather than defended.

## Secondary items

- `docs/correctness.md`: the delivery-semantics contract
  (at-least-once + idempotent consumers), the ownership model, the
  404-vs-403 decision, the lint catalog.
- MCP: `lint` output rides the existing `check` envelope (lints are
  diagnostics); no new tool needed — the fix tool already applies
  their remedies.
- `vocab.rs`/`describe`: new attributes (`idempotent`, `owner`,
  `owner_bypass_scope`, `allow`), lint codes.
- Flagship: `commerce.ciac` (v0.16) gains ownership + idempotent
  checkout + transactional outbox — one example demonstrating the
  whole correctness story, golden- and system-covered.

## Milestones

1. **M1 — outbox, Python**: table migration, verb lowering flip,
   relay task, config; sim crash-point proof + live compose-kill
   proof.
2. **M2 — outbox, Rust**: same bar; per-engine SQL through the
   existing placeholder/engine matrix.
3. **M3 — idempotency**: api + worker, both backends, replay tests in
   the generated suites, sema gates + fixes.
4. **M4 — ownership policies**: sema checks + verb-lowering
   injection + route enforcement, both backends; the two-dev-user
   cross-tenant live proof; semantic-diff classification rows.
5. **M5 — lints**: the five lints above, fixtures, structured fixes,
   suppression attribute, docs catalog.
6. **M6 — flagship, docs, 0.19.0**: `commerce.ciac` upgrade,
   `docs/correctness.md`, README pitch ("correct under partial
   failure, by construction"), full verification, version bump, arc
   notes. Per-milestone discipline throughout.

## Risks

- **Outbox relay becomes a bottleneck/ordering surprise.**
  Mitigation: semantics documented bluntly (at-least-once, per-outbox
  order); batch/interval tunable; the sim proof pins behavior; scale
  concerns are explicitly out of scope (it's a correctness feature,
  not a throughput one — said in docs).
- **Policy bypass through seeded code.** The sharpest risk in the
  version. Mitigation: verb-lowering injection covers the closed verb
  set; the boundary is documented with a bright line — *raw SQL in
  seeded files is outside the policy guarantee* — and the lint pass
  flags owned-record table names appearing in seeded SQL strings
  (best-effort, advisory, honest about being best-effort).
- **404-vs-403 debates.** Decided once (404, no existence oracle),
  documented with the reasoning, consistent across backends — the
  mitigation for bikeshedding is having already chosen.
- **Lint fatigue.** Mitigation: five lints, not fifty; each with a
  false-positive budget and a demotion path; suppression is one
  attribute away and visible in review.
- **Idempotency storage growth.** Mitigation: TTL'd by default
  (configurable), documented retention, and the storage choice
  (db table vs cache) is the author's declared call.

## Cut lines

- Exactly-once delivery: not claimed, not attempted; the docs say
  "at-least-once + idempotent consumers" in the first paragraph.
- Sagas / compensation workflows / distributed transactions across
  services: a future version's question, only with usage evidence.
- Full policy language (attribute-based access control, role
  hierarchies, OPA-style rules): `owner` + scopes + bypass scope
  covers the multi-tenant 90%; the closed surface is the feature.
- Row-level security via database RLS engines: the enforcement lives
  in generated code (portable across engines including SQLite);
  native RLS is an implementation option to revisit, not a surface.
- Lint plugins / user-defined lints.

## After v0.19

The generated system is now expressive (v0.16), instantly verifiable
(v0.17), safely evolvable (v0.18), and correct under partial failure
and multi-tenancy by construction (v0.19). The remaining unclosed
loop is the outermost one: when the *running* system misbehaves,
nothing maps the failure back to the `.ciac` line that generated the
failing behavior. v0.20 — provenance — is that map.
