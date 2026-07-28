# The Intermediate Representation

*Reader: a contributor working on the compiler's own sema/codegen
passes.*

A CIaC program compiles to a `SystemGraph` (crate `ciac-ir`): a typed
directed graph plus a deployable-service table, a record (type) table,
resolved pipelines, and the records of expanded higher-level constructs. Inspect it with
`ciac graph <file> --format json|dot`.

## Services

v0.5 adds deployable service ownership. `SystemGraph::services` contains
`Service { id, name }` entries. Nodes and pipelines have
`Option<ServiceId>` ownership:

- `Some(id)` for APIs, workers, handlers, capabilities, CRUD expansions,
  and events owned by a service block or the legacy implicit service.
- `None` for project-global nodes such as shared streams.

`SystemGraph::multi_service` records the surface form: `true` when the
program used `service { .. }` blocks. A lone `service <Name>;` also
registers one `Service` (for ownership tracking) but stays a single
deployable. Backends key their emission model on this flag (v0.5.1):
single-service programs generate one project at the output root;
multi-service programs generate one complete project per service under
`<service-kebab>/` plus a root docker-compose (one app+workers pair per
service, one shared broker, per-service infrastructure) and README.

## Nodes

`Component` payloads, one per architectural component:

| Kind | Payload | Created by |
|------|---------|------------|
| `Api` | name, optional request record, `ApiConfig` | `api X[: R];`, `crud X[: R];` |
| `Service` | name | handler steps (implicit), `crud` (store) |
| `Worker` | name, `WorkerConfig` | `worker X [on S];`, `events X;` (consumer) |
| `Job` | name, `JobConfig` | `job X { schedule: ..; }` |
| `Channel` | name, `ChannelConfig` | `channel X on Stream;` |
| `Stream` | name, subject, optional record | `stream X: R;`, `events X;`, the default stream |
| `Database` | name, engine (`Postgres`) | `use { db [name] .. }` |
| `Cache` | name, engine (`Redis`) | `use { cache [name] .. }` |
| `Queue` | name, engine (`Nats`/`Kafka`) | `use { queue [name] .. }` |
| `Auth` | name, scheme (`Jwt`) | `use { auth [name] .. }` |
| `Logging` / `Metrics` | name, provider | `use { .. }` |
| `ObjectStore` | name, provider, bucket | `use { object_store name S3 { .. } }` |
| `Email` | name, provider | `use { email name SES; }` |
| `Search` | name, provider | `use { search name OpenSearch; }` |
| `ExternalHttp` | name, base URL | `use { external_http name { base_url: .. } }` |
| `Scheduler` / `Realtime` | name, provider | `use { scheduler ..; realtime ..; }` |

Infrastructure capabilities are named instances in v0.4. Legacy unnamed
`use` entries lower to an instance named `default`; multiple instances of
the same kind can coexist. The `Queue` node is the broker; each stream has
a `DependsOn` edge to the queue instance it uses.

## Records

`record` declarations resolve into a side table (`SystemGraph::records`)
of `Record { name, fields }` values with a closed `FieldType` set
(`Str`, `Int`, `Float`, `Bool`, `Uuid`, `Timestamp`, `Json`,
`Enum { variants }`, `Reference { target, table, cardinality,
on_delete, on_update, unique }` — v0.16). Types are not nodes: they
carry no edges and are referenced by `RecordId` from apis, streams,
resources, and pipelines.

