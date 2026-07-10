# The CIaC Language (v0.11.0)

A CIaC program describes one deployable service — or, with `project` +
`service { .. }` blocks, a system of services — as a set of
declarations. Declaration order is free; the compiler resolves
references after parsing the whole file.

## Implementation status

CIaC's contract is that **whatever `ciac build` accepts, the generated
system actually does**. Constructs the language accepts but no backend
implements yet still pass `ciac check` (so programs can be designed
ahead) and fail `ciac build` with `CIAC0011`:

| Construct | `ciac check` | `ciac build` |
|-----------|--------------|--------------|
| Everything below, unless listed | accepted | generated, working code |
| `queue .. Kafka` | accepted | gated (`CIAC0011`) |

## Grammar

```ebnf
program        = { item } ;
item           = project-decl | service-decl | service-block
               | use-block | record-decl | table-decl | stream-decl
               | handler-decl | api-decl | worker-decl | job-decl
               | channel-decl | crud-decl | events-decl | pipeline-decl
               | import-decl | blueprint-decl | expand-stmt ;         (* v0.8 *)

project-decl   = "project" IDENT ";" ;
service-decl   = "service" IDENT ";" ;
service-block  = "service" IDENT "{" { service-item } "}" ;
service-item   = use-block | api-decl | worker-decl | job-decl
               | channel-decl | crud-decl | events-decl | handler-decl
               | pipeline-decl | expand-stmt ;                        (* v0.8 *)
import-decl    = "import" STRING ";" ;                                (* v0.8 *)
blueprint-decl = "blueprint" IDENT "<" IDENT ":" "record" ">"
                 "{" "params" "{" { field } "}"
                     { blueprint-item } "}" ;                         (* v0.8 *)
blueprint-item = use-block | crud-decl | stream-decl | handler-decl ; (* v0.8 *)
expand-stmt    = "expand" IDENT "<" IDENT ">" decl-tail ;              (* v0.8 *)
use-block      = "use" "{" { use-entry } "}" ;
use-entry      = IDENT IDENT ";"              (* capability provider *)
               | IDENT IDENT IDENT ";"        (* capability name provider *)
               | IDENT IDENT attr-block       (* providerless named capability *)
               | IDENT IDENT IDENT attr-block ;
record-decl    = ( "record" | "error" ) IDENT "{" { field } "}" ;
field          = IDENT ":" type ";" ;
type           = "String" | "Int" | "Float" | "Bool" | "Uuid"
               | "Timestamp" | "Json"
               | "enum" "{" IDENT { "," IDENT } "}" ;
table-decl     = "table" IDENT ":" IDENT ";" ;
stream-decl    = "stream" IDENT ":" IDENT decl-tail ;
handler-decl   = "handler" IDENT "{" { binding } "}"        (* classic *)
               | [ "extern" ] "handler" IDENT
                 "(" [ param { "," param } ] ")" "->" type
                 ( ";" | "{" { stmt } "}" ) ;                (* v0.7 *)
binding        = IDENT ":" IDENT ";" ;
param          = IDENT ":" type ;
api-decl       = "api" IDENT [ ":" IDENT ] decl-tail ;
worker-decl    = "worker" IDENT [ "on" IDENT ] decl-tail ;
job-decl       = "job" IDENT decl-tail ;
channel-decl   = "channel" IDENT "on" IDENT decl-tail ;
crud-decl      = "crud" IDENT [ ":" IDENT ] decl-tail ;
events-decl    = "events" IDENT ";" ;
pipeline-decl  = "pipeline" IDENT ":" step { "->" step } ";" ;
decl-tail      = ";" | attr-block ;
attr-block     = "{" { attr } "}" ;
attr           = IDENT ":" attr-value ";" ;
attr-value     = IDENT | NUMBER | STRING ;
step           = IDENT | "publish" IDENT | "call" qualified-ident
               | match-step ;
match-step     = "match" IDENT "{" { arm } "}" ;
arm            = ( IDENT | "_" ) "->" step { "->" step } ";" ;
qualified-ident = IDENT { "." IDENT } ;

IDENT          = letter-or-underscore { letter-digit-underscore } ;
NUMBER         = digit { digit } ;
STRING         = '"' { char } '"' ;
```

`stmt` (the `{ stmt }` inside a v0.7 inline handler body) is its own,
larger grammar — `let`/`return`/`fail`/`publish` statements and an
expression language with `if`/`match`, record construction, and a
closed set of capability verbs. See `docs/expressions.md`.

Comments: `// line` and `/* block */`.

## Declarations

### `service <Name>;`

Names the system. Exactly one per program (`CIAC0010` if missing,
`CIAC0003` if repeated). The name drives generated package/module names.

