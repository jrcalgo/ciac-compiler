# CIaC — Code as Infrastructure Compiler

A compiler for a declarative backend-architecture language. You describe
*what system exists* — services, APIs, pipelines, storage, messaging —
and `ciac` validates the architecture at compile time and deterministically
generates a complete, runnable backend in your target language.

```text
service VideoPlatform;

use {
    auth JWT;
    db Postgres;
    cache Redis;
    queue NATS;
}

api Upload;
worker Transcoder;

pipeline Upload:
    Auth
    -> StoreVideo
    -> Queue
    -> Return;

pipeline Transcoder:
    Transcode
    -> SaveResult;
```

```sh
ciac build video-platform.ciac --target python --out ./video-platform
# → FastAPI app, JWT auth, SQLAlchemy, Redis, NATS workers,
#   docker-compose, smoke tests. Lint-clean, tests passing.

ciac build video-platform.ciac --target rust --out ./video-platform-rs
# → the same architecture as an Axum/SQLx/async-nats Cargo project.
```

Since v0.3, payloads and message topology are typed and named, and
components can be tuned with closed, validated attributes:

```text
record Video {
    id: Uuid;
    title: String;
    status: enum { Pending, Ready, Failed };
}

stream Uploaded: Video;              // a named, typed channel
stream Transcoded: Video;
stream DeadLetters: Video;

api Upload: Video {                  // request body validated as Video
    method: PUT;
    path: "/videos";
    scope: "videos:write";
}
worker Transcoder on Uploaded {      // consumes Uploaded
    concurrency: 4;
    max_retries: 2;
}
worker Notifier on Transcoded;       // fan-out via a second stream

pipeline Upload: Auth -> StoreVideo -> publish Uploaded -> Return;
pipeline Transcoder:
    Transcode
    -> match status {
        Ready -> Notify -> publish Transcoded;
        Failed -> publish DeadLetters;
    };
pipeline Notifier: Notify;

crud Clip: Video {                   // real typed columns, not JSON blobs
    cache_ttl: 60;
    page_size: 50;
}
```

The compiler checks every publish site against its stream's record
(`CIAC0016`), rejects workers that republish to the stream they consume
(`CIAC0006`), and generates pydantic/serde schemas so malformed payloads
are rejected at the boundary of the running system too. `match` is
checked for enum labels and exhaustiveness (`CIAC0020`/`CIAC0021`).

v0.4 expands the ontology and lets handlers bind to named capability
instances. Every binding is real at runtime: each instance gets its own
settings fields, container, and client, injected into the handler's
constructor (S3 via aioboto3/rust-s3 with MinIO for local dev, SMTP via
aiosmtplib/lettre with Mailpit, OpenSearch, per-instance httpx/reqwest
clients):

```text
use {
    db main Postgres;
    db analytics Postgres;
    object_store media S3 { bucket: "videos"; }
    search catalog OpenSearch;
    external_http billing { base_url: "https://billing.internal"; }
}

handler StoreVideo {
    db: main;
    object_store: media;
}

handler IndexVideo {
    search: catalog;
    external_http: billing;
}
```

v0.5 lets one file describe a multi-service topology with shared typed
streams and checked service-to-service calls:

```text
project MediaSystem;

record Video { id: Uuid; }
stream Uploaded: Video;

service Billing {
    api Charge: Video;
    pipeline Charge: CapturePayment -> Return;
}

service UploadApi {
    use { queue bus NATS; }
    api Upload: Video;
    pipeline Upload:
        call Billing.Charge
        -> publish Uploaded
        -> Return;
}
```

Building a multi-service program emits a **system of deployables** —
one complete project per service plus a root compose file that wires
them together (one shared broker for the shared streams, per-service
databases/caches/stores, container DNS for service calls):

```text
media-system/
├── docker-compose.yml   # whole system: billing + upload-api + queue + ...
├── README.md            # service/port table, run instructions
├── billing/             # complete standalone project
└── upload-api/          # BILLING_URL points at the billing container
```

`call Billing.Charge` compiles into a typed HTTP client
(`app/clients/billing.py` / `src/clients/billing.rs`): it sends the
target api's real method and path, fails the pipeline on non-2xx, and
validates the response envelope back into the `Video` record.

