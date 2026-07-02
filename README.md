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