In v0.5, multi-service projects use `project <Name>;` plus service
blocks. The legacy `service <Name>;` form remains valid and lowers to a
single implicit service.

### `project <Name>;` and `service <Name> { ... }`

`project` names a multi-service CIaC project. Each `service` block owns
its APIs, workers, CRUD resources, handler bindings, capabilities, and
pipelines:

```ciac
project MediaSystem;

record Video { id: Uuid; }
stream Uploaded: Video;

service UploadApi {
    use { queue bus NATS; }
    api Upload: Video;
    pipeline Upload: publish Uploaded -> Return;
}
```

Records and streams are project-global. Service-local declarations must
live inside a service block once any service block is used (`CIAC0030`).
Service names are project-global (`CIAC0026`).

### `import "path";` (v0.8)

Splices another file's items in, in place, at the position of the
`import` — literal textual composition, not a symbol-table merge (the
same file reached through two different import paths loads exactly
once; a cycle is `CIAC0047`). By the time semantic analysis runs, a
multi-file program is indistinguishable from one big file. Paths are
relative to the importing file, except the reserved `std/` prefix
(`import "std/crud.ciac";`), which resolves against a small blueprint
library embedded in the compiler itself rather than the filesystem.
See `docs/blueprints.md`.

### `blueprint <Name><<R: record>> { .. }` and `expand <Name><<Concrete>> { .. };` (v0.8)

A blueprint is a checked template over one record type, expanded with
hygienic naming at every `expand` site — the DRY mechanism for
patterns (audited CRUD, a webhook receiver shape) that would otherwise
mean hand-copying the same few declarations per service. Full grammar,
the hygiene rule, generic constraint checking, and the embedded `std`
library are documented in `docs/blueprints.md`.

### `use { capability Provider; .. }`

Declares the infrastructure capabilities the service is built on. v0.3
programs can still use the legacy unnamed form:

```ciac
use { db Postgres; cache Redis; queue NATS; }
```

v0.4 adds named instances:

```ciac
use {
    db main Postgres;
    db analytics Postgres;
    cache hot Redis;
    object_store media S3 { bucket: "videos"; }
    external_http billing { base_url: "https://billing.internal"; }
}
```

Legacy entries lower to an implicit instance named `default`. Duplicate
instances of the same capability kind/name are `CIAC0012`. Supported
pairs (`CIAC0013` otherwise):

| Capability | Providers | Generated as (Python / Rust) |
|------------|-----------|------------------------------|
| `auth` | `JWT`, `OAuth2` | FastAPI dependency + PyJWT (OAuth2: JWKS) / axum extractor + jsonwebtoken (OAuth2: fetched JWKS) |
| `db` | `Postgres`, `MySQL`* | SQLAlchemy async engine per instance / SQLx pool per instance |
| `cache` | `Redis` | redis-py client per instance / redis crate client per instance |
| `queue` | `NATS`, `Kafka`* | nats-py or aiokafka / async-nats |
| `logging` | `Structured` | structlog / tracing |
| `metrics` | `Prometheus` | prometheus-client / metrics-exporter-prometheus |
| `object_store` | `S3` | aioboto3 wrapper / rust-s3 wrapper (+ MinIO in compose) |
| `email` | `SES`, `SMTP` | aiosmtplib sender / lettre sender (+ Mailpit in compose) |
| `search` | `OpenSearch` | opensearch-py client / opensearch client (+ single-node container) |
| `external_http` | providerless; requires `base_url` | httpx client per instance / reqwest client per instance |
| `scheduler` | `Cron` | in-process scheduled jobs |
| `realtime` | `WebSocket`, `SSE` | stream channels over WebSocket/SSE |

\* `MySQL` and `Kafka` are fully implemented by the Python backend
(v0.11); the Rust backend gates both (`CIAC0011` at build time) —
sqlx pools are typed per database and rdkafka carries a native build
chain, so each waits on real demand. `auth OAuth2` requires an
`issuer` attribute (and optional `audience`): bearer RS256 tokens are
validated against `{issuer}/.well-known/jwks.json` on both backends.

Both `SES` and `SMTP` email providers send over SMTP — for SES, point
the generated `SMTP_*` variables at your SES SMTP endpoint. Handlers
receive their bound instances as typed constructor parameters; the
generated docker-compose runs a local-dev container per instance
(distinct databases, Redis clients, MinIO, Mailpit, OpenSearch).

### `record <Name> { field: Type; .. }`

A typed data schema. Field types are the primitives above or an inline
`enum { A, B }` (`CIAC0015` for anything else); duplicate records or
fields are `CIAC0003`. Records compile to pydantic models (Python) and
serde structs (Rust); enums become `Literal[..]` / Rust enums and are
stored as text.

