# CIaC v0.19 — Correctness Under Failure: Outbox, Idempotency, Ownership, and Architecture Lints (roadmap forecast)

> Forecast document. Assumes v0.16 database transactions, v0.17's
> deterministic simulator/failure model, and v0.18 semantic diff/rename
> have landed. The v0.18 tools are not part of the outbox state machine,
> but the nominal arc uses them to classify and review every new v0.19
> guarantee. Direction-setting; the implementation planning pass freezes
> the outbox lease protocol, message envelope, idempotency state machines,
> ownership-subject provenance, and lint definitions before backend work.
>
> This is not a deployment-maturity version. It adds compiler-owned runtime
> machinery needed to uphold language guarantees, not a control plane,
> cloud broker/database provisioning, rollout orchestration, autoscaling,
> dashboards, or recovery UI.

## The gap this version closes

The generated systems contain production correctness bugs that can be
demonstrated from current output.

### The dual-write window

A pipeline can write a database row and then publish:

```ciac
pipeline Upload: StoreVideo -> publish Uploaded -> Return;
```

The Python lowerer commits mutating database verbs before the later
publish. Rust autocommits the statement before the queue call. A process
crash or broker outage between them leaves committed data with no event.
Publishing first would create the opposite bug: an event for data that
later rolls back.

### Repeated work after retry

Generated workers retry an entire pipeline. If an early write committed
and a later effect failed, the retry repeats the write. A client can also
repeat an API request after the server committed but the response was
lost. Neither API nor worker has durable deduplication.

### Route authorization without row authorization

Scopes answer “may this token call this operation?” They do not answer
“may this token read or mutate this row?” Generated CRUD and typed
`db.*` queries do not automatically constrain by the verified subject.
Every application must remember the ownership predicate in every path.

### Graph correctness without design feedback

The compiler catches cycles, unreachable components, auth placement, and
invalid composition. It does not warn about three cheap, high-value graph
facts:

- a query predicate has no supporting declared index;
- a retried worker makes a synchronous cross-service call;
- one message fans out through an unbudgeted number of worker paths.

**v0.19 theme: make the common partial-failure bugs inexpressible. A
committed transaction cannot lose its event, a repeated key cannot repeat
its committed effect, an authenticated subject cannot cross an ownership
boundary through generated queries, and the graph produces design-review
warnings before runtime.**

## The exact guarantees

Precise wording is part of the feature.

- A `publish` inside a v0.16 `transaction { ... }` is atomic with the
  database transaction because it lowers to an outbox row in the same
  database.
- Outbox delivery is **at least once**, not exactly once. A relay may
  publish the same stable message ID more than once.
- Idempotency provides effectively-once **committed generated effects**
  inside one database transaction: database mutations, outbox inserts,
  and the idempotency success record.
- Email, object storage, search, external HTTP, direct broker calls,
  cross-service calls, and arbitrary seeded code do not become
  transactional. Strict idempotency rejects them rather than weakening
  the promise silently.
- Ownership is enforced in compiler-generated SQL and type-checked
  handler lowering. Deliberately bypassing generated ports or modifying
  owned files is outside the guarantee and remains visible as drift.
- A worker subject is inherited only from compiler-owned authenticated
  request/message metadata. A payload field that happens to contain an
  owner string is not trusted identity.
- Architecture lints are advisory warnings by default. Outbox,
  idempotency, and ownership violations are hard errors and cannot be
  suppressed as lints.

“Exactly once” without naming the effect boundary is not used anywhere in
documentation or diagnostics.

## Surface syntax

### Transactional publish

v0.16 rejects non-database effects inside transactions. v0.19
deliberately admits typed `publish` with new semantics:

```ciac
handler PersistOrder(input: Order) -> Order {
    transaction {
        let saved = db.insert(Orders, input);
        publish OrderPlaced(OrderEvent {
            event_id: Uuid.new(),
            order_id: saved.id,
            owner_id: saved.owner_id
        });
    }
    return saved;
}
```

Outside a transaction, `publish` remains a direct broker effect.
Inside, it is an outbox enqueue. No backend infers that distinction from
indentation; resolved HIR carries:

```text
PublishMode::Direct
PublishMode::Transactional { database_instance }
```

