# CIaC — Code as Infrastructure Compiler

One `.ciac` source file. Five production-quality backends — Python,
Rust, TypeScript, Go, Java — generated at parity, each idiomatic in
its own ecosystem, byte-identical on every rebuild. The same file
runs as a deterministic simulation of the *whole system*, failures
and all, with no database, broker, or Docker. `ciac build` emits a
real deploy: compose today, Kubernetes or Terraform or CI on request.
The compiler owns the wiring; your business logic lives in generated
stubs that are yours to edit and never overwritten.

CIaC is not a scaffolding template you fork and diverge from, and not
a runtime you deploy — it's a compiler you keep running against a
source file for the system's whole life. If that tradeoff isn't one
you want, or your architecture doesn't fit a typed graph of
services/APIs/pipelines/streams, it's the wrong tool for you.

## Fifteen minutes, start to finish

Install:

<!-- ciac-verify:start id=install -->
```sh
curl -fsSL https://raw.githubusercontent.com/jrcalgo/ciac/main/install.sh | sh
# or: cargo install --path crates/ciac   (needs a Rust toolchain; ~2 minutes)
```
<!-- ciac-verify:end -->

Scaffold a project and look at what you got — `ciac new` writes a
`main.ciac` plus a README telling you what to run next:

<!-- ciac-verify:start id=new-and-check -->
```sh
ciac new my-app && cd my-app && ciac check main.ciac
```
<!-- ciac-verify:end -->

The rest of this walkthrough uses a slightly bigger program — a
record, free CRUD, one handler with a `transaction`, one stream, one
worker — checked into this repository as
[`examples/quickstart.ciac`](examples/quickstart.ciac) so it can never
drift from what actually compiles. Paste it into your own `main.ciac`
to follow along in `my-app`, or clone this repository and point the
commands below at `examples/quickstart.ciac` directly:

```text
service Notes;

use { db Postgres; queue NATS; }

record Note { id: Uuid; title: String; body: String; }
crud Note: Note;                    // free CRUD: /notes

record ArchiveEvent { id: Uuid; note_id: Uuid; }
table ArchiveEvents: ArchiveEvent;
stream NoteArchived: Note;

handler ArchiveNote(note: Note) -> Note {
    transaction { db.insert(ArchiveEvents, ArchiveEvent { id: Uuid.new(), note_id: note.id }); }
    return note;
}
api ArchiveNoteRoute: Note { method: POST; path: "/notes/archive"; }
pipeline ArchiveNoteRoute: ArchiveNote -> publish NoteArchived -> Return;

worker LogArchive on NoteArchived { max_retries: 2; }
handler RecordArchiveEvent(note: Note) -> Note { return note; }
pipeline LogArchive: RecordArchiveEvent;
```

Build it for a target — any of the five:

<!-- ciac-verify:start id=build -->
```sh
ciac build examples/quickstart.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

`--target python` becomes `rust`/`typescript`/`go`/`java` and every
command below still works unchanged — that's the parity claim, made
demonstrable instead of asserted. `./build/app/logic/archive_note.py`
is six lines of real, readable code — the transaction and the return
value, nothing the compiler needed to hide from you.

Now run the system without starting anything — no Postgres, no NATS,
no Docker — and inject a real failure into it:

<!-- ciac-verify:start id=sim -->
```sh
ciac sim examples/quickstart.ciac --target python --out ./build \
    --scenario sim/quickstart.ciac-sim.json
# [PASS] 29-m3-quickstart
```
<!-- ciac-verify:end -->

The scenario ([`sim/quickstart.ciac-sim.json`](sim/quickstart.ciac-sim.json))
fails the archive's own database commit once, asserts the audit row
never landed, retries, and asserts it did the second time — an
end-to-end atomicity proof against real generated code, deterministic,
in milliseconds. Then confirm the generated project itself is sound
and start it:

<!-- ciac-verify:start id=verify-and-dev -->
```sh
ciac verify examples/quickstart.ciac --target python --out ./build
ciac dev examples/quickstart.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

`ciac dev` watches the source, regenerates, restarts the compose
stack, and re-probes health on every save. That's the loop: one file,
five languages, a real simulated system before a single container
starts, and a real one when you're ready for it.