`error <Name> { field: Type; .. }` (v0.7) is the same field grammar
under a different keyword: it compiles to a raisable exception
(Python) / `thiserror`-derived error type (Rust) instead of a plain
data model, for use with `fail <Name>(..)` in a handler body — see
`docs/expressions.md`.

### Attributes

`api`, `worker`, `job`, `channel`, `stream`, and `crud` declarations may end in an
attribute block instead of `;`. Attributes are validated from a closed
registry; unknown names are `CIAC0018`, invalid values or unmet
preconditions are `CIAC0019`.

| Target | Attribute | Value | Default | Checks |
|--------|-----------|-------|---------|--------|
| `api` | `method` | `GET`/`POST`/`PUT`/`DELETE`/`PATCH` | `POST` | `GET`/`DELETE` cannot have a typed request body |
| `api` | `path` | string starting with `/` | `/<kebab-name>` | duplicate api paths are `CIAC0003` |
| `api` | `scope` | string | none | pipeline must start with `Auth` |
| `worker` | `concurrency` | integer >= 1 | `1` | |
| `worker` | `max_retries` | integer >= 0 | `0` | |
| `job` | `schedule` | five-field cron string | required | invalid cron is `CIAC0037` |
| `job` | `catch_up` | `true`/`false` | `false` | |
| `channel` | `path` | string starting with `/` | `/channels/<kebab-name>` | duplicate channel paths are `CIAC0003` |
| `stream` | `subject` | string | `<service>.<stream>` | duplicate subjects are `CIAC0003` |
| `crud` | `cache_ttl` | integer >= 1 | `300` | requires `cache` |
| `crud` | `page_size` | integer >= 1 | `100` | |

### `stream <Name>: <Record>;`

A named message channel carrying `<Record>` payloads (`CIAC0015` if the
record is unknown). Requires the `queue` capability (`CIAC0005`).
Pipelines publish to a stream with `publish <Name>`; workers consume one
with `on <Name>`. Multiple workers may consume the same stream —
fan-out is first-class. A stream with no publisher or no consumer is
reported unreachable (`CIAC0007`, warning).

Subjects follow `<service>.<stream>` in snake_case by default, e.g. stream
`Uploaded` in service `Media` uses `media.uploaded`. Each worker
consumes in a queue group named after it, so replicas load-balance.

### `api <Name>[: <Record>];`, `worker <Name> [on <Stream>];`, and `job <Name> { ... }`

Declare an HTTP API surface and an asynchronous consumer. A typed api
validates its request body against the record; an untyped one accepts
any JSON object. A worker with `on` consumes that stream (`CIAC0017` if
undeclared); without `on` it consumes the service's *default stream*
(see `Queue` below). Behavior is attached with a pipeline of the same
name; an api, worker, or job without one is reported unreachable (`CIAC0007`).

Jobs require `scheduler Cron` and a `schedule` cron expression:

```ciac
use { scheduler jobs Cron; }

job Cleanup {
    schedule: "0 3 * * *";
}

pipeline Cleanup: PruneExpired;
```

Job pipelines start with an untyped JSON payload. They may invoke
handlers, calls, and publishes, but cannot use `Auth` or `Return`.

### `channel <Name> on <Stream>;`

Exposes a stream through the declared realtime provider. Channels require
`realtime` and the referenced stream must exist (`CIAC0017`):

```ciac
use { queue NATS; realtime live WebSocket; }

stream Progress: Video;
channel LiveProgress on Progress {
    path: "/live/progress";
}
```

Channels count as stream consumers for reachability. Typed streams are
serialized through their generated schema; untyped streams flow as JSON.

### `handler <Name> { ... }`

Declares which named capability instances a pipeline handler uses:

```ciac
handler StoreVideo {
    db: main;
    cache: hot;
    object_store: media;
}
```

Handlers referenced in pipelines may still be implicit. If no handler
declaration exists, CIaC preserves v0.1-v0.3 behavior by binding to the
default `db`/`cache` instances when they exist. If multiple instances of a
kind exist and none is named `default`, implicit binding is ambiguous
(`CIAC0023`). Binding to a missing instance is `CIAC0022`; binding an
unsupported kind is `CIAC0024`.

### `handler <Name>(params) -> Type { .. }` and `extern handler <Name>(params) -> Type;`

v0.7 gives a handler an optional typed signature, replacing the
binding-only form's implicit capability wiring with real parameter and
return types checked against the pipeline's payload (`CIAC0039`-
`CIAC0046`). Two flavors share that signature:

- **`extern handler`** — a typed stub, seeded once like a classic
  handler: you implement `handle` yourself, and regeneration never
  overwrites it.
- **Inline body** (no `extern`) — the `{ .. }` block is CIaC source,
  lowered straight to Python/Rust on every build. The generated file is
  compiler-owned; there's no stub to fill in.

