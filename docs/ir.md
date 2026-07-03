# The Intermediate Representation

A CIaC program compiles to a `SystemGraph` (crate `ciac-ir`): a typed
directed graph plus a record (type) table, resolved pipelines, and the
records of expanded higher-level constructs. Inspect it with
`ciac graph <file> --format json|dot`.

## Nodes

`Component` payloads, one per architectural component:

| Kind | Payload | Created by |
|------|---------|------------|
| `Api` | name, optional request record | `api X[: R];`, `crud X[: R];` |
| `Service` | name | handler steps (implicit), `crud` (store) |
| `Worker` | name | `worker X [on S];`, `events X;` (consumer) |
| `Stream` | name, subject, optional record | `stream X: R;`, `events X;`, the default stream |
| `Database` | engine (`Postgres`) | `use { db .. }` |
| `Cache` | engine (`Redis`) | `use { cache .. }` |
| `Queue` | engine (`Nats`/`Kafka`) | `use { queue .. }` |
| `Auth` | scheme (`Jwt`) | `use { auth .. }` |
| `Logging` / `Metrics` | provider | `use { .. }` |

Infrastructure kinds (database, cache, queue, auth, logging, metrics)
are singletons in v0.2; streams are not — declare as many as the
topology needs. The `Queue` node is the broker; each stream has a
`DependsOn` edge to it.

## Records

`record` declarations resolve into a side table (`SystemGraph::records`)
of `Record { name, fields }` values with a closed `FieldType` set
(`Str`, `Int`, `Float`, `Bool`, `Uuid`, `Timestamp`, `Json`,
`Enum { variants }`). Types are not nodes: they carry no edges and are
referenced by `RecordId` from apis, streams, resources, and pipelines.

## Edges

| `EdgeKind` | Meaning |
|------------|---------|
| `RequestFlow` | synchronous invocation (api → auth → handler → …) |
| `DataFlow` | reads/writes of stored data (handler ↔ database/cache) |
| `AsyncMessage` | publish (node → stream) and consume (stream → worker) |
| `DependsOn` | provisioning dependency (stream → broker) |

Because streams are nodes, message topology is explicit:
`publisher →(AsyncMessage) stream →(AsyncMessage) worker`. Cycle
detection therefore works *per stream* — a worker republishing to the
stream it consumes is a cycle, while publishing to a different stream
(stage chaining) is not.

## Pipelines

Each `Pipeline` records its owner (api or worker node), its **payload
type** (`Option<RecordId>`: the api's request record or the consumed
stream's record), and resolved `Step`s: `Auth { node }`,
`Publish { stream }`, `Return`, or `Handler { node }`. The surface
`Queue` step lowers to `Publish` on the default stream. Source spans are
kept alongside (not serialized) for diagnostics.

## Expansion records

- `Resource { name, api, service, record }` — one per `crud X[: R];`,
  so backends generate full CRUD surfaces (typed columns when `record`
  is present).
- `EventStream { name, stream, worker }` — one per `events X;`.

## Invariants of `NormalizedIr`

`NormalizedIr` wraps a graph that passed every validation pass. Backends
may assume:

1. All pipeline steps are resolved; every referenced node and record
   exists.
2. Flow edges (`RequestFlow` + `AsyncMessage` + `DependsOn`) are acyclic.
3. `Auth` steps and streams are backed by declared capabilities.
4. `Auth` only appears first in api pipelines; `Return` only appears
   terminally in api pipelines; each stream is published to at most once
   per pipeline.
5. Every `Publish` step's payload type equals the stream's record type
   (untyped streams accept any payload).
6. Node/edge/record iteration order is deterministic (declaration
   order).

Construct it only through `ciac_sema::analyze`;
`NormalizedIr::from_validated` exists for that caller alone.

## Serialization

The whole graph (including the record table) serializes to JSON — the
stable interface for tooling and the future wire format for
out-of-process backends. `to_dot()` renders Graphviz: streams appear as
parallelograms between their publishers and consumers.
