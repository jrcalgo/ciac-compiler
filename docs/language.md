# The CIaC Language (v0.2)

A CIaC program describes one deployable service as a set of declarations.
Declaration order is free; the compiler resolves references after parsing
the whole file.

## Grammar

```ebnf
program        = { item } ;
item           = service-decl | use-block | record-decl | stream-decl
               | api-decl | worker-decl | crud-decl | events-decl
               | pipeline-decl ;

service-decl   = "service" IDENT ";" ;
use-block      = "use" "{" { use-entry } "}" ;
use-entry      = IDENT IDENT ";" ;            (* capability provider *)
record-decl    = "record" IDENT "{" { field } "}" ;
field          = IDENT ":" type ";" ;
type           = "String" | "Int" | "Float" | "Bool" | "Uuid"
               | "Timestamp" | "Json"
               | "enum" "{" IDENT { "," IDENT } "}" ;
stream-decl    = "stream" IDENT ":" IDENT ";" ;
api-decl       = "api" IDENT [ ":" IDENT ] ";" ;
worker-decl    = "worker" IDENT [ "on" IDENT ] ";" ;
crud-decl      = "crud" IDENT [ ":" IDENT ] ";" ;
events-decl    = "events" IDENT ";" ;
pipeline-decl  = "pipeline" IDENT ":" step { "->" step } ";" ;
step           = IDENT | "publish" IDENT ;

IDENT          = letter-or-underscore { letter-digit-underscore } ;
```

Comments: `// line` and `/* block */`.

## Declarations

### `service <Name>;`

Names the system. Exactly one per program (`CIAC0010` if missing,
`CIAC0003` if repeated). The name drives generated package/module names.

### `use { capability Provider; .. }`

Declares the infrastructure capabilities the service is built on. Each
capability may appear once (`CIAC0012`). Supported pairs (`CIAC0013`
otherwise):

| Capability | Providers |
|------------|-----------|
| `auth` | `JWT` |
| `db` | `Postgres` |
| `cache` | `Redis` |
| `queue` | `NATS`, `Kafka`* |
| `logging` | `Structured` |
| `metrics` | `Prometheus` |

\* `Kafka` is accepted by the language but not yet implemented by the
bundled backends (`CIAC0011` at build time).

### `record <Name> { field: Type; .. }`

A typed data schema. Field types are the primitives above or an inline
`enum { A, B }` (`CIAC0015` for anything else); duplicate records or
fields are `CIAC0003`. Records compile to pydantic models (Python) and
serde structs (Rust); enums become `Literal[..]` / Rust enums and are
stored as text.

### `stream <Name>: <Record>;`

A named message channel carrying `<Record>` payloads (`CIAC0015` if the
record is unknown). Requires the `queue` capability (`CIAC0005`).
Pipelines publish to a stream with `publish <Name>`; workers consume one
with `on <Name>`. Multiple workers may consume the same stream —
fan-out is first-class. A stream with no publisher or no consumer is
reported unreachable (`CIAC0007`, warning).

Subjects follow `<service>.<stream>` in snake_case, e.g. stream
`Uploaded` in service `Media` uses `media.uploaded`. Each worker
consumes in a queue group named after it, so replicas load-balance.

### `api <Name>[: <Record>];` and `worker <Name> [on <Stream>];`

Declare an HTTP API surface and an asynchronous consumer. A typed api
validates its request body against the record; an untyped one accepts
any JSON object. A worker with `on` consumes that stream (`CIAC0017` if
undeclared); without `on` it consumes the service's *default stream*
(see `Queue` below). Behavior is attached with a pipeline of the same
name; an api or worker without one is reported unreachable (`CIAC0007`).

### `pipeline <Name>: Step -> Step -> ..;`

Attaches an execution chain to the api or worker named `<Name>`
(`CIAC0004` if neither exists, `CIAC0003` for a second pipeline on the
same component).

**Payload typing.** Each pipeline carries one payload type end-to-end:
the api's request record, or the consumed stream's record for workers
(untyped JSON when neither is typed). Every handler in the pipeline
takes and returns that type.

Steps are:

- **`Auth`** — authenticate the request. Requires the `auth` capability
  (`CIAC0005`). Must be the first step of an api pipeline and may not
  appear in worker pipelines (`CIAC0008`).
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
- **Any other name** — a *handler*: an implicitly declared service
  (business-logic unit) invoked at that point. Handlers are created on
  first reference, shared by name across pipelines, and provisioned
  with the declared `db`/`cache` capabilities. Referencing a declared
  api, worker, or stream as a step is an error (`CIAC0009`).

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
