# The Intermediate Representation

A CIaC program compiles to a `SystemGraph` (crate `ciac-ir`): a typed
directed graph plus resolved pipelines and the records of expanded
higher-level constructs. Inspect it with `ciac graph <file> --format
json|dot`.

## Nodes

`Component` payloads, one per architectural component:

| Kind | Payload | Created by |
|------|---------|------------|
| `Api` | name | `api X;`, `crud X;` |
| `Service` | name | handler steps (implicit), `crud X;` (store) |
| `Worker` | name | `worker X;`, `events X;` (consumer) |
| `Database` | engine (`Postgres`) | `use { db .. }` |
| `Cache` | engine (`Redis`) | `use { cache .. }` |
| `Queue` | engine (`Nats`/`Kafka`) | `use { queue .. }` |
| `Auth` | scheme (`Jwt`) | `use { auth .. }` |
| `Logging` | provider (`Structured`) | `use { logging .. }` |
| `Metrics` | provider (`Prometheus`) | `use { metrics .. }` |

Infrastructure kinds (database, cache, queue, auth, logging, metrics)
are singletons in v0.1.

## Edges

| `EdgeKind` | Meaning |
|------------|---------|
| `RequestFlow` | synchronous invocation (api → auth → handler → …) |
| `DataFlow` | reads/writes of stored data (handler ↔ database/cache) |
| `AsyncMessage` | publish/consume through the queue |
| `DependsOn` | provisioning dependency without data movement |

## Pipelines

Each `Pipeline` records its owner (api or worker node) and resolved
`Step`s: `Auth { node }`, `Queue { node }`, `Return`, or
`Handler { node }`. Source spans are kept alongside (not serialized) for
diagnostics.

## Expansion records

- `Resource { name, api, service }` — one per `crud X;`, so backends can
  generate full CRUD surfaces rather than reverse-engineering the graph.
- `EventStream { name, worker, subject }` — one per `events X;`.

## Invariants of `NormalizedIr`

`NormalizedIr` wraps a graph that passed every validation pass. Backends
may assume:

1. All pipeline steps are resolved; every referenced node exists.
2. Flow edges (`RequestFlow` + `AsyncMessage` + `DependsOn`) are acyclic.
3. `Auth`/`Queue` steps are backed by declared capabilities.
4. `Auth` only appears first in api pipelines; `Return` only appears
   terminally in api pipelines; `Queue` appears at most once per pipeline.
5. Node/edge iteration order is deterministic (declaration order).

Construct it only through `ciac_sema::analyze`;
`NormalizedIr::from_validated` exists for that caller alone.

## Serialization

The whole graph serializes to JSON (spans omitted), which is the stable
interface for tooling and would be the wire format for out-of-process
backends. `to_dot()` renders Graphviz for architecture reviews.