## The map

| CIaC concept | Python | Rust | TypeScript | Go | Java |
|--------------|--------|------|-------------|-----|------|
| API | FastAPI router | Axum router | Fastify plugin | `net/http` `ServeMux` | Spring MVC `@RestController` |
| Database | SQLAlchemy async (Postgres/MySQL/SQLite) | SQLx pool | Drizzle + raw SQL | `database/sql` pool | Spring `JdbcClient` |
| Queue | nats-py / aiokafka | async-nats / rdkafka | `@nats-io/transport-node` / kafkajs | nats.go / franz-go | jnats / spring-kafka |
| Auth | JWT / OAuth2 | JWT / OAuth2 | JWT / OAuth2 | JWT / OAuth2 | JWT / OAuth2 |

Every provider above generates on all five targets; the full table,
every capability (cache, object store, email, search, tracing,
metrics, scheduler, realtime), and every disclosed divergence between
targets lives in [docs/language.md](docs/language.md) and
[docs/backends.md](docs/backends.md) — the honest-disclosure ledger is
itself part of the pitch, not buried.

**Simulation** goes deeper than the walkthrough above: full relational
and broker fakes, virtual time, every failure-injection point, and —
since v0.26 — a whole *multi-service* system (shared broker, shared
clock, cross-service `call`s) simulated as one deterministic run, on
all five targets. See [docs/simulation.md](docs/simulation.md).

**Deployment** turns a build into compose, Kubernetes manifests
(`--deploy k8s`), Terraform (`--deploy terraform`), or a GitHub
Actions workflow that mirrors `ciac verify` exactly (`--deploy ci`);
`ciac verify --system` boots the real compose stack and runs the
generated system tests against it. See
[docs/deployment.md](docs/deployment.md).

**Evolution** is a first-class comparison, not just file diffing:
`ciac diff --semantic` classifies architecture changes as
Breaking/Additive/Internal against a checked-in baseline, `ciac
rename` is a whole-program multi-file rename with transactional
regeneration replay, and `ciac backfill plan` walks a breaking storage
change through an expand/backfill/contract ladder. See
[docs/evolution.md](docs/evolution.md).

**An agent** working against this CLI instead of a human has its own
front door: `ciac describe` (the full vocabulary as one versioned JSON
document), `ciac mcp` (check/build/diff/verify/graph/explain/fix as
MCP tools), and a generated `AGENTS.md` in every scaffolded and built
project. See [docs/agents.md](docs/agents.md).

## Where to go next

- **Evaluating CIaC?** The map above plus
  [docs/backends.md](docs/backends.md)'s divergence ledger is the
  fastest honest read.
- **Building something?** [docs/authoring.md](docs/authoring.md)
  (editor setup, `ciac lsp`, blueprints) and
  [docs/dev-loop.md](docs/dev-loop.md) (the watch loop) cover the
  minutes before and during `ciac build`.
- **Full reference:** [docs/language.md](docs/language.md) (the
  language spec) · [docs/expressions.md](docs/expressions.md)
  (handler bodies, verbs, transactions) ·
  [docs/blueprints.md](docs/blueprints.md) ·
  [docs/architecture.md](docs/architecture.md) · [docs/ir.md](docs/ir.md)
  · [docs/external-backends.md](docs/external-backends.md) ·
  [docs/regeneration.md](docs/regeneration.md) ·
  [docs/operations.md](docs/operations.md) ·
  [docs/errors.md](docs/errors.md).
- **How CIaC got here:** [docs/history.md](docs/history.md) — the
  version-by-version story this README used to open with.

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
| `crates/ciac-syntax` / `ciac-ir` / `ciac-sema` | lexer/parser/AST, typed system graph, validation passes |
| `crates/ciac-codegen` | `Backend` trait, shared model, determinism rules |
| `crates/ciac-backend-*` | one crate per target (python, rust, ts, go, java) |
| `crates/ciac-sim` | the deterministic simulation runtime |
| `examples/` | valid example programs, `sim/` their scenarios |
| `editors/` | TextMate grammar + VS Code extension for `.ciac` |
| `tests/` | golden snapshots, negative suite, determinism tests |
| `docs/` | reference documentation — index above |

## License

Apache-2.0
