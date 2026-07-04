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

Constructs the language accepts but no backend implements yet
(`Kafka`, `scheduler`, `realtime`) pass `ciac check` and are refused by
`ciac build` with `CIAC0011` — if it builds, the generated system
actually does it.

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

| CIaC concept | Python target | Rust target |
|--------------|---------------|-------------|
| API          | FastAPI router | Axum router |
| Service      | async class stub (yours) | async struct stub (yours) |
| Worker       | NATS queue-group subscriber | async-nats + Tokio |
| Database     | SQLAlchemy + asyncpg | SQLx (Postgres) |
| Cache        | redis-py | redis |
| Queue        | nats-py | async-nats |
| Auth (JWT)   | dependency + PyJWT | extractor + jsonwebtoken |

## CLI

| Command | Purpose |
|---------|---------|
| `ciac check file.ciac` | Parse + validate, print diagnostics |
| `ciac build file.ciac --target python\|rust --out DIR` | Generate a project |
| `ciac graph file.ciac --format json\|dot` | Dump the system graph |
| `ciac explain CIAC0005` | Explain an error code |
| `ciac targets` | List code-generation targets |

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
| `tests/` | golden snapshots, negative suite, determinism tests |
| `docs/` | [language](docs/language.md) · [architecture](docs/architecture.md) · [IR](docs/ir.md) · [backends](docs/backends.md) · [errors](docs/errors.md) |

## License

Apache-2.0