`Reference` fields resolve in two passes (v0.16): every record first
registers with a `FieldType::Json` placeholder in the field's declared
position (so field order/index is stable regardless of forward
references), deferred into the builder's `pending_references`; once
every record, table, service, and capability instance in the whole
program exists, `Builder::resolve_references()` makes one final pass
patching each placeholder in place via `SystemGraph::record_mut` —
`references`/`cardinality`/`on_delete`/`on_update`/`unique` are only
known correct once every table exists to check them against.
`Cardinality` (`One`/`Many`) and `RefAction` (`Restrict`/`Cascade`) are
their own small enums, not string-typed. `Table` itself carries the
resolved `service: Option<ServiceId>`/`db_instance: Option<NodeId>` it
belongs to, resolved by the same capability-resolution machinery a
handler body's own `db.*` calls use. A colored-DFS cycle check
(`find_reference_cycles`) walks `Cardinality::One` edges only —
`Many`-relations use a link table and can't produce this kind of
insertion-order cycle.

## Configs

Component attributes lower into typed config structs with defaults:

- `ApiConfig { method, path, scope }`
- `WorkerConfig { concurrency, max_retries }`
- `JobConfig { schedule, catch_up }`
- `ChannelConfig { path }`
- `CrudConfig { cache_ttl, page_size }`

Stream `subject` remains a plain field on `Component::Stream` after
defaults/overrides are resolved.

Handler declarations lower to `DataFlow` edges from the handler service
node to the selected capability instances. Backends use those edges to
surface binding metadata in generated handler stubs.

## Edges

| `EdgeKind` | Meaning |
|------------|---------|
| `RequestFlow` | synchronous invocation (api → auth → handler → …) |
| `DataFlow` | reads/writes of stored data (handler ↔ database/cache) |
| `AsyncMessage` | publish (node → stream) and consume (stream → worker/channel) |
| `ServiceCall` | synchronous typed call from one service pipeline to another service API |
| `DependsOn` | provisioning dependency (stream → broker) |

Because streams are nodes, message topology is explicit:
`publisher →(AsyncMessage) stream →(AsyncMessage) worker/channel`. Cycle
detection therefore works *per stream* — a worker republishing to the
stream it consumes is a cycle, while publishing to a different stream
(stage chaining) is not.

## Pipelines

Each `Pipeline` records its owner (api, worker, or job node), its
**payload type** (`Option<RecordId>`: the api's request record, the
consumed stream's record, or `None` for jobs), and resolved recursive
`Step`s. A step has an embedded
`StepKind` plus an optional source span (not serialized):

- `Auth { node }`
- `Publish { stream }`
- `Return`
- `Handler { node }`
- `Call { target }`, where `target` is the API node in another service
- `Match { field, arms }`, where each `MatchArm` has an optional label
  (`None` = wildcard) and its own nested `Vec<Step>`.

The surface `Queue` step lowers to `Publish` on the default stream.

## Expansion records

- `Resource { name, api, service, record, config }` — one per
  `crud X[: R];`, so backends generate full CRUD surfaces (typed
  columns when `record` is present).
- `EventStream { name, stream, worker }` — one per `events X;`.

## Invariants of `NormalizedIr`

`NormalizedIr` wraps a graph that passed every validation pass. Backends
may assume:

1. All pipeline steps are resolved; every referenced node and record
   exists.
2. Flow edges (`RequestFlow` + `AsyncMessage` + `DependsOn`) are acyclic.
3. `Auth` steps, streams, jobs, and channels are backed by declared
   capabilities.
4. `Auth` only appears first in api pipelines; `Return` only appears
   terminally in api pipelines; jobs have neither `Auth` nor `Return`;
   each stream is published to at most once per top-level path or match
   arm.
5. Every `Publish` step's payload type equals the stream's record type
   (untyped streams accept any payload).
6. Every `Match` is terminal, non-nested, branches on an enum payload
   field, and covers every variant.
7. Every `Call` targets an existing service API and carries the target
   API's request record.
8. Node/edge/record/service iteration order is deterministic (declaration
   order).

Construct it only through `ciac_sema::analyze`;
`NormalizedIr::from_validated` exists for that caller alone.

## Serialization

The whole graph (including the record table) serializes to JSON — the
stable interface for tooling and the future wire format for
out-of-process backends. `to_dot()` renders Graphviz: streams appear as
parallelograms between their publishers and consumers.