Every capability provider generates on both bundled targets as of
v0.13. A construct no target can implement yet still passes `ciac
check` and is refused by `ciac build` with `CIAC0011` rather than
silently miscompiling — if it builds, the generated system actually
does it. The per-provider support table lives in
[docs/language.md](docs/language.md).

v0.15 turns a generated system into one a team can operate and point
other software at: every `ciac build` emits an `openapi.json`, and
`--client ts` adds a dependency-free typed TypeScript `fetch` client
alongside it; `use { tracing OpenTelemetry; }` gets one trace id
spanning a `call`/`publish`→worker chain, with an otel-collector and
Jaeger wired into dev compose; `--deploy ci` emits a GitHub Actions
workflow that mirrors `ciac verify` exactly; `use { users Keycloak; }`
makes an `auth OAuth2` system runnable without an external IdP — a
seeded dev realm, two dev users, and a `scripts/token.sh` to mint real
tokens. And the compiler's own diagnostics grew a `fixes` field: the
mechanical, unambiguous ones (a missing capability, a typo'd
provider/field name, a missing required attribute) carry an applyable
edit, the same data an editor's quick-fix and an agent's
check → apply → re-check loop both consume. Details in
[docs/operations.md](docs/operations.md) and
[docs/deployment.md](docs/deployment.md).

v0.16 adds relations and explicit database transactions: a
`Reference<T>` field on a `table`-backed record gets a real, named
foreign-key constraint (`restrict`/`cascade`, enforced by the database
engine) and an optional `unique` constraint, on both targets; a
`transaction { .. }` block groups a handler body's `db.*` writes so
they succeed or fail together — real end to end on the Python backend,
validated-but-not-yet-atomic on Rust (disclosed, not hidden — see
[docs/expressions.md](docs/expressions.md)). The wire contract stays
deliberately narrow: a relation is a flat foreign-key id everywhere
(`customer_id: string`), never a nested embedded object, and
`cardinality: many` has real sema/migration support but no wire
exposure yet. Details in [docs/language.md](docs/language.md)'s
`Reference<T>` section and [docs/expressions.md](docs/expressions.md)'s
`transaction` section.

## Quick start

```sh
curl -fsSL https://raw.githubusercontent.com/jrcalgo/ciac/main/install.sh | sh
# or: cargo install --path crates/ciac
# or: download a binary from the latest GitHub Release

ciac new my-app                     # templates: crud | multi-service | kafka | minimal
cd my-app
ciac check main.ciac
ciac build main.ciac --target python --out ./build
ciac dev main.ciac --target python --out ./build   # watch + regenerate + restart on save
```

Every `ciac new` template is a checked-in example the test suite
already compiles and (where applicable) system-verifies in CI, so a
scaffold always passes `ciac check`. Editing support — the `ciac lsp`
language server (live diagnostics, hover with per-target provider
notes, completion), a VS Code extension (`editors/vscode/`), TextMate
syntax highlighting, and cross-project blueprint reuse via `registry:`
imports — is covered in [docs/authoring.md](docs/authoring.md); the
watch loop in [docs/dev-loop.md](docs/dev-loop.md). An agent working
against this CLI instead of a human has its own front door — `ciac
describe`, `ciac mcp`, generated `AGENTS.md` files — covered in
[docs/agents.md](docs/agents.md).

## Why

Most backend systems are the same dozen architectural patterns glued
together by hand. CIaC models those patterns as a typed graph and moves
their failure modes to compile time: missing queues, unreachable workers,
misplaced authentication, cyclic message flows — all are errors with
stable codes (`ciac explain CIAC0006`) before any code exists.

Because compilation is deterministic — identical input produces
byte-identical output — generated projects can be reviewed, diffed, and
regenerated safely. Business logic lives in generated handler stubs that
are yours to edit; the wiring stays compiler-owned.

Regeneration is manifest-aware:

```sh
ciac build app.ciac --target python --out ./app
# edit ./app/app/services/*.py
ciac diff app.ciac --target python --out ./app --patch
ciac verify app.ciac --target python --out ./app
```

Compiler-owned edits produce `.ciac-new` sidecars instead of overwriting
work. See [docs/regeneration.md](docs/regeneration.md).

## How it works

