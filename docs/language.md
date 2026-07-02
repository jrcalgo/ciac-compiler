# The CIaC Language (v0.1)

A CIaC program describes one deployable service as a set of declarations.
Declaration order is free except that readability conventions apply; the
compiler resolves references after parsing the whole file.

## Grammar

```ebnf
program        = { item } ;
item           = service-decl | use-block | api-decl | worker-decl
               | crud-decl | events-decl | pipeline-decl ;

service-decl   = "service" IDENT ";" ;
use-block      = "use" "{" { use-entry } "}" ;
use-entry      = IDENT IDENT ";" ;            (* capability provider *)
api-decl       = "api" IDENT ";" ;
worker-decl    = "worker" IDENT ";" ;
crud-decl      = "crud" IDENT ";" ;
events-decl    = "events" IDENT ";" ;
pipeline-decl  = "pipeline" IDENT ":" IDENT { "->" IDENT } ";" ;

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

### `api <Name>;` and `worker <Name>;`

Declare an HTTP API surface and an asynchronous queue consumer. Behavior
is attached with a pipeline of the same name; an api or worker without a
pipeline is reported unreachable (`CIAC0007`, warning).

### `pipeline <Name>: Step -> Step -> ..;`

Attaches an execution chain to the api or worker named `<Name>`
(`CIAC0004` if neither exists, `CIAC0003` for a second pipeline on the
same component). Steps are:

- **`Auth`** — authenticate the request. Requires the `auth` capability
  (`CIAC0005`). Must be the first step of an api pipeline and may not
  appear in worker pipelines (`CIAC0008`).
- **`Queue`** — publish the current payload to the queue. Requires the
  `queue` capability (`CIAC0005`); at most one per pipeline (`CIAC0009`).
  Publishing is fire-and-forget: later steps still run synchronously.
- **`Return`** — respond to the caller. Only valid as the final step of
  an api pipeline (`CIAC0009`).
- **Any other name** — a *handler*: an implicitly declared service
  (business-logic unit) invoked at that point. Handlers are created on
  first reference, shared by name across pipelines, and provisioned with
  the declared `db`/`cache` capabilities. Referencing a declared api or
  worker as a step is an error (`CIAC0009`).

Worker pipelines consume from the queue (requiring the `queue`
capability) and run their steps per message. A worker that publishes
back to the queue it consumes from is a cycle (`CIAC0006`).

### `crud <Name>;`

Expands to `REST API → (Auth) → Service → Database (+ Cache)`: a full
create/read/update/delete HTTP resource at `/<name>s`, a store service
with read-through caching when `cache` is declared, and a generic
keyed-document model. Requires `db` (`CIAC0005`); uses `auth` when
declared.

### `events <Name>;`

Expands to `Queue → Worker (→ Database)`: a dedicated consumer for the
`<service>.<name>` subject. Requires `queue` (`CIAC0005`).

## Messaging conventions

- `Queue` steps publish to the shared subject `<service>.events`
  (snake_case).
- Declared workers consume `<service>.events` in a queue group named
  after the worker, so replicas load-balance.
- `events X;` consumers use the dedicated subject `<service>.<x>`.

## Determinism

Given identical source, `ciac build` produces byte-identical output —
generated projects are safe to diff and regenerate.