The v0.16 `NonTransactionalEffect` diagnostic keeps its published
meaning: an effect that cannot participate in the database transaction is
illegal inside the block. A typed `publish` no longer triggers it because
v0.19 resolves that statement to a database outbox insert. Cache, HTTP,
email, object store, search, direct/untyped publish, and every other
non-transactional effect continue to trigger the same code. The
v0.16→v0.19 documentation and negative fixtures make that narrower
triggering set explicit without repurposing the append-only code.

### Idempotency

APIs:

```ciac
api CreateOrder: Order {
    method: POST;
    path: "/orders";
    idempotency_key: "header:Idempotency-Key";
    idempotency_ttl: 86400;
}
```

Workers:

```ciac
worker Fulfill on OrderPlaced {
    concurrency: 4;
    max_retries: 8;
    idempotency_key: "message:id";
    idempotency_ttl: 604800;
}
```

Typed workers may instead use:

```ciac
idempotency_key: "field:event_id";
```

The key source grammar is closed:

- API: `header:<valid-header-name>`;
- typed worker: `message:id` or `field:<String|Uuid|Int field>`;
- untyped worker: `message:id`.

When several databases are available, an explicit
`idempotency_db: <instance>` attribute resolves storage. With one
unambiguous/default database it is optional.

### Ownership field

v0.16 field attributes make ownership a domain property on a record:

```ciac
record Order {
    id: Uuid;
    owner_id: String {
        owner: true;
    }
    status: enum { Pending, Accepted, Failed };
}

table Orders: Order;
crud OrdersView: Order {
    read_scope: "orders:read";
    write_scope: "orders:write";
}
```

Rules:

- exactly zero or one `owner: true` field per record;
- the owner field is required `String`;
- `owner: true` implies a deterministic non-unique index; an explicit
  `index: true` coalesces with it rather than emitting a second index;
- it stores the verified token `sub`;
- generated create inputs never accept it from the caller;
- generated update cannot change it;
- every table/typed CRUD projection of that record is ownership-aware;
- an ownership-aware API path requires `Auth`;
- untyped JSON CRUD cannot declare ownership.

This first policy is intentionally narrow. No custom claim, role, group,
organization membership, ACL expression, administrator bypass, or
ownership transfer enters v0.19.

### Optional ordering and lint budget attributes

An ordered stream may declare:

```ciac
stream OrderEvents: OrderEvent {
    ordering_key: order_id;
}
```

The key must be `String`, `Uuid`, or `Int`. Ordering is per stream/key,
not global.

To make fan-out review explicit:

```ciac
stream OrderEvents: OrderEvent {
    fanout_limit: 4;
}
```

The limit is a compile-time graph budget, not runtime throttling.

Lint policy is deliberately small:

```ciac
lint deny predicate_without_index;

lint allow cross_service_call_in_worker_retry_loop on Fulfill
    because "the downstream operation is independently idempotent";
```

Warnings default to `warn`. Targeted `allow` requires a non-empty reason;
stale suppressions warn.

## Pillar 1 — Transactional outbox

### Atomic meaning

The crash matrix is:

| Failure point | Business write | Outbox row | Event |
|---------------|----------------|------------|-------|
| before transaction | absent | absent | absent |
| after write, then rollback | absent | absent | absent |
| after outbox insert, before commit | absent | absent | absent |
| after commit, before relay | present | pending | eventually |
| broker unavailable | present | pending | after recovery |
| publish ack, relay crash before mark | present | pending | duplicate possible, same ID |
| relay marks published | present | published | at least one acknowledged copy |

The acceptance property is:

> No event is emitted for a rolled-back transaction, and every committed
> outbox row remains durable until an acknowledged publish or a visible
> terminal failure.

### Why this is native runtime machinery

The old forecast of `std/outbox.ciac` did not ship because a straight-line
handler language cannot continually drain rows. Adding general loops to
express an infrastructure protocol would weaken the closed language.

v0.19 emits a compiler-owned relay task in the workers process. The
source declares the transactional publish; generated runtime owns
leasing, broker acknowledgement, retry, and terminal state.

### Logical schema

Each database used by transactional publish receives a reserved
`_ciac_outbox` table:

```text
id                  UUID, primary key and stable message ID
service             qualified producer
queue_instance      resolved broker instance
subject             resolved stream subject/topic
message_type        record name/schema version
payload             canonical JSON
headers             canonical generated metadata
ordering_key        nullable canonical scalar
ordering_sequence   nullable per-key sequence
publish_ordinal     source order within transaction
state               pending | published | dead
attempts            durable relay count
available_at_ms     next eligible database time
lease_owner         nullable relay ID
lease_until_ms      nullable database time
created_at_ms        enqueue time
published_at_ms      nullable acknowledgement time
last_error           nullable bounded text
```

`_ciac_*` is reserved across user tables, resources, indexes, and
constraints.

Payload/headers are serialized before insert. Serialization or the
documented 1 MiB event limit aborts the transaction. Binary/streaming
outbox payloads are outside v0.19.

### Portable lease protocol

The semantic state machine is shared across Postgres, MySQL, and SQLite.
DDL/placeholders/database-time expressions differ by dialect.

The portable claim algorithm:

1. read a bounded ordered candidate set;
2. conditionally update one candidate whose state/time/lease is still
   eligible;
3. one updated row owns the lease;
4. publish outside the claim transaction;
5. mark published only when lease owner still matches.

Provider-specific `SKIP LOCKED` may optimize later, but cannot define
semantics SQLite lacks.

Times persist as integer epoch milliseconds, avoiding the current
Postgres-shaped `TIMESTAMPTZ DEFAULT now()` problem in Python migration
metadata.

SQLite uses WAL, bounded busy timeout, and one default relay claimer per
database. Semantic parity is required; Postgres throughput is not.

### Relay placement

One relay runs per `(service, database instance)` with outbox rows:

- Python: workers process;
- Rust: workers binary.

A service with no declared worker/job still receives a workers process
when it owns outbox work. The relay is a synthetic runtime task, not a
fictional source `Worker` node; reachability and graph semantics continue
to reflect the user's publish edge.

Leases permit multiple worker replicas without singleton assumptions.
A crash leaves reclaimable work.

### Durable broker acknowledgement

The relay marks success only after provider acknowledgement.

**Kafka**

- producer `acks=all`;
- stable `ciac-message-id` header;
- ordering key becomes record key;
- producer idempotence may reduce repeats but does not replace consumer
  idempotency.

**NATS**

- Core NATS cannot prove durable acknowledgement;
- outbox/idempotent-worker programs require JetStream;
- generated runtime configures/validates durable subjects;
- `Nats-Msg-Id` uses outbox ID;
- startup fails when JetStream is unavailable; no weaker fallback.

Programs not using reliability features may retain current lightweight
direct NATS mode. The shared model exposes `requires_durable_queue`.

### Message metadata and tracing

The domain payload remains unchanged. Compiler metadata travels in
headers:

```text
ciac-message-id
ciac-origin-service
ciac-message-type
ciac-schema-version
ciac-publish-ordinal
ciac-subject            when identity provenance exists
traceparent
tracestate
baggage
```

Trace/subject metadata is captured during transaction execution, not
invented by the later relay. The relay gets its own linked span but
preserves the original carrier for worker extraction.

No bearer token is stored in an outbox row.

### Ordering

No key means no ordering guarantee.

For `ordering_key`:

- enqueue allocates the next `(subject,key)` sequence inside the business
  transaction;
- rollback rolls sequence allocation back;
- relay publishes only the lowest unpublished sequence per key;
- Kafka receives the key; JetStream receives sequential release;
- other keys proceed independently;
- a dead event blocks later events for that key rather than violating
  declared order.

An ordered stream must have one publisher service/database ownership
domain, and every publisher must use transactional publish. Mixed direct
and outbox publication is rejected.

### Retry and retention

Defaults are documented and configurable only through generated runtime
settings, not new language syntax in v0.19:

- bounded batch and lease;
- exponential bounded retry with deterministic message-ID jitter;
- finite attempt budget;
- published-row retention;
- dead rows retained/visible.

There is no operator replay UI or dead-letter browser in this release.

## Pillar 2 — Durable API and worker idempotency

### Shared storage

Each selected database receives `_ciac_idempotency`:

