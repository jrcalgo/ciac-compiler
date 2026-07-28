# History

CIaC's version-by-version story — how the language and its five
targets grew, told in the order it happened. This is not the
introduction (see the [README](../../README.md) for that); it's the
narrative every early version of the README carried at the top,
moved here at v0.27 (`29UpdatePlan.md` M3) once the README itself
became a fifteen-minute pitch rather than a changelog.

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
[docs/language.md](../language.md).

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
[docs/operations.md](../operations.md) and
[docs/deployment.md](../deployment.md).

v0.16 adds relations and explicit database transactions: a
`Reference<T>` field on a `table`-backed record gets a real, named
foreign-key constraint (`restrict`/`cascade`, enforced by the database
engine) and an optional `unique` constraint, on both targets; a
`transaction { .. }` block groups a handler body's `db.*` writes so
they succeed or fail together — real end to end on every target as of
v0.24 (`26UpdatePlan.md` M1–M2 closed the Rust backend's own interim
non-atomic gap; see [docs/expressions.md](../expressions.md)). The wire
contract stays deliberately narrow: a relation is a flat foreign-key id
everywhere (`customer_id: string`), never a nested embedded object, and
`cardinality: many` has real sema/migration support but no wire
exposure yet. Details in [docs/language.md](../language.md)'s
`Reference<T>` section and [docs/expressions.md](../expressions.md)'s
`transaction` section.

v0.18–v0.19 add TypeScript and Go as full targets alongside Python and
Rust, restating the same ontology twice more; v0.20 adds Java, closing
five targets at declared parity. v0.21–v0.22 harden the compiler's own
correctness and consistency posture (real Rust transaction atomicity,
structured logging on every target, no-infra OAuth2 scope tests,
dependency/vulnerability scanning in CI, a two-table divergence ledger,
and a frozen, versioned language specification — `LANGUAGE_VERSION
1.0.0`, distinct from the compiler's own version). v0.25 deepens
`ciac sim` on Rust, TypeScript, Go, and Java to match Python's own
simulation coverage — every target can now deterministically simulate
a whole running system, including injected failures, with no Docker.
v0.26 extends simulation across service boundaries: a multi-service
program's `ciac sim` runs the whole system — every service's handlers,
one shared broker and virtual clock, cross-service `call`s routed
in-process — as a single deterministic run, on all five targets. See
each `NNUpdatePlan.md` file at the repository root for the full
milestone-by-milestone record of how each of these arcs actually
shipped.