```ciac
extern handler Notify(v: Video) -> Video;

handler StoreVideo(v: Video) -> Video {
    let inserted = db.insert(Videos, v);
    return inserted;
}
```

The inline body language — statements, expressions, the closed
capability-verb set, and builtins — is documented in full in
`docs/expressions.md`.

### `table <Name>: <Record>;`

Declares a real database table backed by `<Record>` (requires `db`,
`CIAC0005`; unknown record is `CIAC0015`), for `db.insert`/`db.get`
calls in a handler body to target (`CIAC0042` for an unrecognized
table). Unlike `crud`, a `table` has no REST surface of its own — it's
plain storage a handler body reads and writes.

Schema changes are additive-only, incremental SQL migrations: a new
`table` or a new field on one produces a numbered migration file
(`CREATE TABLE` / `ALTER TABLE ... ADD COLUMN`); a field being removed
or retyped, or a table disappearing, is refused as `CIAC0046` rather
than guessed at — write that change by hand. Migration files are
`Seeded` (see `docs/regeneration.md`): once generated, later builds
leave them alone.

### `pipeline <Name>: Step -> Step -> ..;`

Attaches an execution chain to the api, worker, or job named `<Name>`
(`CIAC0004` if none exists, `CIAC0003` for a second pipeline on the same
component).

**Payload typing.** Each pipeline carries one payload type end-to-end:
the api's request record, the consumed stream's record for workers, or
untyped JSON for jobs. Every handler in the pipeline takes and returns
that type.

Steps are:

- **`Auth`** — authenticate the request. Requires the `auth` capability
  (`CIAC0005`). Must be the first step of an api pipeline and may not
  appear in worker or job pipelines (`CIAC0008`).
- **`publish <Stream>`** — publish the current payload to a named
  stream (`CIAC0017` if undeclared). The payload type must match the
  stream's record (`CIAC0016`). Publishing is fire-and-forget; at most
  one publish per stream per pipeline (`CIAC0009`). A worker
  republishing to the stream it consumes is a cycle (`CIAC0006`);
  publishing to a *different* stream is the normal way to chain stages.
- **`Queue`** — legacy sugar (v0.1): publishes to an auto-created
  untyped default stream (`<service>.events`) that unbound workers
  consume. Requires the `queue` capability (`CIAC0005`).
- **`Return`** — respond to the caller. Only valid as the final step of
  an api pipeline (`CIAC0009`).
- **`call <Service>.<Api>`** — synchronously invoke another service's
  typed API. The target service and API must exist (`CIAC0027`/
  `CIAC0028`), and the caller payload type must match the target API's
  request record (`CIAC0029`). Malformed call targets are `CIAC0032`.
  **HTTP contract:** the compiler generates a typed client per target
  service (`app/clients/<service>.py` / `src/clients/<service>.rs`)
  with one method per called api. It serializes the payload record,
  sends the target api's real method and path, fails the pipeline on
  any non-2xx response, and validates the response's `data` envelope
  back into the record. The base URL comes from the `<SERVICE>_URL`
  environment variable; the system docker-compose points it at the
  target's container (`http://billing:8000`), and the development
  default is the target's host port from the compose mapping.
- **`match <field> { ... }`** — statically branch on an enum field of
  the pipeline payload. A match must be the final top-level step and may
  not be nested in v0.3 (`CIAC0020`). Arm labels must be declared enum
  variants, with at most one trailing `_` wildcard. Every enum variant
  must be covered directly or by `_` (`CIAC0021`). Arm chains can contain
  handlers, publishes, and `Return`; type checks, cycle detection, auth
  placement, and duplicate-publish checks apply inside each arm.
- **Any other name** — a *handler*: an implicitly declared service
  (business-logic unit) invoked at that point. Handlers are created on
  first reference, shared by name across pipelines, and provisioned
  with the declared `db`/`cache` capabilities. Referencing a declared
  api, worker, job, channel, or stream as a step is an error
  (`CIAC0009`).

### `crud <Name>[: <Record>];`

Expands to `REST API → (Auth) → Service → Database (+ Cache)`: a full
create/read/update/delete HTTP resource at `/<name>s`. With a record,
the resource gets real typed columns (the `id` primary key is always
server-generated); without one it stays a generic keyed JSON document.
Requires `db` (`CIAC0005`); uses `auth` when declared.

### `events <Name>;`

Expands to `Stream → Worker (→ Database)`: an untyped stream on the
`<service>.<name>` subject with a dedicated consumer. Requires `queue`
(`CIAC0005`). Prefer explicit `stream`/`worker on` pairs when the
payload should be typed.

## Determinism

Given identical source, `ciac build` produces byte-identical output —
generated projects are safe to diff and regenerate.