```text
namespace           qualified API/worker plus subject where relevant
key_hash            SHA-256; raw key never stored
payload_hash        canonical request/message fingerprint
kind                api | worker
state               processing | succeeded | terminal_failed
lease_owner         nullable execution ID
lease_until_ms      database time
attempts            durable attempt count
next_attempt_at_ms  nullable retry time
response_status     API only
response_headers    safe canonical subset
response_body       bounded JSON
completed_at_ms
expires_at_ms
last_error
PRIMARY KEY(namespace, key_hash)
```

Namespaces prevent cross-route/worker collisions:

- authenticated API: service + API + verified subject;
- anonymous API: service + API + anonymous sentinel;
- worker: service + worker + stream.

Two workers intentionally process one event independently. One user
cannot replay another user's response by guessing a key.

### Fingerprint collision

The same key with a different canonical payload is never replayed:

- API returns 409 without executing;
- worker records terminal poison-message failure and acknowledges/commits
  so it does not loop forever.

Fingerprints include:

- API method, route identity, path params, canonical body;
- worker stream type and canonical payload.

Canonical JSON has stable object keys, UUIDs, enums, and nested records.

### API state machine

Auth, scope, request validation, and key extraction happen before a
stored response can be replayed. Old success cannot bypass current auth.

For a new key:

1. claim/insert `processing` with a lease;
2. run the semantically validated effect closure;
3. serialize exact response before commit;
4. in the same transaction as durable effects, store `succeeded` plus
   response/expiry;
5. commit;
6. send response.

Crash after step 5 and before step 6 is the defining test: retry returns
the stored status/body with `Idempotency-Replayed: true` and no second
effect.

Concurrent duplicates either observe completion or receive bounded 409
plus `Retry-After`; they do not execute in parallel.

The same execution renews its lease for long work through a separate
short transaction. Completion is conditional on still owning the lease.

Authentication/scope errors are never stored. Accepted deterministic
4xx outcomes may be stored; unhandled 5xx remains retryable.

Responses are JSON and bounded to 1 MiB. Streaming/binary replay is not
supported.

### Strict effect closure

An idempotent API is accepted only when sema proves that all durable
effects are:

- generated mutations on the selected database;
- transactional outbox inserts on that database;
- idempotency completion.

They must share one v0.16 transaction.

Rejected effects:

- direct publish;
- service call;
- external HTTP;
- email/object/search;
- authoritative cache mutation;
- classic/extern handler with raw database/provider access;
- a second database transaction.

The diagnostic labels the exact effect and suggests moving publish into
the transaction/outbox or removing idempotency. There is no best-effort
mode under the same attribute.

### Worker state machine

For each delivery:

1. extract key and payload fingerprint;
2. claim/increment durable processing state;
3. if succeeded/terminal, acknowledge without effects;
4. otherwise execute one supported database transaction;
5. mark succeeded in that transaction;
6. commit;
7. acknowledge JetStream or commit Kafka offset.

Crash behavior:

- before effect commit: effects and marker roll back, delivery retries;
- after commit before broker ack: duplicate sees succeeded and skips;
- after ack: done.

`max_retries` becomes a durable count. Transient failures schedule
redelivery; permanent missing-key/subject/collision failures become
terminal. Unknown failures remain transient until budget exhaustion.

Kafka disables auto-commit and pauses the affected partition during
backoff. JetStream uses explicit ack/nack. Preserving partition/key order
may create head-of-line blocking; that is an explicit trade-off.

No automatic dead-letter topic enters v0.19.

### API versus worker

| Concern | API | Worker |
|---------|-----|--------|
| key | required header | message ID or typed field |
| duplicate in progress | bounded wait then 409 | broker redelivery after lease |
| completed duplicate | replay response | ack without effects |
| payload collision | 409 | terminal |
| success marker | atomic with effects/outbox | atomic with effects/outbox |
| retry budget | client within TTL | durable `max_retries` |
| missing key | 400 | permanent failure |

TTL starts at terminal completion. After expiry, the key may execute
again. Worker TTL must exceed broker retention/redelivery expectations;
the compiler documents but cannot inspect production broker policy.

## Pillar 3 — Ownership policy on every generated query

### Verified execution identity

For an API:

1. existing JWT/OAuth2 validation runs;
2. scope validation runs;
3. a non-empty string `sub` is required;
4. generated immutable execution context carries subject;
5. handlers and storage ports receive that context.