```text
.ciac source
   │  ciac-syntax      lexer + recovering parser → AST
   ▼
typed system graph (ciac-ir)
   │  ciac-sema        name resolution, crud/events expansion,
   │                   validation passes (cycles, reachability,
   │                   auth placement, composition)
   ▼
NormalizedIr — the validated contract
   │  ciac-codegen     Backend trait + deterministic file tree
   ▼
ciac-backend-python │ ciac-backend-rust │ (your target here)
```

Every backend consumes the same validated IR, so adding a target language
never touches the language or its guarantees. See
[docs/backends.md](docs/backends.md).

A backend doesn't have to be a Rust crate, either (v0.10): `ciac build
--target <name>` falls back to running a `ciac-backend-<name>`
executable found on `$PATH`, speaking a versioned JSON protocol over
stdin/stdout — write one in any language against the published schema
(`ciac codegen-schema`, [docs/protocol-schema.json](docs/protocol-schema.json)).
See [docs/external-backends.md](docs/external-backends.md) and the
worked Go example in [backends/go/](backends/go/).

| CIaC concept | Python target | Rust target | TypeScript target | Go target | Java target |
|--------------|---------------|-------------|--------------------|-----------|-------------|
| API          | FastAPI router | Axum router | Fastify plugin | `net/http` 1.22+ `ServeMux` handler | Spring MVC `@RestController` |
| Service      | async class stub (yours) | async struct stub (yours) | async class stub (yours) | struct + `Handle` method stub (yours) | `@Component` class + `handle` method stub (yours) |
| Worker       | NATS queue-group subscriber or aiokafka consumer group | async-nats or rdkafka consumer group | `@nats-io/transport-node` queue group or kafkajs consumer group | nats.go queue-group subscriber or franz-go consumer group | jnats `Dispatcher` queue group or `@KafkaListener` |
| Job          | croniter task in workers process | cron + Tokio task | croner task in workers process | robfig/cron task in workers process | Spring `@Scheduled` |
| Channel      | FastAPI WebSocket/SSE route | Axum WebSocket/SSE route | `@fastify/websocket`/SSE route | `gorilla/websocket`/SSE route | Spring `@RestController` SSE route |
| Database     | SQLAlchemy async engine (asyncpg / aiomysql / aiosqlite) | SQLx pool (`PgPool` / `MySqlPool` / `SqlitePool`) | Drizzle per instance, raw SQL via `$client` (`pg` / `mysql2` / `better-sqlite3`) | `database/sql` pool (`pgx` / `go-sql-driver/mysql` / `modernc.org/sqlite`) | Spring `JdbcClient` (HikariCP; `postgresql` / `mysql-connector-j` / `sqlite-jdbc`) |
| Cache        | redis-py | redis | ioredis | go-redis | spring-data-redis (Lettuce) |
| Queue        | nats-py or aiokafka | async-nats or rdkafka | `@nats-io/transport-node` or kafkajs | nats.go or franz-go | jnats or spring-kafka |
| Auth (JWT)   | dependency + PyJWT | extractor + jsonwebtoken | preHandler + `jose` | inline route-body check + golang-jwt | inline route-body check + `spring-boot-starter-oauth2-resource-server` |

Full per-provider support lives in [docs/language.md](docs/language.md)
— as of v0.23, every provider above generates on all three targets.

## CLI

| Command | Purpose |
|---------|---------|
| `ciac new DIR [--template crud\|multi-service\|kafka\|minimal]` | Scaffold a new project from a proven example |
| `ciac check file.ciac` | Parse + validate, print diagnostics |
| `ciac build file.ciac --target python\|rust\|typescript\|go\|java --out DIR [--deploy k8s\|terraform\|ci] [--client ts]` | Generate a project, optionally with deploy artifacts and/or a typed TypeScript client |
| `ciac dev file.ciac --target python\|rust\|typescript\|go\|java --out DIR` | Watch, regenerate, restart the compose stack, and re-probe health on every save |
| `ciac diff file.ciac --target python\|rust\|typescript\|go\|java --out DIR` | Preview regeneration drift |
| `ciac verify file.ciac --target python\|rust\|typescript\|go\|java --out DIR [--system]` | Check regeneration drift and generated project validity, optionally running compose-backed system tests |
| `ciac graph file.ciac --format json\|dot` | Dump the system graph |
| `ciac explain CIAC0005` | Explain an error code |
| `ciac describe` | Print the language's full vocabulary as one versioned JSON document |
| `ciac codegen-schema` | Print the external-backend wire-contract JSON Schema |
| `ciac lsp` | Language Server Protocol server over stdio (diagnostics, hover, completion, rename) |
| `ciac mcp` | Model Context Protocol server over stdio (check/build/diff/verify/graph/explain/describe/fix/diff_semantic/rename as tools) |
| `ciac diff file.ciac --semantic [--deny-breaking]` | Compare architecture (not generated files) against a baseline, classifying each change `Breaking`/`Additive`/`Internal` |
| `ciac baseline file.ciac [--update --accept-breaking]` | Create/replace the checked-in semantic baseline `--semantic`/generated CI compare against |
| `ciac rename entry.ciac Old New [--apply] [--out DIR]` | Whole-program, multi-file symbol rename, with transactional `--out` regeneration replay |
| `ciac backfill plan file.ciac --out DIR [--allow-destructive ID]` | The expand/backfill/contract ladder for a breaking storage change |
| `ciac targets` | List code-generation targets |

`check`, `build`, `diff`, and `verify` all accept `--json`: one
machine-readable document on stdout (diagnostics resolved to
file/line/column; for `diff`, the regeneration plan), human narration
on stderr. Mechanical, unambiguous diagnostics also carry a `fixes`
array (v0.15) — the same edits `ciac lsp`'s quick-fix and `ciac mcp`'s
`fix` tool apply, so an editor and an agent's check → apply → re-check
loop consume identical data. `ciac describe` and `ciac mcp` are the
same machine-facing front door for an agent client — see
[docs/agents.md](docs/agents.md).

Multi-file programs (`import "path";`), reusable `blueprint`/`expand`
templates, and cross-project `registry:` imports (v0.12, cached and
pinnable to a git ref) are all accepted directly by `file.ciac` above
— see [docs/blueprints.md](docs/blueprints.md) and
[docs/authoring.md](docs/authoring.md). Deployment (compose, k8s,
Terraform, and `ciac verify --system`) is covered end to end in
[docs/deployment.md](docs/deployment.md).

Architecture changes over time — not just generated-file drift — are
a first-class comparison (v0.18): `ciac diff --semantic` classifies
each change as `Breaking`/`Additive`/`Internal` against a checked-in
baseline, `ciac rename` is a whole-program multi-file rename with
transactional regeneration replay, and `ciac backfill plan` walks a
breaking storage change through an expand/backfill/contract ladder a
human completes one seeded step of. See
[docs/evolution.md](docs/evolution.md).

## Building from source

```sh
cargo build --release            # the compiler
cargo test --workspace           # unit, golden, negative, determinism tests
cargo run -p ciac -- check examples/video-platform.ciac
```

## Repository layout

| Path | Contents |
|------|----------|
| `crates/ciac` | CLI |
| `crates/ciac-diagnostics` | spans, source maps, `CIAC` error codes |
| `crates/ciac-syntax` | lexer, parser, AST |
| `crates/ciac-ir` | typed system graph, `NormalizedIr` |
| `crates/ciac-sema` | graph building, expansion, validation passes |
| `crates/ciac-codegen` | `Backend` trait, shared model, determinism rules |
| `crates/ciac-backend-python` | FastAPI target |
| `crates/ciac-backend-rust` | Axum target |
| `examples/` | valid example programs |
| `editors/` | TextMate grammar + VS Code extension for `.ciac` |
| `tests/` | golden snapshots, negative suite, determinism tests |
| `docs/` | [language](docs/language.md) · [expressions](docs/expressions.md) · [blueprints](docs/blueprints.md) · [authoring](docs/authoring.md) · [dev loop](docs/dev-loop.md) · [agents](docs/agents.md) · [architecture](docs/architecture.md) · [IR](docs/ir.md) · [backends](docs/backends.md) · [external backends](docs/external-backends.md) · [regeneration](docs/regeneration.md) · [evolution](docs/evolution.md) · [deployment](docs/deployment.md) · [operations](docs/operations.md) · [errors](docs/errors.md) |

## License

Apache-2.0