Missing/non-string subject returns 403.

For a worker:

1. consumer extracts compiler-owned `ciac-subject`;
2. sema proves every in-graph publisher of that stream has a verified or
   inherited subject;
3. worker receives immutable context;
4. missing identity fails before SQL.

A job has no end-user subject and cannot access ownership-aware storage.
An externally produced stream without trusted subject provenance also
cannot.

A generated synchronous service call forwards the current bearer token;
the target validates it again. Bearer tokens never cross broker/outbox.

### Identity-provenance pass

A hard semantic pass classifies each execution root/path:

```text
VerifiedRequestSubject
InheritedMessageSubject
NoSubject
```

It walks APIs, workers, jobs, pipeline branches, handler HIR, direct and
transactional publishes, and service calls.

If any publisher of a stream lacks valid provenance, a consuming worker
cannot use ownership-aware storage. The analysis never chooses the safe
publisher optimistically per message.

### Structured policy-aware queries

Ownership is inserted into a target-neutral query plan before Python or
Rust renders SQL:

| Operation | Effective behavior |
|-----------|--------------------|
| insert | owner bound from context, caller value ignored |
| get | `WHERE id = ? AND owner = ?` |
| update | ID + owner predicate; owner column immutable |
| delete | ID + owner predicate |
| query/count | existing typed predicate AND owner |
| delete_where | existing typed predicate AND owner |
| CRUD list | owner filter before order/limit/offset |
| CRUD create | owner omitted from input and injected |
| CRUD item write | ID + owner in one statement |

All values remain bound parameters.

`db.get` retains optional absence for hidden rows. Generated CRUD item
routes map both nonexistent and foreign rows to the same 403 response,
avoiding a second unrestricted existence query. Lists/counts simply omit
foreign rows.

### Cache isolation

Current CRUD keys resemble `<resource>:<id>`. Owned resources use:

```text
<resource>:<subject-hash>:<id>
```

Only owner-constrained database reads populate cache. Update/delete
invalidate the same subject namespace. Enabling ownership changes the
prefix version so old unscoped entries cannot leak.

### Bypass prevention

The guarantee requires exhaustive closure:

- every typed DB verb has an ownership arm;
- CRUD uses subject-aware stores;
- Python cannot use `session.get` for owned reads;
- owner is absent from caller create/update input;
- no generated update changes owner;
- classic/extern handlers cannot receive raw database access to an
  ownership-aware table;
- workers never trust payload identity;
- future DB verbs default to unsupported until policy lowering exists;
- external backends must advertise the ownership protocol feature.

The compiler does not sandbox hostile hand-edits to generated code;
regeneration drift exposes them.

### Ownership migration

New storage creates owner column/index normally. Enabling ownership on
existing data is staged:

1. add/backfill owner field with reviewed migration;
2. ensure every row is non-empty;
3. enable `owner: true`.

The compiler refuses to invent historical owners or silently make old
rows invisible, preserving v0.16's refusal to guess backfills. This is a
reviewed manual migration lane: v0.18 semantic diff classifies enabling
ownership as breaking, and the team advances its checked-in baseline only
after the backfill/policy change is reviewed. Native Postgres RLS is not
used; portable generated predicates are the v0.19 guarantee.

## Pillar 4 — Advisory architecture lints

The new lint pass runs after hard validation and uses the same resolved
graph/HIR/codegen metadata.

### `predicate_without_index`

Scans `db.query`, `db.count`, and `db.delete_where`.

An index supports the predicate when:

- primary `id` lookup is used; or
- a declared field index covers the leading equality/range field; or
- an ownership query uses the automatic owner index.

With v0.16's single-field indexes, a multi-field predicate warns when no
useful leading field is indexed. Composite-index reasoning remains out of
scope with composite indexes themselves.

`contains`, `!=`, and several ranges are not claimed to be optimized by a
portable B-tree.

The warning can offer an `index: true` field edit when one unambiguous
field is responsible. Applying the fix must clear the diagnostic.

This is a heuristic, not a statistics/query-plan engine.

### `cross_service_call_in_worker_retry_loop`

Fires when:

- pipeline owner is a worker;
- `max_retries > 0`;
- any path, including match arms, contains `call Service.Api`.

The downstream call can succeed, a later step fail, and retry invoke it
again. Labels point at both call and retry attribute.

The target API's idempotency declaration does not automatically suppress
the warning because key propagation is not part of the strict v0.19
surface. It may justify an explicit suppression reason.

### `unbounded_fan_out`

Computes logical worker invocations caused by one publish:

- distinct queue groups add;
- sequential publishes add;
- mutually exclusive match arms take maximum;
- linear chains preserve multiplier;
- worker concurrency does not multiply one message;
- realtime connection count is unknown and excluded.

The graph is acyclic, so the value is finite. “Unbounded” means
unbudgeted by `fanout_limit`, not mathematically infinite. The warning
shows contributing paths.

### Severity/suppression

- default warning;
- project `deny` promotes to error;
- targeted `allow ... because` suppresses one declaration;
- stale allow emits unused-suppression warning;
- unknown lint/target is an error;
- hard correctness diagnostics cannot be allowed;
- effective severity appears in human/JSON/LSP/MCP/describe output.

The diagnostic registry stores one code/default severity per lint. Policy
changes effective severity rather than minting duplicate codes.

Lints are cheap experiments. Each lint gets a checked-in labeled corpus
of expected-positive and expected-negative architectures plus the full
example suite. It graduates only with every expected positive explained,
zero false positives in the negative corpus, and no unexplained warning
in an existing example. A lint that misses that bar is omitted from the
release; richer auto-fixes/path rendering are cut first. None delays
outbox/idempotency/ownership.

## v0.18 semantic-diff extensions

Every new v0.19 meaning joins the canonical semantic model and
classification matrix:

| Change | Classification |
|--------|----------------|
| add API idempotency header requirement | breaking: old clients omit a now-required header |
| remove API idempotency guarantee | breaking guarantee removal, even though request acceptance broadens |
| shorten API idempotency TTL | breaking guarantee-window reduction |
| lengthen API idempotency TTL | additive guarantee strengthening, with storage-retention note |
| add worker `field:` key | internal when the field already exists and all producers are modeled; breaking when an external producer contract is affected |
| require `message:id` from an external producer | breaking |
| remove worker idempotency | breaking duplicate-effect guarantee removal |
| shorten worker idempotency TTL below the baseline | breaking guarantee-window reduction and broker-retention warning |
| lengthen worker idempotency TTL | additive guarantee strengthening |
| add `owner: true` | breaking: auth/subject, visibility, input, and mutation behavior change |
| remove ownership | additive acceptance with a high-severity security-relaxation note |
| direct → transactional publish | breaking delivery contract: consumers must tolerate at-least-once duplicates carrying stable IDs; list every stream/worker consumer |
| transactional → direct publish | breaking guarantee removal |
| first transactional publish on NATS | breaking operational prerequisite: JetStream replaces Core NATS for that program |
| emit a relay-only workers process | internal topology change with explicit runtime/compose note |
| add/change ordering key | breaking delivery-order/partition contract |
| change `fanout_limit` or lint policy | internal; a new `deny` may still fail CI and is called out |
| add/evolve `_ciac_*` internal tables | internal architecture change with migration note, never a public domain table |

The v0.18 baseline schema is versioned to represent these values. A
v0.19 compiler never drops them when comparing an older baseline.

## Pillar 5 — Simulation and real-provider proofs

### Simulator state/faults

v0.19 extends the v0.17 simulator with:

- outbox rows/leases/order counters;
- broker acknowledgements;
- idempotency rows/execution leases;
- worker offset/ack state;
- ownership subjects and identity-provenance metadata;
- virtual database time.

Named fault points cover:

```text
after business write
after outbox insert
before/after transaction commit
after relay claim
before/after broker acknowledgement
before outbox published mark
after API idempotency claim/effect/success record
before HTTP response
after worker claim/effect/success
before worker acknowledgement
during lease renewal
```

Bounded exploration asserts:

1. rollback leaves no business or outbox row;
2. no event exists without committed outbox;
3. pending rows eventually publish or become visible dead under fair
   recovery;
4. duplicate events preserve ID/payload/headers/order;
5. per-key order never inverts;
6. one API key/fingerprint commits effects at most once during TTL;
7. crash after API commit replays response;
8. one worker key commits effects at most once despite duplicate/crash;
9. key collision never executes the second payload;
10. owned operations never observe/mutate another subject's row;
11. missing/payload-forged subject fails closed;
12. lint output is deterministic.

Bounds/fairness/transition counts are printed. This is not an unbounded
formal proof.

### Generated system tests

Add:

- outbox commit/rollback/broker outage/relay crash;
- API concurrent duplicate/replay/collision;
- worker duplicate/crash-before-ack/retry budget;
- two-user ownership across CRUD and every DB verb;
- ordering with two keys and one key sequence.

JWT tests remain infrastructure-free. OAuth2 uses existing Keycloak
system paths. Real provider matrix covers both targets, three database
engines, Kafka, and NATS JetStream.

Fault hooks exist only in generated system-test profile, never normal
runtime configuration.

## Verification strategy

Release acceptance combines:

- compiler negative/type/effect/identity/lint fixtures;
- deterministic v0.17 crash/interleaving exploration;
- migration snapshots for every internal schema on all three engines;
- Python/Rust generated-project verification;
- cross-target message-envelope conformance;
- focused live Postgres/MySQL/SQLite, Kafka, and JetStream tests;
- two-user ownership checks across CRUD and every typed database verb;
- v0.18 semantic-baseline/classification fixtures for every new
  attribute and guarantee.

Goldens prove emitted shape; simulator and live tests prove the failure
properties. No feature is called supported from template text alone.

## Implementation map

### Syntax/IR/sema

- Field attr `owner`; stream attrs `ordering_key`/`fanout_limit`;
  API/worker idempotency attrs; lint declarations.
- Preserve direct versus transactional publish in HIR.
- Add reusable HIR effect visitors.
- Add outbox/idempotency/ownership/index contexts to normalized IR.
- Hard passes: identity provenance, idempotent effect closure, ordered
  stream topology.
- Advisory architecture-lint pass after hard validation.
- Reserve `_ciac_*`.

### Diagnostics/tooling

- Append hard errors and three warning codes after prior versions'
  registry tail.
- Update errors, vocab, describe, LSP hover/completion/code actions.
- JSON reports effective lint severity.
- `ciac explain` states at-least-once/effect boundaries.

### Shared codegen

- Model contexts for outbox relay, durable envelope, idempotency,
  execution identity, ownership, ordering, lints.
- Dialect-aware migrations for internal tables/indexes.
- Internal schema version in manifest; immutable additive internal
  migrations.
- OpenAPI: required idempotency header and 400/409/403 responses.
- TypeScript client: require idempotency key argument.
- System-test generator: fault and ownership checks.
- Compose: JetStream only when required; workers process for relay-only
  services; shared SQLite volume where API/worker need one file.
- External protocol feature declaration and hard refusal when required
  semantics are unsupported.

No k8s/Terraform/provider-provisioning work belongs here.

### Python backend

- Transactional publish lowers to outbox insert.
- Runtime modules for relay, idempotency, execution context.
- Queue supports explicit headers, JetStream ack, Kafka `acks=all`.
- Worker acknowledges/commits only after terminal durable state.
- Every owned SQL path binds subject; cache keys are scoped.
- Route/CRUD auth builds subject context and maps 400/403/409.
- Workers main starts relays and cleanup.
- Migration/runtime startup initializes internal schemas safely.

### Rust backend

Equivalent changes in lowerer, route, worker, queue, database, resource
store, auth, workers binary, state, errors, config, and dependencies.
Rust exhaustive matches over DB verbs become an extra guard: a new verb
cannot compile until ownership semantics are decided.

### Documentation/examples

- `docs/correctness.md` with crash matrices and non-guarantees.
- Update language, expressions, IR, architecture, backends, external
  protocol, blueprints (native outbox supersedes forecast loop), and
  errors.
- Flagship combines authenticated owned table, transactional publish,
  ordered stream, `message:id` worker, idempotent API, indexed query,
  and one justified lint suppression.

## Milestones

1. **M1 — Semantic contract:** syntax, IR effect/identity types,
   append-only diagnostics, executable crash/idempotency/ownership
   vectors.
2. **M2 — Internal schema/dialects:** outbox, idempotency, order,
   ownership indexes, portable epoch time, additive migration/state
   snapshots for all databases.
3. **M3 — Python outbox/relay:** transaction enqueue, leases, stored
   trace/subject, Kafka ack, JetStream, simulator crash matrix.
4. **M4 — Rust outbox/relay and envelope parity:** same state machine,
   cross-target messages, live engines/brokers.
5. **M5 — API idempotency:** key/fingerprint/lease/atomic response/replay,
   TTL, OpenAPI/client, both targets.
6. **M6 — Worker idempotency:** durable attempts, JetStream manual ack,
   Kafka explicit commit, crash-before-ack proof.
7. **M7 — Ownership:** subject provenance, every DB/CRUD query, scoped
   cache, staged migration, two-user tests, bypass rejection.
8. **M8 — Architecture lints:** three analyses, severity/suppression,
   safe index fix, machine/editor surfaces.
9. **M9 — Integrated matrix:** simulator exploration, generated system
   tests, provider/target parity, protocol/goldens/determinism.
10. **M10 — Docs, reconciliation, v0.19.0:** full workspace/generated/
    system verification and whole-version analysis.

## Explicit cuts

The only generated compose-topology changes allowed are those required by
the correctness contract: JetStream enablement, a relay-only workers
process, and a shared SQLite volume when API and relay must see one
database file. They do not generalize into a new deployment target.

- No XA/two-phase commit or database+broker distributed transaction.
- No global exactly-once claim.
- No global order across keys/services/databases.
- No saga/general durable workflow.
- No idempotent wrapper around arbitrary external effects.
- No key propagation between services.
- No bearer token in outbox/broker.
- No custom ownership claim, roles, ACLs, admin bypass, or transfer.
- No native provider-specific RLS as the portable guarantee.
- No untyped JSON ownership.
- No automatic/recommended indexes beyond declared field indexes.
- No query statistics/plan analysis.
- No dead-letter/replay control plane.
- No general loops in `.ciac`.
- No deployment maturity or new backend.

## Risks

- **Exactly-once wording can outrun reality.** Mitigation: at-least-once
  delivery plus exactly one committed supported effect per durable key,
  always stated with boundary.
- **Core NATS is too weak.** Mitigation: require JetStream and fail
  loudly.
- **SQLite contention can be high.** Mitigation: one claimer, WAL,
  bounded batches; semantic rather than throughput parity.
- **Lease protocols depend on time.** Mitigation: database time, integer
  epochs, renewal, conditional completion, simulator skew/failure tests.
- **Relay can publish twice.** Mitigation: stable message ID plus consumer
  idempotency; never claim duplicates impossible.
- **Ordering can block.** Mitigation: isolate by key, expose dead state,
  preserve guarantee rather than skipping failed sequence.
- **Stored responses can be sensitive.** Mitigation: subject namespace,
  hashed key, bounded TTL/body, auth revalidation, no tokens.
- **TTL bounds the guarantee.** Mitigation: make it explicit and document
  broker-retention relationship.
- **One query path may omit ownership.** Mitigation: policy-aware shared
  query plan, exhaustive verb/CRUD tests, default-deny new verbs.
- **Broker subject metadata can be forged by trusted producers.**
  Mitigation: state broker namespace as trust boundary; never trust
  payload; signed delegation is later work.
- **Existing rows lack owner.** Mitigation: staged activation and no
  guessed backfill.
- **Idempotency feels restrictive.** Mitigation: diagnostics name the
  exact effect that prevents the guarantee.
- **Lints can be noisy.** Mitigation: advisory default, deterministic
  path explanation, reasoned suppression; lints do not gate structural
  features.
- **Live matrix is expensive.** Mitigation: exhaustive target-neutral
  simulator plus focused parallel real-provider jobs; golden text alone
  never proves support.

## Confidence and v0.20 handoff

Outbox and idempotency are structural: the dual-write and retry bugs are
present in generated control flow today. Ownership is the domain/security
wall between route-level demos and multi-tenant systems. The three lints
are intentionally cheap experiments and may remain advisory or be cut
back if real examples produce poor signal.

v0.20 uses the effect taxonomy, stable message identities, simulator
failure sites, and tracing already present here to close the final agent
loop: a runtime failure must point back to the exact `.ciac` construct
that generated it.
