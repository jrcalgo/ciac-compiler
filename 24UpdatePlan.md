# CIaC v0.24-file — The Go Backend: Static Binaries at Absolute Parity (implementation plan)

> Implementation plan. Document number ≠ release number (standing
> precedent in 17/22UpdatePlan.md; the release version is assigned at
> execution time). Assumes 22UpdatePlan.md (the backend factory) has
> shipped AND 23UpdatePlan.md's M5 checkpoint has passed — this plan
> begins by reconciling its own estimates against the TypeScript
> arc's measured costs and against any `HostSyntax` contract notes
> that arc produced, exactly as v0.17 M1 began by reconciling against
> real v0.16 output rather than its own plan's prose.
>
> Go has unique in-repo history: the v0.8 backend spike
> (docs/backend-spike-report.md) and the v0.10 external-protocol
> reference backend (`backends/go/` — a real Go module that consumes
> `CodegenRequest` over stdio and generates a ping-level project,
> live-proved through `ciac build --target go`). This plan supersedes
> neither. The external reference stays exactly where it is, as the
> protocol's living documentation and test fixture; the parity
> backend is an INTERNAL crate (`ciac-backend-go`) like Python's and
> Rust's, for the reason 22UpdatePlan.md's audit established: typed
> handler lowering, scope tests, validators, dev-loop integration,
> and simulation have no external-protocol surface, and inventing one
> is unbounded work this arc explicitly does not attempt. The
> relationship is disclosed in docs/external-backends.md so nobody
> reads the two Go artifacts as competing implementations: one
> demonstrates the wire protocol, the other is the product.
>
> **Parity contract:** identical to 23UpdatePlan.md's preamble —
> every capability/provider row, typed handlers (full HIR lowering),
> typed CRUD + keyed-document store, relations, REAL transactions
> (matching Python, exceeding Rust's disclosed non-atomic gap, with
> the standing cross-reference), the shared migration pipeline,
> scopes + the no-live-infra scope-test suite (JWT-only, same stated
> OAuth2 exclusion), OpenAPI embedding, observability with broker
> traceparent propagation, typed call clients, realtime channels,
> generated system tests, compose/k8s/Terraform/CI emission,
> AGENTS.md + ownership manifest discipline, `ciac verify`
> validators, the `ciac dev` loop, vocab/LSP/describe/MCP visibility,
> evolution/rename-replay participation, and the narrow simulation
> slice as a gated final milestone.
>
> **Confidence:** high — arguably the highest of the three language
> plans. Go's standard library carries more of the load than any
> other target's ecosystem (HTTP serving, logging, SQL interface,
> testing, HTTP-test injection are all stdlib), the deployment story
> (CGO-free static binary in a distroless image) is the best of all
> five backends, and the two prior in-repo Go artifacts de-risked the
> model-consumption layer several versions ago. The one pre-declared
> contract amendment (the multiple-return error idiom, Pillar 2) is
> scheduled goldens-first so it lands as a visible contract change,
> never a hack.

## The gap this version closes

Go is the default language of the infrastructure and platform
audience CIaC's whole premise addresses — the people who currently
hand-wire the exact service/queue/database/deploy topology CIaC
compiles are, disproportionately, Go shops. A compiler that generates
infrastructure-shaped systems but cannot emit the infrastructure
community's own language has a credibility gap independent of feature
count.

It is also the deployment-density argument made concrete. A Go
target produces a ~10–20MB static binary in a distroless image — the
strongest possible answer to "what does the generated artifact cost
to run," and the first backend where `docker compose up --build`
times stop being a CI-budget concern: the `generated-system` CI
job's standing comment (a compose build of a Rust service compiles
its whole crate tree inside an uncached container, so the 5-service
media system is excluded from the Rust matrix on time-budget
grounds) names a real pain this target simply does not have. Go's
compose builds are measured in seconds-to-low-minutes, which lets
M7 put Go into system-verification rows Rust had to skip.

Finally, Go completes an instructive spread for the factory: Python
(dynamic, statement-oriented), Rust (static, expression-oriented,
ownership), TypeScript (structural static, GC), Go (nominal static,
GC, multiple-return errors, zero-value semantics). If `lower_core`'s
contract survives all four, Java is a formality — which is exactly
why Java goes last.

### The two Go artifacts, reconciled in detail

Because this repo will contain two Go code generators after this
plan, the division of labor is spelled out beyond the preamble's
sentence, and lands in docs/external-backends.md at M1:

- `backends/go/` (v0.10) remains the **external-protocol
  reference**: a standalone Go module consuming `CodegenRequest`
  JSON over stdio, demonstrating to third-party backend authors how
  to parse `SystemModel`, respect `FieldTypeKind`, and reply with
  `CodegenResponse` files. Its capability level is the protocol's
  honest level (no typed handlers, no validators, no sim), its test
  is the existing `ciac build --target go` live proof, and after the
  factory's M2 protocol bump it is updated once and re-proven. It
  does NOT gain features from this plan.
- `crates/ciac-backend-go` (this plan) is the **bundled parity
  backend**, indistinguishable in kind from Python's and Rust's
  crates. It takes the `go` target id; the external reference moves
  to an explicitly-demo id (`go-external-demo`) with a
  docs/external-backends.md note — renaming the *demo* rather than
  the product, so users typing `--target go` get the real backend
  and protocol authors still have their worked example. The rename
  is the one user-visible external-protocol change this plan makes,
  disclosed in the changelog and trivially discoverable via `ciac
  targets`.

The v0.8 spike report (docs/backend-spike-report.md) stays as
history; its findings (model-consumption viability, the
string-matching hazard that produced `FieldTypeKind`) are cited by
the audit and superseded by this plan's fuller treatment.

## Pillar 1 — Ecosystem selection

Same criteria and the same rejection-recording discipline as
23UpdatePlan.md: most widely accepted/utilized first, maintenance
health second, generation-model fit third (the compiler owns SQL,
migrations, and schemas), typing/ergonomics fourth. Go adds a fifth
criterion the other languages don't have: **CGO-freedom** — every
selection below preserves `CGO_ENABLED=0`, because the static-binary
deployment story is a headline feature of this target, and one cgo
dependency silently destroys it.

A note on the four contested rows before the table, because Go is
the ecosystem where "most-used" and "right for generated code" split
most sharply, and this plan chooses deliberately each time: GORM is
more installed than raw `database/sql` usage patterns, gin more than
chi, mattn/go-sqlite3 more than modernc, confluent-kafka-go carries
the vendor's name — and each loses here to a structural criterion
(compiler ownership of SQL, stdlib-idiom generated code, CGO-freedom
×2) that this plan values above install rank. Those four decisions
are exactly the ones most likely to be challenged in review, so
their reasons are in the table, their fallbacks are named, and each
sits behind a one-module seam that makes reversal a bounded change.
The uncontested rows (nats.go, prometheus, otel, testify, slog) are
the ecosystem's settled answers and get no further defense.

| Concern | Choice | Rejected alternatives, with reasons |
| --- | --- | --- |
| Language/toolchain | Go ≥ 1.23, modules, `gofmt` as the canonical format | — |
| HTTP | **net/http (1.22+ method+pattern ServeMux) + chi v5** for middleware/route grouping | gin: the most-starred framework, but its own context type and binding idioms diverge from stdlib — generated code should read as textbook Go, and gin's `c *gin.Context` is not that; echo: same reasoning; gorilla/mux: archived-then-revived history; chi is 100% `net/http`-compatible (`http.Handler` everywhere), which also makes the scope-test story pure stdlib (`httptest`) |
| Validation | **go-playground/validator v10** struct tags + explicit decode checks | manual-only validation: insufficient parity with pydantic/zod refinements (uuid format, enum membership, required-field presence); validator is the overwhelming standard for tag-based validation |
| Database | **database/sql** + `jackc/pgx/v5/stdlib`, `go-sql-driver/mysql`, **modernc.org/sqlite** | ORMs — GORM: the most-used Go ORM, rejected because an ORM that owns schema, dirty-checking, and SQL conflicts three ways with CIaC owning schema, migrations, and SQL text; ent: codegen-on-codegen (its generated client colliding with ours); sqlc: excellent tool, but it is itself a SQL→Go compiler — a second compiler inside CIaC's output; raw pgx native API: faster but abandons the one-interface-three-engines shape `database/sql` gives for free. Driver notes — mattn/go-sqlite3: the historically most-used sqlite driver but cgo (violates criterion five); modernc.org/sqlite is the pure-Go port, production-proven, decision recorded with mattn as the fallback if a correctness issue ever surfaces |
| Migrations | CIaC's sequential SQL + a small generated runner (ledger table, apply-in-order) | golang-migrate, goose, atlas: all standard, all second migration authorities; rejected on the same principle as every other target — the v0.7 M5 differ is the only author |
| Cache | **redis/go-redis/v9** | rueidis: technically superior pipelining, smaller mindshare; go-redis is the standard, decision recorded |
| Queue: NATS | **nats-io/nats.go** (official) | — |
| Queue: Kafka | **twmb/franz-go** | confluent-kafka-go: official Confluent but cgo (librdkafka) — violates criterion five, the exact tax Rust pays with cmake; segmentio/kafka-go: historically the readable pure-Go choice but development has slowed and its consumer-group implementation lags; franz-go is the current pure-Go consensus for correctness and performance. Decision recorded with segmentio as the named fallback |
| Auth | **golang-jwt/jwt/v5** + **MicahParks/keyfunc/v3** (JWKS) | lestrrat-go/jwx: capable and complete, heavier API surface; golang-jwt is the community successor-of-record to dgrijalva/jwt-go and the most-imported; keyfunc provides the cached, lazily-fetched, kid-matched JWKS keyfunc — the v0.17 M11 laziness bar met by dependency choice, third backend in a row |
| Object store | **aws/aws-sdk-go-v2** `service/s3` | minio-go: simpler API, but "most accepted" is the official SDK; MinIO-in-compose works via endpoint override + path-style either way |
| Email | **wneessen/go-mail** | net/smtp: frozen by the Go team with an explicit pointer to community libraries; go-mail is the maintained standard successor with sane TLS/auth handling for the Mailpit/SES-SMTP cases |
| Search | **opensearch-project/opensearch-go** (official) | — |
| External HTTP | **net/http** stdlib client with per-instance `http.Client` | resty: convenience sugar generated code doesn't need; the generated wrapper is ~40 lines of stdlib |
| Logging | **log/slog** (stdlib, JSONHandler) | zap/zerolog: faster and hugely deployed, but slog is the post-1.21 standard answer, zero-dep, and structurally sufficient for the shared field conventions; decision recorded — the observability module is one file if a user community demands zap |
| Metrics | **prometheus/client_golang** (official) | — |
| Tracing | **go.opentelemetry.io/otel** + `otelhttp` (official) | — |
| Realtime | **gorilla/websocket** + stdlib SSE (http.Flusher) | nhooyr/coder websocket: modern, context-first, smaller base; gorilla is the most-utilized and actively maintained again post-2022-archival; decision recorded with coder/websocket as fallback |
| Scheduler | **robfig/cron/v3** with `cron.ParseStandard` | — effectively the standard; ParseStandard is 5-field and accepts weekday 0–7 natively, so the source expression passes through UNTRANSLATED — no seconds-prefix, no weekday rewrite (contrast Rust's crate); this makes Go the second no-translation target after TS/croner, and the model's `cron_crate_schedule` field is confirmed Rust-only |
| Testing | stdlib `testing` + **stretchr/testify** + `net/http/httptest` | — |
| Lint | validators: `go vet` + `gofmt -l`; CI adds **staticcheck** | golangci-lint: a meta-linter with config surface generated projects don't need; staticcheck is the focused standard |
| Docker | `golang:1.23` build stage → **`gcr.io/distroless/static-debian12`** | scratch: works for pure Go but lacks CA certs/tzdata that the S3/TLS/timezone paths need; alpine: unnecessary once the binary is static |

`TargetInfo` values:

- `project_marker`: `go.mod`
- `migrations_dir`: `migrations/` (identity filename mapping)
- `validate`: `go build ./...` → `go vet ./...` → `gofmt -l .`
  (empty output asserted) → `go test ./...`, all with
  `CGO_ENABLED=0` in the env so a cgo dependency can never sneak in
  unnoticed — the validator IS the static-binary guarantee
- `compose`: `db_url_scheme: "postgres"` (pgx accepts URL DSNs);
  `mysql_url_scheme` — see the DSN note below; `sqlite_url_prefix:
  "file:data/"`, suffix ``; data dir `/data`; `workers_command:
  ["/app/workers"]` (second binary)
- `dev`: `go build ./...` + process restart
- `ci_test_steps`: setup-go@v5 (module cache on) + the validate
  sequence + staticcheck
- `sim`: `None { reason }` until M9, then `Narrow` via the shared
  coverage function

**The MySQL DSN note, contained.** go-sql-driver/mysql does not
accept URL-style DSNs — its form is `user:pass@tcp(host:port)/db`.
Rather than teach the shared compose layer a second URL dialect
(compose emits the same discrete env vars for every target already),
the generated `internal/config` composes the driver DSN from those
discrete vars, and `mysql_url_scheme` goes unused by this backend's
config path. Compose templates: untouched. The containment decision
is recorded here and validated by the existing generated system
tests, which round-trip real capability connections.

## Pillar 2 — Type system mapping and Go-specific semantics

| CIaC | Go | Wire (JSON) | Notes |
| --- | --- | --- | --- |
| `Str` | `string` | string | |
| `Int` | `int64` | number | exact i64 parity with Rust |
| `Float` | `float64` | number | |
| `Bool` | `bool` | boolean | |
| `Uuid` | `string` | string | `google/uuid` for `Uuid.new()` only; stored/carried as string, matching the all-targets TEXT-id decision |
| `Timestamp` | `time.Time` | RFC 3339 | stdlib JSON marshaling is RFC 3339 UTC — free parity |
| `Json` | `json.RawMessage` | any | byte-preserving; handler indexing lowers through a generated helper (below) |
| `enum { A, B }` | `type XStatus string` + typed consts + `Validate()` | string | same wire form; decode-time membership check via validator custom rule |
| `Record` | struct with `json:"snake_name"` tags | object | field names are already snake on the wire for every target |
| `Option<T>` | `*T` | null | **the Go trap, named and mitigated below** |
| `List<T>` | `[]T` | array | nil-slice normalization decided below |
| error records | `type XError struct` implementing `error` | — | route layer maps via `errors.As` to the same status/shape envelope |

**The zero-value/null trap, treated as the plan's top defect risk.**
Go's decoding does not distinguish `{"total": 0}` from a missing
`total` unless the field is a pointer — and CIaC's wire contract
(inherited from pydantic/serde behavior) requires exactly that
distinction: absent-required is a 400, explicit null is only legal
for `Option<T>`, zero is a value. The decided discipline:

- `Option<T>` fields are pointers (`*T`), and ONLY Option fields are
  pointers — a lower_core-enforced invariant, not a convention.
- Non-optional fields decode through a two-pass check: unmarshal into
  the struct, then a generated required-field presence validation
  (validator `required` tags don't distinguish zero from absent for
  value types, so presence is checked against the raw key set — a
  small generated helper, written once, shared by every decode
  site).
- `[]T` marshaling normalizes nil to `[]` so a Go-generated service
  never emits `null` where every other target emits `[]` — decided
  here, asserted by the conformance harness's decode/encode
  boundary cases (built for exactly this trap, then run against
  every target including Java, where records dodge it differently).

**`Json` indexing.** `payload["items"][0]` open-coded against
`json.RawMessage` would be ten lines per site; it lowers instead
through a tiny generated compiler-owned helper (`internal/jsonx`,
~40 lines: `Get(raw, keys ...any) (json.RawMessage, error)`), erroring
on absent paths the same way Python's `KeyError` path behaves —
divergent silent-nil behavior is the bug class this helper exists to
prevent.

**`HostSyntax` leaves and the pre-declared contract amendment.** Go
runs in the `StatementOriented` mode Python already exercises (its
second consumer and first real validation). The expression-leaf
rules, tabled for M4 transcription:

| Leaf | Generated Go |
| --- | --- |
| int/float/str/bool literals | native; float through the shared must-contain-a-dot rule |
| local / field access / index | `name` / `x.Field` / `jsonx.Get(raw, k…)` for Json, native index otherwise |
| record cons (+ `..base`) | struct literal; base-spread lowers to copy-then-assign (`out := base; out.F = …`) since Go lacks spread |
| binary ops | native; string-concat special case via `fmt.Sprintf` for mixed operands; `Int / Int` native i64 division (Rust parity, equivalence-tested) |
| unary | native |
| `if` / `match` | statements assigning a pre-declared variable (sink shaping); enum `match` → `switch` over typed consts with the shared exhaustiveness posture |
| builtins | `uuid.NewString()`, `time.Now().UTC()` |
| value-semantics hook | documented no-op (GC; no moves — the E0382 class is structurally absent) |

`match` on enums lowers to `switch` with the typed consts; string
concat via `fmt.Sprintf` for mixed operands; float fidelity via the
shared rule; `Int` division is native i64 division (Rust parity,
asserted by the equivalence test's division cases). No clone
discipline (GC — the value-semantics hook is a documented no-op). The one genuinely new requirement: **the
multiple-return error idiom.** Every verb leaf emits
`v, err := …; if err != nil { return zero, err }`, which means
lower_core's tail shaping must thread the enclosing function's
zero-value/error-return shape. This is a `HostSyntax` contract
amendment, pre-declared here so it lands by the factory's amendment
procedure: the contract change + identity-syntax golden update + all
existing targets' goldens proven byte-identical land FIRST, then Go's
leaves consume it (M4's internal ordering). If TS's arc already
forced an equivalent generalization, this collapses to a no-op —
reconciled at this plan's M1, per the preamble.

## Pillar 3 — Project shape and the HTTP layer

```text
go.mod  go.sum  Dockerfile  README.md  AGENTS.md  openapi.json
docker-compose.yml  migrations/000N_*.sql
cmd/api/main.go        # chi bootstrap, health, openapi, migrate-on-boot
cmd/workers/main.go    # all workers/jobs/consumers in one process
internal/config/       # env config; DSN composition; zero I/O at init
internal/state/        # AppState struct: lazy db/redis/nats/jwks/clients
                       #   (+ world seam, M9)
internal/schemas/      # record structs, enums, error types, decode helpers
internal/models/       # row structs + explicit Scan column order
internal/db/           # engine-keyed open + migration runner
internal/jsonx/        # the RawMessage path helper (compiler-owned)
internal/observability/
internal/routes/       # one file per api pipeline
internal/logic/        # compiler-owned lowered handlers
internal/services/     # seeded, user-owned stubs
internal/workers/      # subscribe loops + exported HandleMessageOnce
internal/clients/      # typed call clients
internal/routes/scope_test.go   # httptest scope suite (M6)
tests/system/          # shared Python system suite (free, unchanged)
```

`internal/` because generated packages are not a public Go API — the
seeded `services/` files are the user surface, and the ownership
split is the same manifest-enforced discipline every target carries.
Two binaries (`cmd/api`, `cmd/workers`) mirror the Rust main/workers
split and give compose the same two services with no template
changes.

A worked route sketch, pinning the parity properties (envelope,
decode/validate, error mapping, publish-through-state):

```go
// internal/routes/place_order_api.go — Generated by CIaC.
func PlaceOrderAPI(state *state.AppState) http.HandlerFunc {
    return func(w http.ResponseWriter, r *http.Request) {
        payload, err := schemas.DecodeOrder(r.Body) // 400 on failure
        if err != nil { httpx.BadRequest(w, err); return }
        result, err := logic.NewPlaceOrder(state).Handle(r.Context(), payload)
        if err != nil { httpx.MapError(w, err); return }
        result, err = logic.NewRecordAudit(state).Handle(r.Context(), result)
        if err != nil { httpx.MapError(w, err); return }
        if err := state.Publish(r.Context(),
            "sim_vertical_slice.order_created", mustJSON(result)); err != nil {
            httpx.MapError(w, err); return
        }
        httpx.Accepted(w, result) // {"status":"accepted","data":...}
    }
}
```

`httpx` is a ~60-line generated helper (compiler-owned) carrying the
envelope, the 400/500 mapping (`errors.As` against generated error
records → their status; unknown → 500 + slog error, canonical-reason
body) — the `AppError`/`error.rs` analog. `/health` and
`/openapi.json` (embedded via `go:embed` on the emitted
openapi.json — idiomatic and byte-faithful) round out the standard
routes. Context propagation (`r.Context()`) threads through handlers
because tracing and clean shutdown need it — a Go-idiom requirement
the handler signature absorbs (handlers take `ctx` as their first
parameter; the model's `HandlerRef` needs nothing new since the
signature is a template concern).

**HTTP behavior parity, itemized** (the same checklist plan 23
pins, restated only where Go's answer differs): JSON content type on
all endpoints; 400 with JSON body on malformed/invalid payloads
(decode + presence check + validator, in that order); router
defaults for 404/405 (ServeMux 1.22 pattern matching gives correct
405s natively); 401 missing/invalid token vs 403 missing scope;
health `{"status":"ok"}` for `--live`. All pinned by the same three
layers (C3 OpenAPI equality, smoke test, system tests).

A second worked sketch — the schemas/decode layer, because the
presence-check discipline is this plan's most important generated
detail:

```go
// internal/schemas/order.go — Generated by CIaC.
type Order struct {
    ID    string  `json:"id" validate:"required,uuid4"`
    Total float64 `json:"total"`
}

// DecodeOrder distinguishes absent, null, and zero exactly as the
// other targets' decoders do: absent required field -> error;
// explicit null on a non-Option field -> error; zero -> value.
func DecodeOrder(r io.Reader) (Order, error) {
    var keys map[string]json.RawMessage
    if err := json.NewDecoder(r).Decode(&keys); err != nil {
        return Order{}, fmt.Errorf("malformed body: %w", err)
    }
    if err := requireKeys(keys, "id", "total"); err != nil {
        return Order{}, err
    }
    var out Order
    if err := unmarshalStrict(keys, &out); err != nil { return Order{}, err }
    return out, validate.Struct(out)
}
```

`requireKeys`/`unmarshalStrict` live in one generated helper file —
written once, used by every decode site, boundary-tested by the
shared conformance cases. The statement-lowering table completing
the `HostSyntax` picture:

| HIR statement | Generated Go |
| --- | --- |
| `Let { name, value }` | `name := <expr>` (with the error-idiom threading where the value is a verb call) |
| `Expr(value)` | `_ = <expr>` or bare call per value-use analysis (mirrors the unused-let tolerance) |
| `Return(Some(v))` | `return <expr>, nil` |
| `Return(None)` | `return nil` (or zero-value pair per signature) |
| `Fail { error, args }` | `return zero, &schemas.XError{…}` |
| `Publish { stream, value }` | `if err := state.Publish(ctx, subject, mustJSON(v)); err != nil { return zero, err }` |
| `Transaction { body }` | `tx, err := state.DB.BeginTx(ctx, nil); defer rollback; <body with tx-threaded leaves>; tx.Commit()` |

## Pillar 4 — Database, transactions, migrations

**Placeholders and bind order.** pgx-stdlib takes `$N`; mysql and
modernc-sqlite take `?` — exactly the Rust configuration, so the
shared `sqlph` filter and the v0.13 M1 fields-first-id-last UPDATE
bind order apply with zero new logic, and the conformance topology
assertion pins the SQL text to the other targets byte-for-byte.

**Verb lowering table** (M4's transcription target):

| Verb | Generated Go shape |
| --- | --- |
| `db.insert(T, v)` | `row := v; _, err := state.DB.ExecContext(ctx, INSERT_SQL, binds…); …; row` — world-guarded in M9 |
| `db.get(T, id)` | `QueryRowContext` + explicit `Scan` into the model struct; `sql.ErrNoRows → nil` |
| `db.update(T, id, v)` | fields-first-id-last; `RowsAffected()==0 → nil` |
| `db.delete(T, id)` | `RowsAffected() > 0` |
| `db.query/count/delete_where` | shared predicate SQL + binds |
| `cache.*` | go-redis with the shared JSON codec |
| `object_store.*` / `email.send` / `search.*` / `http.call` | the generated wrappers of Pillar 6 |

**Transactions.** `sql.Tx` via `BeginTx` + defer-rollback +
explicit commit — REAL atomicity (Python parity, exceeding Rust's
disclosed gap; standing cross-reference maintained). The lowered
`transaction {}` block passes the `*sql.Tx` to enclosed verb leaves
through the same mechanism the Python session threading uses in the
shared dispatch — the tail-shaping work Pillar 2's amendment covers.

**Migrations.** The generated ledger-table runner
(`internal/db/migrate.go`): `_ciac_migrations` table, apply-in-order,
per-file transaction where the engine supports transactional DDL,
idempotent. Runs at `cmd/api` boot; `cmd/workers` performs a
bounded wait-for-ledger-current check rather than racing to apply —
matching whichever behavior the Python workers process actually
exhibits, verified by reading it at implementation time
(reconciliation-first, per house rules), and recorded in the
template comment.

**Relations.** Shared FK-bearing migration SQL; constraint
violations surface as driver errors, disclosed-unmapped exactly like
the other targets (parity of gaps, tracked cross-target).

**Lazy state.** `database/sql` pools are lazy by construction
(connections on first use); go-redis dials on first command; NATS
behind a `sync.Once`-guarded connect; keyfunc fetches JWKS on first
verification. The M1 no-infra construction test (state built against
unreachable endpoints, no error, no goroutine leak — asserted with
`goleak`) makes the v0.17 bar a tested property from day one.

## Pillar 5 — Broker, workers, jobs, channels

**NATS:** `nc.QueueSubscribe(subject, group, handler)` — queue-group
parity in one call. **Kafka:** franz-go consumer group (groupId =
queue group, topic = subject), manual commit after successful
handling for at-least-once — the v0.11 M3 delivery contract; record
headers carry traceparent both directions.

**Workers.** Same seam discipline as every target:

```go
const Subject = "sim_vertical_slice.order_created"
const QueueGroup = "sim-vertical-slice-process-order"
const MaxRetries = 2

func HandleMessageOnce(ctx context.Context, s *state.AppState,
    payload schemas.Order) error { /* lowered steps */ }

func handleMessage(...) error { /* retry loop, attempt 0..=MaxRetries */ }

func Run(ctx context.Context, s *state.AppState) error {
    /* concurrency goroutines × subscribe loop */
}
```

`HandleMessageOnce` exported for the M9 sim runner and attempt
counting; `concurrency` maps to N goroutines sharing the group
subscription (NATS) or N-consumer container concurrency (franz-go),
matching each engine's semantics as the other backends map them.

The full worker module sketch, since the concurrency mapping is the
one Go-specific structural choice in it:

```go
// internal/workers/process_order.go — Generated by CIaC.
func Run(ctx context.Context, s *state.AppState) error {
    g, ctx := errgroup.WithContext(ctx)
    for i := 0; i < Concurrency; i++ {
        g.Go(func() error { return consume(ctx, s) })
    }
    return g.Wait()
}

func consume(ctx context.Context, s *state.AppState) error {
    nc, err := s.NATS(ctx)
    if err != nil { return err }
    sub, err := nc.QueueSubscribeSync(Subject, QueueGroup)
    if err != nil { return err }
    for {
        msg, err := sub.NextMsgWithContext(ctx)
        if err != nil { return err } // ctx cancellation ends the loop
        payload, err := schemas.DecodeOrderBytes(msg.Data)
        if err != nil { slog.Warn("discarding malformed message"); continue }
        if err := handleMessage(ctx, s, payload); err != nil {
            slog.Error("message processing failed after retries", "err", err)
        }
    }
}
```

(`errgroup` is `golang.org/x/sync` — quasi-stdlib, recorded.) Every
beat mirrors the other targets' workers: malformed-discard, retry
loop behind `handleMessage`, per-attempt entry exported.

**Jobs.** `cron.ParseStandard(schedule)` — untranslated; the run
loop computes next-fire and sleeps under context; `HandleTickOnce`
exported. `catch_up` per the shared contract:

```go
// internal/workers/reconcile.go (job) — Generated by CIaC.
const Schedule = "0 3 * * *"

func Run(ctx context.Context, s *state.AppState) error {
    sched, err := cron.ParseStandard(Schedule)
    if err != nil { return err }
    for {
        next := sched.Next(time.Now().UTC())
        select {
        case <-ctx.Done(): return ctx.Err()
        case <-time.After(time.Until(next)):
            if err := HandleTickOnce(ctx, s); err != nil {
                slog.Error("scheduled job Reconcile failed", "err", err)
            }
        }
    }
}
```

**Channels.** gorilla upgrade / SSE flusher loop bridging a plain
(non-group) subscription — fan-out semantics matching
`channel.py.j2`/`channel.rs.j2`, probed by the existing generated
system tests unchanged.

## Pillar 6 — Auth, scopes, ontology remainder, observability

**Auth.** golang-jwt parses/verifies HS256 (JWT provider, shared
secret) and RS256 with keyfunc's cached lazy JWKS (OAuth2). The
generated middleware extracts claims (`sub`, `scopes`), and
`RequireScope` produces the same 403 semantics; scoped routes wrap
in the middleware per the shared scope collection.

**Scope tests.** `httptest.NewRequest` + `router.ServeHTTP` — pure
stdlib, no listener, the cleanest oneshot analog of all five
backends; the generated `scope_test.go` mints HS tokens with the
test secret and asserts the 403-without/200-with pair per scope.
JWT-only with the standing OAuth2 exclusion comment (real RS256
needs a real issuer — same sentence as the other targets).

**Ontology.** aws-sdk-go-v2 S3 wrapper (endpoint override +
UsePathStyle for MinIO; same five config fields); go-mail against
Mailpit (same six); opensearch-go (same one); stdlib-client
`ExternalHTTP` wrapper per instance and generated `clients/<svc>`
call clients on the same base-URL env convention.

**Observability.** slog JSONHandler with the shared field
conventions; promhttp `/metrics` when declared; otel SDK +
otelhttp middleware/transport when `tracing` is declared, OTLP env
conventions identical, broker header propagation per Pillar 5 —
proven by the cross-target trace test extended to four targets.

**Deployment.** Dockerfile: `golang:1.23` build (`CGO_ENABLED=0
go build -trimpath -ldflags="-s -w"`) → `distroless/static-debian12`
with the two binaries; `.dockerignore` minimal (no build cache dirs
in context).

**Scope-test sketch**, because httptest's purity is the selling
point (no framework test kit at all):

```go
// internal/routes/scope_test.go — Generated by CIaC.
func TestOrdersWriteScopeEnforced(t *testing.T) {
    state := teststate.NoInfra(t) // lazy state, unreachable endpoints
    router := routes.Router(state)

    denied := httptest.NewRecorder()
    router.ServeHTTP(denied, signedReq(t, "POST", "/orders",
        orderBody(), scopes()))
    require.Equal(t, http.StatusForbidden, denied.Code)

    allowed := httptest.NewRecorder()
    router.ServeHTTP(allowed, signedReq(t, "POST", "/orders",
        orderBody(), scopes("orders:write")))
    require.NotEqual(t, http.StatusForbidden, allowed.Code) // mechanism proof
}
```

### The config/env surface

Same cross-target env-var contract as every backend (the compose/k8s
layer emits the names; `internal/config` reads them); Go-specific
rows only: `DATABASE_URL` parsed by pgx directly for Postgres;
MySQL's driver DSN composed from the discrete vars per Pillar 1's
containment note; sqlite as a file path under `/data`. Everything
else — `REDIS_URL`, `NATS_URL`/`KAFKA_URL`, `JWT_SECRET`/
`OAUTH_ISSUER`, `<SVC>_URL` call targets, S3/email/search instance
fields, `OTEL_*` — identical names, identical semantics, per the
table in plan 23 (not repeated; the contract is shared, which is
the point).

### Template inventory

Estimate: ~34 templates, ~2,500–2,800 lines (Go's stdlib leverage
shows up as fewer wrapper templates than TS), checked at M5:

| Group | Templates |
| --- | --- |
| project | `go.mod`, `Dockerfile`, `README.md`, `system-README.md` |
| binaries | `main.go` (api), `workers_main.go` |
| app core | `config.go`, `state.go`, `observability.go`, `httpx.go`, `jsonx.go` |
| data | `schemas.go` (+ decode helpers), `models.go`, `db.go`, `migrate.go`, `resource_store.go` |
| http | `route_api.go`, `resource_api.go`, `channel.go`, `router.go` |
| async | `worker.go`, `consumer.go`, `job.go`, `queue.go` |
| handlers | `logic.go` (compiler-owned), `service.go` (seeded stub) |
| ontology | `cache.go`, `object_store.go`, `email.go`, `search.go`, `http_clients.go`, `auth.go` |
| tests/sim | `scope_test.go`, `smoke_test.go`, `noinfra_state_test.go`, `sim_runner.go` (M9) |

Every row has a named analog in the audited Python/Rust inventories —
the same no-novel-file-kinds parity check plan 23 states.

## Implementation map

| Artifact | Content |
| --- | --- |
| `crates/ciac-backend-go/src/lib.rs` | `TargetInfo`, `go_type` filter, emission table, gating ladder |
| `crates/ciac-backend-go/src/lower.rs` | `HostSyntax for GoSyntax` — leaves only, including the error-idiom threading |
| `crates/ciac-backend-go/templates/` | the ~34 templates above |
| `crates/ciac/src/commands.rs` | ONE registry line |
| `tests/tests/snapshots/` | `gen__go__*` goldens (registry-enumerated) |
| `.github/workflows/ci.yml` | `generated-go` job + system rows (incl. the multi-service row Rust's budget excluded) |
| docs | backends.md section, external-backends.md clarification, simulation.md column (M9) |
| shared | ONE pre-declared lower_core amendment (error idiom), goldens-first |

## Capability parity checklist

Same matrix discipline as plan 23 (module / proving example /
milestone); rows identical except the implementing modules are the
Go files above and the milestone mapping matches this plan's ladder
(M1 core+envelope, M2 data, M3 async+channels, M4 handlers+
transactions+relations, M6 auth+scopes, M7 ontology+clients+
observability+system rows, M8 integration surfaces, M9 sim). The
signed-off copy lives in M8's milestone notes with each row's golden
and proof linked — the table is the definition of done, not
decoration.

## Determinism and supply chain

`go.mod` pins exact versions; `go.sum` is emitted AND
golden-snapshotted (the transitive pin); validators run with
`GOFLAGS=-mod=readonly` so generation-time and CI builds can never
silently update deps; the Go toolchain version is pinned via the
`toolchain` directive. Generated code is emitted gofmt-canonical and
a test asserts gofmt idempotence — formatting is golden bytes, not a
post-pass. `CGO_ENABLED=0` in every validator/CI/Docker invocation
is the standing static-binary assertion.

## Pillar 7 — Simulation (gated) and the divergence ledger

| Row | Python | Rust | Go |
| --- | --- | --- | --- |
| sim | full, record/replay | narrow, no replay | narrow slice, M9 (gated); no replay, disclosed |
| scope tests | full | JWT-only | JWT-only, same reason |
| `transaction {}` | atomic | disclosed non-atomic | atomic |
| `Int` | arbitrary | i64 | int64 |
| `Option` decode | native | native | pointer + presence check (Pillar 2), boundary-tested |
| nil-slice `List` | n/a | n/a | normalized to `[]`, disclosed |
| cron translation | none | seconds-first + weekday rewrite | none |
| deploy artifact | image + venv | ~stripped binary image | static binary + distroless (best of five) |

M9 mirrors the v0.17 M11 continuation: `internal/world` is a narrow
restatement (Go cannot vendor `ciac-sim`'s Rust source — Python's
disclosed position, same docstring discipline): fake table map +
fake queue + occurrence-counted failure rules (`error` action only).
Its shape, sketched because the restatement discipline (same
semantics, same counters, same refusal posture as `world.rs`) is the
whole point:

```go
// internal/world/world.go — Generated by CIaC (sim builds only).
// A narrow restatement of ciac-sim's SimWorld: Go cannot consume the
// Rust crate's source the way the Rust backend vendors it, so this
// mirrors sim/pyrunner/world.py's disclosed position. Only db.insert
// and broker publish/consume are faked; only the `error` failure
// action is implemented; unmatched rules are surfaced, not ignored.
type World struct {
    mu       sync.Mutex
    tables   map[string][]json.RawMessage
    queue    [](struct{ Subject string; Payload []byte })
    failures *failureEngine // occurrence-counted (effect, subject) rules
}

func (w *World) DBInsertChecked(table string, row json.RawMessage) error {
    if w.failures.shouldFail("db.commit", table) {
        return errors.New("simulated db.commit failure (injected)")
    }
    w.mu.Lock(); defer w.mu.Unlock()
    w.tables[table] = append(w.tables[table], row)
    return nil
}
```
`state.Publish` and the `db.insert` leaf gain the world-guard
branch. A generated `cmd/sim_runner` (or `sim_runner_test.go` driven
binary — decided at implementation against how `TargetInfo.validate`
interacts with extra binaries, recorded when decided) drives
`httptest` for requests (real status codes), `HandleMessageOnce`
retry budgets for drains, `robfig`-computed due-instants for
advances, world state for expects, and prints the one-line
`SimScenarioOutcome` JSON. `ciac sim --target go` goes through
`SimSupport::Narrow` with the shared `unguarded_verbs` gate.
Acceptance: both checked-in scenarios reproduce
`{"ProcessOrder":3}/{"Reconcile":1}` and
`{"ProcessOrder":100}/{"Reconcile":7}` exactly; order-system refusal
names its reasons; sim-vertical-slice × go joins the ratchet CI
matrix. The runner's dispatch is the fleet architecture (plan 23's
sketch) in Go spelling — same closed step vocabulary, same
`SimScenarioOutcome` line, same first-worker-per-subject drain
semantics with the shared per-`(subject, group)`-cursor gap
disclosed in `world.go`'s doc comment verbatim from `world.rs`'s.

**The equivalence suite** (plan 23's specification) gains Go's
distinguishing cases when this arc lands: i64 division vs Python's
`//` and TS's `Math.trunc`; the absent/null/zero decode triple;
nil-slice normalization; `time.Time` RFC 3339 formatting at the
sub-second boundary (Go trims trailing zeros — asserted-as-documented
if it differs from the other targets' formatters, another
divergence-ledger row discovered by writing the case, which is the
suite doing its job).

**Multi-service, `ciac new`, and docs surfaces** follow plan 23's
pattern verbatim: per-service directories under the shared system
compose; scaffold templates for `--target go` in M8; provider-table
rows flip via the registry-derived tables as `supports()` un-gates.

## Diagnostics, gating, and docs impact

Gating via `supports()` per milestone with CIAC0011, as always; the
conformance harness reports gated pairs as disclosed skips. No new
error codes expected; any Go-specific diagnosable condition lands
with a code + docs/errors.md entry through the standard procedure.
Docs: generated provider table flips rows as milestones un-gate;
docs/backends.md gains the Go section (deps + divergence ledger +
the two-Go-artifacts disclosure); docs/simulation.md gains the Go
column at M9; docs/external-backends.md gains the
reference-vs-parity clarification at M1.

Deployment-layer interaction, noted for completeness because it is
all inherited: k8s manifests and Terraform modules are emitted by
the shared generators with zero Go-specific logic (the image is the
only per-target fact, and it comes from the generated Dockerfile);
the `--profile` sizing applies unchanged; the generated project's
own CI workflow (`--deploy ci`) picks up `ci_test_steps` from
`TargetInfo`. The keyed-document store's cache-aside path (when
`crud` declares `cache_ttl`) follows the Python store template's
read-through/invalidate shape against go-redis — same TTL semantics,
same key convention, asserted by the capability round-trips the
system tests already run.

## Relationship to the forecast documents

Same posture as plans 22/23: v0.19–v0.21 remain open forecasts;
this plan neither blocks nor consumes them (23 consumed v0.21's TS
candidate; Go was never a v0.21 candidate — it graduated from
spike/reference history instead). If a forecast track lands between
plans, M1's reconciliation absorbs the drift. One forward note:
v0.19's outbox machinery, when it executes, adds a per-target
transactional-write-plus-publish template concern — this plan's
`sql.Tx` transaction leaf and `state.Publish` seam are exactly the
two attachment points it will need, called out so the v0.19
planning pass finds them named.

## Milestones

1. **M1 — Reconcile + skeleton to ping-parity.** Reconcile estimates
   against TS-arc actuals and any contract amendments (recorded in
   this file). Copy the skeleton; register `TargetInfo` (the one
   external line — factory assertion #2). Emit go.mod/Dockerfile/
   README/AGENTS.md/config/state (with the goleak-backed no-infra
   construction test)/observability/httpx/health/openapi-embed.
   ping verifies fully locally (Go toolchain present): build, vet,
   gofmt, test. Goldens begin; docs/external-backends.md
   clarification lands; cold/warm build times recorded.

   **Shipped (v0.24 M1):** `crates/ciac-backend-go` — `GoBackend` with
   `TargetInfo` (`project_marker: "go.mod"`, `validate`: `go build
   ./...` → `go vet ./...` → `gofmt -l .` → `go test ./...`, all with
   `CGO_ENABLED=0`; `ci_test_steps` via `actions/setup-go@v5` +
   staticcheck; `dev.rebuild` empty/`RestartStyle::Restart`; `sim:
   None` until M9). `supports()` is gated to exactly `Component::Api`
   — narrower than TS's own M1 (`Api` alone, no `Service`): ping's
   `pipeline Echo: Return` binds no handler, so no `Service` node is
   even in play, and claiming that kind now (before `route_api.go.j2`
   implements any handler dispatch) would pass gating and then fail on
   an undefined template variable instead of a clean `CIAC0011` — the
   exact crash class `supports()` exists to prevent. `AGENTS.md`
   needed **zero** code in this crate: `backends/skeleton-internal`'s
   own doc comment claims "a real backend's `AGENTS.md` (see
   `ciac-backend-python`/`-rust`'s)" but neither actually emits one —
   `crates/ciac/src/commands.rs::agents_md()` adds it centrally, for
   every registered target, target-neutrally, after `generate()`
   returns. A real, stale claim in checked-in reference code, found by
   grepping for the precedent this milestone's own doc comment cited
   rather than trusting it — corrected here, not fixed in the
   skeleton (out of this milestone's scope).

   **Routing, decided** (Pillar 1's own pre-registered open question
   #3): dropped `chi` for plain stdlib `net/http` 1.22+ `ServeMux`.
   Every route shape this compiler's model can emit (static paths,
   single `{id}` params, method-first dispatch) is covered by pattern
   matching alone; `chi` would add a dependency and an indirection
   (`chi.URLParam` vs. `r.PathValue`) for route-grouping/middleware
   this backend's own `httpx` helpers already give by plain function
   wrapping. Recorded in `lib.rs`'s own doc comment since Pillar 1's
   table named `chi` as the pick.

   **`gofmt`, decided as a real generation-time dependency, not a
   post-pass approximation.** The plan's own words — "generated code
   is emitted gofmt-canonical... formatting is golden bytes, not a
   post-pass" — ruled out committing bytes that merely happen to
   already look right and hoping `ciac verify`'s own `gofmt -l` check
   agrees. `gofmt` owns two things no Jinja template can reproduce
   without re-implementing the formatter: struct-field column
   alignment (depends on every *sibling* field's rendered width, not
   any one template's local context) and empty-composite-literal
   collapse (`T{\n}` -> `T{}`, hit immediately by `ping`'s own
   zero-capability `Config{}`). `emit_service`'s `render_go` shells
   out to the real `gofmt` binary on every `.go` file's rendered
   content before it ever reaches `project.add_file` — a disclosed,
   narrow new dependency: `gofmt` must be on `PATH` to *generate*
   `--target go` output, not only to validate it. `cargo test
   --workspace` now needs Go on `PATH` too (both `ciac-backend-go`'s
   own tests and the registry-driven golden/conformance suites call
   `generate()`), so CI's `test` job gained `actions/setup-go@v5`
   alongside the new dedicated `generated-go` job.

   **A real C3 conformance collision, found and fixed.** `go:embed`
   cannot reach outside its own file's directory (no `..` in embed
   patterns, unlike Rust's `include_str!("../../openapi.json")`), so
   `cmd/api/main.go` needs its own colocated copy of the project-root
   `openapi.json` to embed at build time. The first attempt named it
   `cmd/api/openapi.json`, then `embedded_openapi.json` — both still
   match `tests/tests/conformance.rs`'s C3 check, which finds every
   `openapi.json` file by `path.ends_with("openapi.json")`, a plain
   string suffix check with no path-segment awareness. Landed as
   `cmd/api/apidoc.json` instead — the fix lives entirely in this
   crate (a naming choice), asking C3 to learn a Go-specific exception
   was never necessary once the actual matching rule was read
   correctly.

   **Live-verified**, not just golden-generated: `ciac build
   examples/ping.ciac --target go` end to end against the real
   toolchain (Go 1.24.7 locally; `go.mod` pins `go 1.23` +
   `github.com/go-playground/validator/v10 v10.27.0` +
   `go.uber.org/goleak v1.3.0`, both chosen for a go-version floor at
   or below 1.23 rather than each library's own `@latest`, which
   would have forced a `toolchain go1.25.0` directive and a
   network-dependent auto-download this sandbox happened to have but
   a locked-down CI runner might not) — `go build`/`go vet`/`gofmt
   -l`/`go test` all clean, `CGO_ENABLED=0` throughout. The built
   binary answers real HTTP: `/health` -> `{"status":"ok"}`, `POST
   /echo` with a valid body -> `{"status":"accepted","data":{...}}`,
   and Pillar 2's own zero-value/null boundary triple exercised for
   real against `internal/schemas`'s `requireKeys` + presence/null
   check + `validator.Struct`: a missing `text` key -> 400 `missing
   required field "text"`; an explicit `"text":null` -> 400 `field
   "text" must not be null`; `"text":""` (a legitimate zero value) ->
   200, proving the `validate` struct tags deliberately carry format
   constraints only (`uuid4`), never `required` — re-asserting
   "required" through `validator` would have rejected that last case,
   exactly the trap Pillar 2 named. An invalid `id` -> 400 naming the
   failed `uuid4` tag. `go build` timing (this sandbox, not
   representative of CI hardware): ~10.9s cold (`go clean -cache`
   first), ~0.5s warm (build cache hit, no source changes) — the
   plan's own "Go's compose builds are measured in seconds-to-low-
   minutes" claim already reads true at the single-binary scale M1
   proves.

   **The external Go artifact, reconciled.** `backends/go/` (the v0.10
   external-protocol demo) is renamed to `ciac-backend-go-external-demo`
   / `--target go-external-demo`, live-verified still reachable and
   generating correctly under the new name — the one user-visible
   external-protocol change this plan makes, exactly as disclosed in
   the preamble. `docs/external-backends.md` gained the two-Go-
   artifacts reconciliation paragraph and corrected build/invoke
   commands; `backends/go/README.md` and `main.go`'s own self-
   referential strings (error prefix, generated-by note) updated to
   match — a real backend author copying that worked example verbatim
   today gets the right name, not a stale one.

2. **M2 — Records, schemas, models, CRUD, keyed store, migrations.**
   Structs/enums/error types/decode helpers (the presence-check
   discipline lands here with its boundary tests); models with
   explicit Scan order; engine-keyed open + runner; typed CRUD +
   keyed store on all three engines with shared placeholder/bind
   discipline. sqlite-notes fully local with zero Docker (pure-Go
   sqlite — the strongest local proof of any target); crud/mysql
   static-local, round-trips CI-delegated.

   **Shipped (v0.24 M2):** `supports()` gained `Component::Database`
   (all three engines at once — `database/sql`'s interface is
   engine-agnostic, so unlike TS's own per-driver M2 cost there is no
   per-engine gating to stage) and, once un-gated, one real
   discovery: `crud <Name>[: <Record>];` does **not** expand into
   `Api`+`Database` alone. `ciac_sema::build::crud` also synthesizes a
   `Component::Service { name: "<Name>Store", signature: None }`
   marker node — found empirically (`examples/sqlite-notes.ciac`
   refused on it the moment `Database` alone passed), not from reading
   the sema source first. `signature: None` is the documented
   "classic binding-only handler" discriminant, exactly the shape
   `resource_store.go.j2`/`resource_api.go.j2` already render
   unconditionally from `ctx.resources` with no handler body to lower
   — so the fix was widening the gate to `signature: None` specifically
   (never `Some(_)`, which is M4's typed-handler feature), then
   confirming by generating **every** checked-in example against the
   new gate that only `crud`-synthesized Store nodes are ever
   `signature: None` among newly Api+Database-reachable programs (a
   bare classic handler not tied to `crud` would need a `ctx.services`
   seeded-stub feature this milestone doesn't build, and none exists
   among reachable examples to expose the gap).

   Shipped: `internal/db` (engine-keyed `Open` — one function, since
   `database/sql` needs no per-engine call-shape branching at all,
   the sharpest contrast yet with TS's own M2 cost; `EnsureSchema<Engine>`
   for CRUD resources; a ledger-based `Migrate` for `table` declarations,
   generic and unexercised by any currently-reachable example, reading
   `migrations/*.sql` from disk at runtime rather than `go:embed` — the
   same directory-crossing constraint M1's `openapi.json` hit, resolved
   the same way M1 resolved it for Python: read from disk, not embed,
   with the Dockerfile copying `migrations/` into the runtime image);
   `internal/models` (row structs in Scan order + `<Name>In` payloads +
   `Decode<Name>In` presence-checked decoders for typed resources);
   per-resource `internal/services/<resource>.go` stores and
   `internal/routes/<resource>_api.go` REST handlers (create/list/get/
   update/delete); `db_pascal`-cased `AppState` db-pool fields, lazily
   opened by `database/sql.Open` itself (never dials).

   **Three real bugs found by live-proving against `sqlite-notes.ciac`
   end to end, not just golden-generating it:**
   1. `sql.Open` is lazy about *dialing* but not about *goroutines* —
      it unconditionally starts one supervisory `connectionOpener`
      goroutine per pool (stdlib behavior, does no I/O, only wakes on
      an actual query). The M1 goleak-backed no-infra test failed on
      exactly this the first time a db instance existed; fixed with
      `goleak.IgnoreTopFunction("database/sql.(*DB).connectionOpener")`,
      conditioned on `c.has_db` so the claim stays precise rather than
      blanket-permissive.
   2. sqlite's data file couldn't be created (`unable to open database
      file: out of memory (14)` from modernc.org/sqlite) because
      `data/` didn't exist yet — `db.Open` now `os.MkdirAll("data",
      0o755)`s for the sqlite engine specifically, mirroring the other
      backends' own `create_dir_all("data")` before their lazy pool
      construction.
   3. The CRUD row struct (`models.Note`) had no `json` tags at all,
      so create/get/list/update responses serialized as
      `{"ID":...,"Title":...}` instead of the wire-contract
      `{"id":...,"title":...}` every other target uses — caught by
      actually reading a live response, not by any static check (Go
      happily compiles and `gofmt`-formats a tagless struct). Fixed by
      tagging every field on both the row struct and the `table`-decl
      row struct.

   **The zero-value/null boundary triple (Pillar 2's "centerpiece"),
   live-verified against real CRUD requests, not just unit-tested:**
   the first working version silently accepted a `POST /notes` with no
   `title` at all, since `resource_api.go.j2`'s handlers decoded
   `<Name>In` payloads with plain `encoding/json.Decode` — schemas.go's
   own presence-check discipline (M1) was never wired into the CRUD
   path. Fixed by giving `internal/models` its own `Decode<Name>In`
   (a `requireKeys` + typed-unmarshal pair, structurally identical to
   `schemas.Decode<Record>` but scoped to the create/update field set,
   i.e. excluding `id`) and routing `create`/`update` through it for
   typed resources. Live-verified all three cases against the running
   binary: a missing `title` → 400 `missing required field "title"`;
   an explicit `"title":null` → 400 `field "title" must not be null`;
   `"title":""` (a legitimate zero value) → 201/200, proving the
   generic (no-record) resource path's `Data json.RawMessage` stays
   correctly optional (`#[serde(default)]`'s Go analog: absent simply
   decodes to `nil`, matching Rust's own choice) while typed fields do
   not.

   **Live-verified**, full CRUD lifecycle against a real SQLite file
   (not just `go build`): create → 201 with the row echoed back with
   a server-generated UUID; get/list → 200 with the row(s); update →
   200 with the new value persisted; delete → 204 then a subsequent
   get → 404. `gofmt -l`/`go vet`/`go test` (goleak included) all
   clean throughout. mysql-notes/crud-notes stay `CIAC0011`-gated on
   `auth`/`cache`, exactly as before — Database's own un-gating didn't
   silently widen anything beyond the one example that needed it.

   Full verification: fmt/clippy/test workspace green (63 suites, zero
   failures), two new golden snapshots (`gen__go__ping` regenerated for
   the M1→M2 template churn, `gen__go__sqlite-notes` new) reviewed and
   accepted. `docs/targets.json` regenerated (Go's description text
   changed to "1.24+", matching the corrected floor M1 already
   recorded).

3. **M3 — Broker, workers, jobs, channels.** nats.go + franz-go,
   retry/`HandleMessageOnce` seam, robfig jobs (untranslated
   schedules, unit-tested against the same cron-equivalence cases
   the Rust translation carries), WS/SSE channels, publish
   traceparent headers. The four broker/schedule examples verify.

   **Shipped (v0.24 M3):** `supports()` gained `Component::Queue`,
   `Component::Stream`, `Component::Worker`, `Component::Job`,
   `Component::Channel`, `Component::Scheduler`, `Component::Realtime`
   — one wide gate, same "engine-agnostic component, per-engine branch
   stays inside the template" shape M2 already established for
   `Database`. `events <Name>;` needed no separate gate: it lowers to
   the same `Component::Worker` node a plain `worker` declaration
   does, split into `ConsumerCtx` only at the codegen model layer.

   New templates: `internal/queue/queue.go` (broker client),
   `internal/services/<module>.go` seeded stubs for classic
   (`signature: None`) handlers not tied to a `crud` (`ctx.services`
   — genuinely new at M3; no earlier example had a bare handler with
   capability bindings and no `crud`), `internal/workers/<name>.go`
   (workers), `<jobs>.go` (robfig/cron jobs), `<consumer>.go` (events
   consumers), `internal/routes/channel_<name>.go` (WS/SSE),
   `cmd/workers/main.go`, and a shared `_steps.go.j2` macro
   ({% import %}-ed by `route_api.go.j2`/`worker.go.j2`/`job.go.j2`,
   mirroring TS's own single-macro choice over Python/Rust's
   per-file duplication).

   **Traceparent headers are not shipped this milestone** — Go
   doesn't support `Component::Tracing` yet (lands at M7, same as
   TS), so no reachable example has anything to propagate; the plan
   text's "publish traceparent headers" is deferred to M7 alongside
   tracing itself, not silently dropped.

   **A genuine per-engine divergence from Rust, found live, not
   assumed:** Rust's `queue.rs.j2` constructs its Kafka producer
   *eagerly* in `Queue::new` (rdkafka's `create()` neither dials nor
   spawns background work) while keeping NATS lazy behind a
   `OnceCell`. Go's franz-go looked like it should follow the same
   split — until a `goleak`-backed scratch test against a real,
   unreachable broker address showed `kgo.NewClient` starting two
   supervisory goroutines (`updateMetadataLoop`, `reapConnectionsLoop`)
   immediately on construction, with no error and no blocking dial.
   That means an eager Kafka producer would fail Go's own M1 no-infra
   construction test the moment a program declared `queue Kafka;` — so
   Go's `Queue` connects *both* engines lazily (mutex-guarded,
   connect-on-first-use), a real, disclosed difference from Rust's
   design rather than the "same as Rust" default assumption would have
   produced. `Queue.Consumer(subject, group)` (Kafka) still builds a
   fresh client per call, matching Rust's own per-call `consumer()` —
   a Kafka consumer group is bound at client construction, so it can't
   reuse the shared lazy producer client the way a NATS subscription
   reuses the shared connection.

   **A second live find on the same seam:** the first version of
   `Queue.Publish` (Kafka) called `cl.ProduceSync(context.Background(),
   ...)` — against a real, unreachable broker this **hung the HTTP
   request past 60 seconds** rather than failing, because franz-go
   retries internally forever against a `context.Background()` with
   no deadline. Fixed with a 5s `context.WithTimeout`, matching Rust's
   own `message.timeout.ms=5000` — found by actually curling a running
   binary with `--max-time`, not by reading the franz-go source.

   **Pipeline step emission (`_steps.go.j2`) needed a real design
   decision Rust/TS's own macros didn't have to make:** Go has no
   `?`/exception propagation, and both obvious shapes for a shared
   `result`/`err` pair across steps break — `result, err := f()` needs
   `:=` for `err`'s first declaration but `=` for every later step
   (which step is "first" depends on which kinds are present in which
   order), and pre-declaring `var err error` up front is a compile
   error ("declared and not used") on a publish-only pipeline (nothing
   ever reads it) — both shapes are reachable by M3's own examples
   (`kafka-pipeline.ciac`'s `api Ingest` and `realtime-progress.ciac`'s
   `api Complete` are both single-`publish`-step pipelines). Resolved
   by giving every step its own block-scoped `if v, err := ...; err !=
   nil { <on_err> } else { result = v }` (fresh names, fresh scope,
   every time) instead of a shared mutable pair — step order and
   composition stop mattering entirely. `call`/`match` step kinds
   render plausible code (mirroring Rust/TS's own M3-era templates,
   which did the same before their `call` infrastructure existed) but
   are inert until M6/M7 populate `call_imports`/match arms for real.

   **Three more real bugs, all found only by live-generating and
   building the four target examples, not by inspection:**
   1. `schemas.go.j2` never emitted an enum *type* at all —
      `go_type`/`go_zero` already returned `VideoStatus`/
      `VideoStatus("")` for an enum field (wired at M1 for future use),
      but nothing ever generated `type VideoStatus string` or its
      constants, so `realtime-progress.ciac` (the first example with
      an inline-enum record field) failed with `undefined: VideoStatus`
      at `go build`. Fixed by emitting the named-string type + typed
      constants + a `Valid()` membership method per `record.enums`,
      and wiring a presence-checked `Decode<Record>` to call it after
      unmarshal — Go's `encoding/json` accepts *any* string into a
      named-string field with no error, so without this an invalid
      enum value would have silently round-tripped instead of
      rejecting with the 400 every other target gives. The first
      version of `Valid()` also had a real logic bug (not just a
      compile error): `case A: case B: return true` only returns true
      for the *last* case in the list — Go's `switch` doesn't fall
      through by default — caught by `go vet`'s "missing return"
      diagnostic, fixed with one comma-joined case line.
   2. `main.go.j2` unconditionally imported and called
      `routes.Router(st)`, but `scheduled-cleanup.ciac` (M3's first
      jobs-only example — zero apis, zero resources, zero channels)
      has no `internal/routes` package at all, since `router.go.j2`
      is gated on all three being non-empty. Fixed with a `has_routes`
      boolean threaded from `lib.rs`, falling back to a bare
      `http.NewServeMux()` with just `/health` when nothing else
      declares a route — `/openapi.json` still always serves.
      Separately, `cmd/workers/main.go.j2`'s `errgroup.Wait()` returns
      immediately for a jobs-only program (nothing was ever added to
      the group via `g.Go`), which would have exited the process right
      after starting the cron scheduler; fixed with a trailing
      `select {}` gated on `c.jobs` being non-empty.
   3. `emit_service`'s per-service `openapi.json` write was gated
      behind `!multi`, silently dropped for every service inside a
      multi-service system. Invisible until M3's own wider `supports()`
      gate first made a multi-service example (`audited-crud.ciac`)
      reachable for Go at all — caught by the C3 conformance test's
      byte-identical-path-set check (`python: [accounts/openapi.json,
      catalog/openapi.json, openapi.json]` vs `go: [openapi.json]`),
      confirmed against Rust's own `lib.rs` (which never gates this
      write behind `!multi` either) before fixing. A second,
      adjacent gap on the same path: `generate()` still called
      `env.get_template("system-README.md.j2")` for multi-service
      systems, a template this crate never created (a latent M1 gap,
      also invisible until a multi-service example became reachable).
      Fixed by *not* emitting a system-level README at all, matching
      `ciac-backend-ts`'s own settled answer to this exact question
      (`ciac-backend-ts/src/lib.rs` computes the model value and
      discards it) rather than writing a template neither Python nor
      Rust's own users specifically need duplicated for Go.

   **Live-verified end to end, not just golden-generated,** all four
   target examples (`gofmt -l`/`CGO_ENABLED=0 go build`/`go vet`/
   `go test` clean throughout) plus the newly-reachable multi-service
   `audited-crud.ciac`:
   - `kafka-pipeline.ciac`: `POST /clicks` with a non-UUIDv4 `id` →
     400 from the `validator` tag; a real UUIDv4 with no Kafka broker
     running → 500 after the bounded 5s timeout (the fix above),
     server stays alive and `/health` still responds afterward.
   - `realtime-progress.ciac`: `POST /complete` with `"status":
     "Bogus"` → 400 `field "status": invalid VideoStatus value
     "Bogus"` (the enum fix above, live); a valid enum value with no
     NATS broker running → fast 500, not a hang; `GET
     /channels/progress` (the actual registered path — `/progress`
     alone 404s, confirming `channel.path`'s real shape) with
     WebSocket upgrade headers → 101 Switching Protocols via
     gorilla/websocket; without them → 400, matching the SSE/WS
     boundary every other target's channel gives.
   - `scheduled-cleanup.ciac` (jobs-only): `cmd/workers` starts, logs
     no schedule-parse error (`robfig/cron`'s standard 5-field parser
     accepts CIaC's `"0 3 * * *"` directly — no leading-seconds-field
     or 0-7-weekday translation needed at all, a genuinely simpler
     answer than Rust's own `cron` crate needed), and stays running
     (`select {}` fix above) rather than exiting immediately; a
     throwaway test calling `HandleTickCleanupOnce` directly against a
     real local Postgres passes clean — the same "seam a future
     simulation runner drives directly" contract Pillar 5 specifies.
   - `event-pipeline.ciac`: **full live broker round-trip**, not just
     construction — `go install`ed a real `nats-server` binary from
     the module proxy (no Docker needed, mirroring how MariaDB/Postgres
     get apt-installed for other milestones' live proofs) and ran it
     locally; `POST /submit` → `Validate` handler → publish to
     `ingest.events` over the real NATS connection → `worker Processor`
     consumes it → `Enrich` handler observably receives the payload
     (confirmed via a temporary log line in the generated, not
     template, file) — the actual HTTP→publish→consume chain working
     end to end, the strongest proof this milestone has for the
     broker plumbing being real rather than merely compiling.
   - `audited-crud.ciac` (multi-service, newly reachable): both
     `accounts/` and `catalog/` build/vet/test clean independently;
     `docker-compose.yml` and the root `openapi.json` index plus each
     service's own `openapi.json` all present and correctly shaped.

   Full verification: `cargo fmt`/`clippy -D warnings`/`cargo test
   --workspace` all green (63 suites, zero failures) after every fix
   above; five unit tests in `ciac-backend-go`'s own `tests` module
   (two new: `supports_broker_workers_jobs_channels_at_m3`,
   `generates_worker_pipeline_file_set`); seven golden snapshots
   updated/added (`ping`/`sqlite-notes` regenerated for the go.mod/
   go.sum dependency-set expansion; `kafka-pipeline`/`realtime-
   progress`/`scheduled-cleanup`/`event-pipeline`/`audited-crud` new),
   each reviewed before accepting, not blanket-accepted.
4. **M4 — Typed handlers: `HostSyntax` for Go.** FIRST the
   error-idiom contract amendment lands against existing targets'
   goldens (byte-identical proof + identity-syntax golden update) —
   or is confirmed already-generalized from the TS arc; THEN the Go
   leaves per Pillars 2/4: all verbs, real `sql.Tx` transactions,
   `jsonx`, builtins (`uuid.NewString()`, `time.Now().UTC()`), enum
   switches. typed-handlers/typed-video/domain-orders/query-verbs/
   extras-verbs verify; equivalence test → four targets (division,
   Json-indexing, Option-decode cases included).

   **Shipped so far (v0.24 M4, part 1 — the amendment itself):**
   confirmed, by reading `ciac-backend-ts/src/lower.rs`, that TS is
   **not** already-generalized: TS is `Orientation::Statement` (like
   Python) but propagates verb-call failure through JS exceptions,
   exactly like Python does — neither target has ever needed an
   explicit per-call error check, so the "or is confirmed already-
   generalized from the TS arc" branch does not apply; the amendment
   had to be designed and landed for real.
   
   The actual gap, found by tracing `lower_tail`'s own dispatch
   table: `db.insert`/`update`/`delete`/`query`/`count`/`delete_where`
   already have `Dest`+`indent`(+`in_tx`)-aware `_tail` leaf methods
   for `Statement` orientation — but `db.get`, `cache.*`,
   `object_store.*`, `email.send`, `search.*`, and `http.call` (the
   "every one of these already fits a single expression" leaves) fall
   through `lower_tail`'s generic `_ =>` arm: compute the plain scalar
   value via the existing leaf, then `apply_dest` it in one line. That
   one line is exactly what Python's exception model and TS's
   exception model can get away with and Go cannot — a fallible
   Go call needs its *own* `if err != nil { ... }` statement before
   its result is usable at all, with no expression-position analog of
   Rust's inline `?`.
   
   Added 12 new default-implemented `HostSyntax` methods
   (`db_get_tail`, `cache_get_tail`/`cache_set_tail`/
   `cache_delete_tail`, `object_store_put_tail`/`_get_tail`/
   `_delete_tail`/`_list_tail`, `email_send_tail`, `search_index_tail`/
   `search_query_tail`, `http_call_tail`) plus 12 new match arms in
   `lower_tail` routing each verb kind to its new method instead of
   the generic fallback. Each default implementation is defined to
   perform the *exact same computation* the old fallback arm did
   (call the existing scalar leaf, then `apply_dest`) — not a
   `Statement`-only `unimplemented!()` default like `db_insert_tail`'s
   sibling methods get, deliberately, so Python/TS need **zero** code
   changes and produce **zero** output difference; only a host that
   actually overrides one of the twelve (Go) changes shape. This
   makes "byte-identical for existing targets" provable by
   construction, not just by snapshot diff.
   
   **The identity-syntax golden update the plan calls for turned out
   to be an *absence* of one, and that absence is itself the proof:**
   `tests/tests/host_syntax_identity.rs` already runs
   `IdentitySyntaxStatement` against every M4 target example
   (`typed-handlers`, `domain-orders`, `query-verbs`, `extras-verbs`
   all have their own `host_syntax_identity__identity_stmt__*`
   snapshots checked in, `extras-verbs` alone covering all seven of
   the newly-routed verb kinds across its seven handlers) — after
   landing the amendment, `cargo test --workspace` (63 suites) passed
   with **zero** `.snap.new` files and zero snapshot diffs anywhere,
   Python/Rust/TS included. `apply_dest`'s own signature gained
   `H: HostSyntax + ?Sized` (the one non-additive shared-crate edit,
   needed so the new default methods can call it on `&self` inside
   the trait itself without forcing `Self: Sized` onto every
   `HostSyntax` implementor). Full verification: fmt/clippy/test
   green with this change alone, committed and pushed *before* any Go
   `lower.rs` leaf — the one intra-milestone ordering
   `24UpdatePlan.md`'s own "Milestone dependencies and parallelism"
   section hard-codes.

   **Shipped (v0.24 M4, part 2 — the Go leaves themselves):** a full
   `GoSyntax: HostSyntax` implementation in `ciac-backend-go/src/
   lower.rs` (~1150 lines), wired into `lib.rs` (`supports()` widened
   from `Component::Service { signature: None, .. }` to
   `Component::Service { .. }`; typed handlers render through
   `lower::render` into `internal/logic/<module>.go` (inline,
   compiler-owned) or `internal/services/<module>.go` (`extern`,
   seeded) — plain functions, not the struct+constructor shape classic
   handlers get, since a typed handler is stateless and its real,
   declared signature never needs `any`), a new `logic.go.j2`
   template, and an `Error() string` method added to `schemas.go.j2`
   for `record.is_error` types (needed so `fail`'s `&schemas.X{..}`
   satisfies Go's `error` interface — no example had exercised an
   error-kind record on Go before this milestone).

   Real `sql.Tx` transaction atomicity (Pillar 4's own commitment,
   exceeding Rust's disclosed non-atomic gap) is proven live, not just
   asserted: generated `domain-orders.ciac` against a real local
   Postgres, POSTed a negative-total order through `PlaceOrder`'s
   `transaction { db.insert(Orders, ..); fail InvalidOrder(..); db
   .insert(OrderAudits, ..); }`, and confirmed *both* inserts were
   rolled back (`SELECT` on both tables after the 500 response: zero
   rows) — then POSTed a valid order and confirmed *both* commits
   landed, plus a follow-on `LineItem` insert exercising the `Reference
   <Order>` FK. `query-verbs.ciac` (SQLite, zero Docker) got the same
   treatment: seeded rows directly via the sqlite driver (no HTTP
   insert path exists in that example — by design, it only exercises
   `db.query`/`count`/`update`/`delete`/`delete_where`), then drove
   every one of its five endpoints over real HTTP and confirmed the
   list/count/update/delete/delete_where results by hand against the
   underlying rows. Both generated trees also pass `go build`/`go vet`/
   `gofmt -l`/`go test` clean.

   Deviation from the plan's literal text: **the domain-orders rollback
   proof ran against real local Postgres, not "local sqlite"** — the
   plan's own M4 exit-checklist line says "local sqlite," but `domain-
   orders.ciac` itself declares `use { db Postgres; }` (unchanged since
   v0.16), and Postgres was reachable locally in this sandbox
   (`apt`-installed, matching v0.13 M1's own MariaDB precedent), so the
   live proof ran against the example's actual declared engine rather
   than substituting a different one to match the checklist's wording
   — a strictly stronger proof than the text asked for, recorded here
   rather than silently reconciled.

   Five real bugs were found and fixed by generating actual `.ciac`
   examples and reading/running the output, not by reasoning about the
   code in the abstract — every one of them would have shipped silently
   otherwise, since none was caught by `cargo check`/`clippy` (all are
   runtime-shaped: malformed Go source, wrong Go types, or a missing
   import), only by real `gofmt`/`go vet`/`go build` against generated
   trees:
   1. **Nested-quote panic message.** `HostSyntax::index`'s `StrKey`
      arm built its Go panic message with `format!("panic(\"KeyError:
      {s:?}\")")` — `{s:?}` (Rust's `Debug` for `&str`) already adds
      its own quotes, so the result was `panic("KeyError: "label"")`,
      invalid Go (`gofmt` rejected it: "missing ',' in argument list").
      Fixed by using a plain `{s}` interpolation for the *message*
      text (still `{s:?}` for the map*-key* position, which correctly
      needs a real quoted Go string literal) — found generating a
      `Json`-indexing handler for the cross-target equivalence test.
   2. **`db.update`'s `Option<Record>` result produced untyped `nil`.**
      The original `db_update_tail` applied `dest` separately inside
      each branch of an *internal* found/not-found `if`/`else` (`db
      .update`'s own HIR type is `Option<Record>`), passing bare
      `"nil"` to `Dest::Discard`/`Dest::Assign` — `_ = nil` and `name
      := nil` are both "use of untyped nil" compile errors, and even
      when typed, applying `dest` per-branch `:=`-declares a
      `Dest::Assign` name block-scoped to whichever branch ran (`
      collect_branching_lets` never sees this internal branch, since
      it only hoists *source-level* `if`/`match`, so nothing hoists a
      `var` for it either) — undefined immediately after the `if/else`
      closes. Rewritten as one `func() (*T, error) { .. }()` closure
      (matching `db_get`'s own already-correct shape) routed through
      `fallible_tail`, so there's exactly one `apply_dest` call and the
      closure's own return type gives the value real typing before
      `apply_dest` ever sees it. Found generating `query-verbs.ciac`'s
      `Replace` handler and reading the output.
   3. **`HirType::Json` disagreed with `FieldTypeKind::Json`.** `go_type
      (HirType::Json)` mapped to `any`, but the record-*field*-level
      mapping (`filters::go_type_of`, standing since M1/M2) already
      spelled a `Json` field `json.RawMessage` — every other target
      keeps one representation for both (Rust: `serde_json::Value`
      both places; Python: `dict[str, Any]`; TS: `unknown`); Go was the
      one inconsistent target. Surfaced as `go vet`: "(p.Extra) ...
      is not an interface" (a type assertion on a concrete `[]byte`-
      backed type, not an interface). Fixed by changing `HirType::Json`
      to also map to `json.RawMessage`, and rewriting `HostSyntax::
      index` to `json.Unmarshal` into a `map[string]any` first rather
      than type-asserting `any` directly (plus matching fixes to
      `object_store_put`'s Json-vs-Record payload branch, unexercised
      this milestone but now consistent). Found in the same
      `Json`-indexing example as bug 1, one layer deeper.
   4. **`table`-declared `Json` fields never imported `encoding/json`.**
      `models.go.j2`'s `needs_json` flag (standing since M2) only
      considered `ctx.resources`, never `ctx.tables` — no M1-M3 example
      ever put a `Json` field on a bare `table`, so this was a latent,
      un-exercised gap until M4's `DIVISION_EXAMPLE`-shaped test became
      the first. Fixed by also checking `ctx.tables`' own fields for
      `f.is_json`. Found as `go vet: undefined: json` in `internal/
      models/models.go` immediately after fixing bug 3.
   5. **A typed-handler pipeline step inside a `worker` asserted an
      empty type.** `_steps.go.j2`'s `handler`-step macro arm needed a
      concrete Go type to assert the pipeline's `any`-typed `result`
      back to before calling a typed handler's real, typed signature;
      `route_api.go.j2` already had this (`api.payload.class_name`),
      but `worker.go.j2` passed a bare `""`, producing the literal
      invalid Go expression `result.()`. Not a hypothetical: `examples/
      sim-vertical-slice.ciac`'s `ProcessOrder` worker pipeline
      genuinely calls a typed handler (`HandleOrderCreated`), and once
      `supports()` widened to accept typed handlers broadly, this
      example became reachable by Go for the first time and the
      conformance suite (`c3`/`c4a`/`c4b`) caught it immediately. Fixed
      two ways at once: `worker.go.j2` now computes `payload_type` from
      `worker.payload.class_name` exactly like `route_api.go.j2` does,
      *and* the message-decode sites (both NATS and Kafka) now decode
      directly into the concrete `schemas.<Type>` (assigned into a
      still-`any`-typed `payload` variable, so every existing call-site
      signature is unchanged) instead of a bare `json.Unmarshal(..,
      &payload)` into `any` — decoding into `any` alone would have
      produced a `map[string]any` at runtime, which the type assertion
      would then panic on even with the right type name spelled out.
      `HandlerRef` (shared `ciac-codegen::model`, used by every
      backend) gained one new field, `is_typed_handler: bool`, so
      `_steps.go.j2` can tell a typed-handler step from a classic one
      at the template layer without re-deriving it; `docs/protocol-
      schema.json` regenerated to match (its own checked-in-schema
      test caught the drift immediately).

   The M4-scope conformance/golden/equivalence suites are green with
   all five fixes in: `cargo test --workspace` (65 suites, two new
   since part 1 — `typed_handler_equivalence`'s Go extension, and the
   protocol-schema regen) passes with zero failures; `c3`/`c4a`/`c4b`
   (conformance.rs) pass, meaning Go now participates correctly in
   every conformance example it's `supports()`-eligible for, not just
   the two originally-scoped proving examples; five new/updated golden
   snapshots accepted (`domain-orders`, `kafka-pipeline` — a real,
   deliberate improvement: `Enrich`'s worker now decodes its Kafka
   message into a concrete `schemas.Click` instead of generic `any`,
   `modular-video`, `query-verbs`, `sim-vertical-slice`); `typed_handler
   _equivalence.rs`'s `DIVISION_EXAMPLE` test extended from 3 targets
   to 4, Go's own two divergences pinned by assertion (native `/`
   truncation, no `Math.trunc`-style special case needed at all; `Json`
   indexing's `json.Unmarshal`-then-panic shape). `typed-handlers.ciac`/
   `typed-video.ciac`/`extras-verbs.ciac` remain `CIAC0011`-refused
   this milestone exactly as planned (`object_store`/`auth`/`cache`
   stay gated to M6/M7); every *reachable* example — including three
   (`kafka-pipeline`, `modular-video`, `sim-vertical-slice`) the plan's
   own M4 text didn't name, newly reachable only because `supports()`
   widened to `Component::Service { .. }` broadly rather than
   enumerating specific examples — verifies clean.

   `docs/protocol-schema.json` and `cargo insta`-accepted goldens
   committed alongside the code; fmt/clippy/full-workspace-test all
   green as the closing gate before commit, per the standing per-
   milestone discipline.
5. **M5 — CHECKPOINT.** Measured cost vs the factory model with TS
   actuals as baseline; conformance harness green across four
   targets (OpenAPI byte-equality ×4, topology equality,
   boundary-case decode suite). Go/no-go for the remainder and for
   25UpdatePlan.md's start; "pause and amend the factory" remains a
   valid outcome.

   **Shipped (v0.24 M5) — the measured cost table**, against
   `23UpdatePlan.md` M5's own TS actuals (M1–M4, the same milestone
   marker Go has just reached) as the primary baseline, plus Rust's/
   Python's mature full-arc figures for extra context exactly as TS's
   own table did:

   | | Rust (mature, full arc) | Python (mature, full arc) | TypeScript (M1–M4 actuals) | Go (M1–M4 actuals) |
   | --- | --- | --- | --- | --- |
   | `lower.rs` (leaves + `render`) | 607 | 932 | 1,098 | **1,296** |
   | `lib.rs` (emission wiring) | 559 | 429 | 501 | **710** |
   | `filters.rs` (neutral-field mapping) | n/a (folded into `lower.rs`) | n/a | 206 | **139** |
   | templates | ~2,800 (Python/Rust full-arc baseline) | ~2,800 | 5,608 across 28 files | **2,176 across 25 files** |
   | edits outside the crate | 1 (registry line) | 1 (registry line) | 1 (`commands.rs`) | **1** (`commands.rs`, identical single-line registration — verified via `git show 5ec685f --stat`) plus **2 disclosed, narrow shared-crate amendments (below)** |

   **`lower.rs`/`lib.rs`: a real but modest overrun, not TS's pattern.**
   Go's `lower.rs` is 18% over TS's M1–M4 figure (1,296 vs 1,098) and
   `lib.rs` is 42% over (710 vs 501) — both driven by the same
   concrete, disclosed cause: M4's error-idiom amendment itself (every
   fallible verb needs its own `if err != nil` block plus, for
   `db.update`, a full closure-wrapped `fallible_tail` call — no other
   target's `lower.rs` carries this shape, since Python/TS propagate
   failure through exceptions and Rust through `?`) and `lib.rs`'s
   typed-handler emission split (inline `internal/logic/` vs. seeded
   `internal/services/`, mirroring Rust/TS's own split, plus the
   `is_typed_handler`-aware payload-type threading `_steps.go.j2`
   needed once M4 widened `supports()`). `filters.rs` is smaller than
   TS's own (139 vs 206) since Go's `go_pascal`/`go_type_of`/
   `go_validate_tag` filter set is genuinely narrower than TS's own
   Zod/Drizzle-adjacent filter surface.

   **Templates: the clearest confirmation of TS's own M5 retro.** TS's
   M5 checkpoint (`23UpdatePlan.md`) explicitly flagged, as its own
   corrected-budget warning for Go: *"expect a `lower.rs` noticeably
   larger than Rust's 577-line figure whenever the target language's
   database ecosystem lacks a `sqlx`-equivalent unifying driver (true
   for Go's `database/sql` + per-engine driver split too — worth Go's
   own author checking this before, not after, writing its own
   `lower.rs`)."* Measured evidence says that specific worry doesn't
   transfer: Go's `database/sql` interface (`*sql.DB`/`*sql.Rows`/
   `sql.Result`, identical method sets across the `pgx`/`go-sql-
   driver/mysql`/`modernc.org/sqlite` drivers, only the connection
   string and placeholder style varying — v0.13 M1's own established
   `sqlph` discipline, reused unchanged since) genuinely *is* Go's own
   `sqlx`-equivalent unifying layer, unlike Node's three structurally
   different driver APIs (`pg`'s `[rows]` destructuring vs. `better-
   sqlite3`'s sync `.run()`/`.changes` vs. `mysql2`'s `affectedRows`)
   that drove TS's own per-engine template branching. The result: Go's
   templates land at 2,176 lines across 25 files (M1–M4 scope) against
   TS's 5,608 across 28 files at the *same* milestone marker — a real,
   measured confirmation that the unifying-driver factor TS's retro
   named is exactly the right lens, and that Go sits on the favorable
   side of it. (`route_api.go.j2`'s own single `net/http` `ServeMux`
   routing shape, with no framework-specific request/response wrapper
   the way Fastify's schema-validation plugin machinery needed on TS's
   side, is a second, smaller contributor to the same gap.)

   **Two disclosed, narrow shared-crate amendments — the honest
   accounting of "edits outside the crate" beyond the registry line,**
   both already fully documented in their own milestones' shipped
   notes and re-cited here for the checkpoint's own record: M4 part 1's
   error-idiom amendment (`ciac-codegen/src/lower/{dispatch,
   host_syntax}.rs`, 12 new default-implemented `HostSyntax` methods +
   12 new dispatch arms, proven byte-identical for Python/Rust/TS by
   the full `cargo test --workspace` snapshot suite passing with zero
   `.snap.new` files the moment it landed) and M4 part 2's
   `HandlerRef::is_typed_handler` field (`ciac-codegen/src/model.rs`,
   17 lines, additive, `docs/protocol-schema.json` regenerated to
   match). Both are the kind of "pause and amend the factory" this
   checkpoint's own text names as a valid outcome — exercised for
   real, narrowly, and disclosed, not silently absorbed. M2 and M3
   needed **zero** shared-crate edits (verified via `git show
   b40711f/eaf7432 --stat`), confirming the factory held without
   amendment for the large majority of the arc so far. The remaining
   files this arc has touched outside the crate — `Cargo.toml`/
   `Cargo.lock`/`tests/Cargo.toml` (workspace member registration),
   `tests/src/lib.rs` (the `backends()` list, TS's own equivalent
   registration point), `.github/workflows/ci.yml` (the per-target CI
   row, exactly what M1 added for every prior target), and `docs/
   protocol-schema.json`/`docs/targets.json` (mechanical, test-
   enforced regeneration, not hand-authored) — are routine scaffolding
   every target needs, not additional factory amendments.

   **Conformance harness, run for real across all four targets:**
   `cargo test --workspace` (65 suites) is green with Go included;
   `tests/tests/conformance.rs`'s `c3_openapi_is_byte_identical_across_
   targets`, `c4a_migration_sql_is_byte_identical_across_targets`, and
   `c4b_declared_topology_appears_in_every_target` all pass — Go's
   OpenAPI/migration-SQL/topology output is byte-identical to Python's/
   Rust's/TS's for every example all four targets support, catching
   real bugs this same arc (M4's five live-found-and-fixed bugs,
   documented in M4 part 2's own shipped notes, were largely surfaced
   by this exact suite once `supports()` widened). `tests/tests/
   golden.rs`'s three snapshot suites are green (25 `.go.j2` templates,
   5 example projects goldened this arc). The boundary-case decode
   suite this checkpoint's own text names is M2's own live-verified
   absent/explicit-null/legitimate-zero triple (`24UpdatePlan.md` M2's
   shipped notes: a missing field → 400, an explicit `null` → 400, a
   real empty-string zero value → 200/201) — already passing, not new
   work this milestone. `ciac targets --json` lists `"id": "go"` with
   its own `TargetInfo` (validate steps, `go.mod` marker, CI test
   steps) alongside python/rust/typescript, unchanged in shape since
   M1.

   **Go/no-go verdict: GO.** Every measured gap has a concrete,
   disclosed, ecosystem-shaped cause (the error-idiom amendment's own
   line cost, already justified in M4's own notes) or is a *favorable*
   surprise relative to TS's own corrected-budget warning (templates
   at 39% of TS's line count for the same milestone marker, `database/
   sql` genuinely delivering the unifying-driver benefit TS's retro
   hoped for but couldn't measure directly). Nothing measured here is
   a capability gap, a correctness gap, or a structural blocker: the
   factory's structural promise (`TargetInfo`, the backend-owned-filter
   pattern, the shared scanner, the shared `HostSyntax` dispatcher —
   *including* its own M4 amendment, itself proven additive — the
   conformance harness, `Emit`/skeleton) held for a fourth backend,
   and the one amendment it did need was narrow, justified, and
   provably non-breaking rather than a silent scope-creep. 24UpdatePlan
   .md's remaining milestones (M6–M9) may proceed without pausing to
   amend the factory further; 25UpdatePlan.md's own start should cite
   this table (not 22UpdatePlan.md Pillar 8's original estimate, nor
   TS's own M1–M4 actuals alone) as its starting budget, and should
   specifically expect a `lower.rs` premium of roughly TS's own
   percentage (Python/Rust/TS/Go's own error-propagation-idiom cost)
   whenever a fifth target's own idiom also lacks an expression-
   position propagation operator.
6. **M6 — Auth, scopes, scope tests.** golang-jwt + keyfunc,
   middleware, generated httptest suite green under zero
   infrastructure; order-system and oauth-echo verify.

   **Shipped (v0.24 M6):** `internal/auth/auth.go.j2` (new) —
   `VerifyToken` (`jwt.ParseWithClaims` against a static HS256 secret
   for JWT, a lazily-constructed `keyfunc.NewDefaultCtx` behind a
   `sync.Once` for OAuth2/RS256, mirroring every other lazy provider
   client — db pools, the broker client, TS's own `jwks ??= ...` — so
   `state.New` never makes a network call a route never needs) and
   `RequireScope` (space-joined `scope` string or `scp` array, both
   real-issuer conventions, matching every other target's own two-
   shape check). Errors flow through the existing `httpx.Unauthorized`
   /`httpx.Forbidden`/`httpx.WriteError` machinery — no separate auth
   error-mapping layer needed. `VerifyToken`/`RequireScope` are called
   inline at the top of each generated route handler body (matching
   every other target's own inline-in-route-body placement, not a
   middleware chain): `route_api.go.j2` gates on `api.has_auth_step`/
   `api.scope`; `resource_api.go.j2` gates each of its five handlers
   on `resource.has_auth` with `resource.read_scope`/`write_scope`
   threaded through as an explicit `cfg config.Config` parameter
   (below). `scope_test.go.j2` (new) mirrors TS's `scope.test.ts.j2`/
   Rust's `scope_tests.rs.j2` shape exactly — `httptest.NewRequest` +
   `mux.ServeHTTP` in place of `fastify.inject()`/`tower::ServiceExt::
   oneshot`, `jwt.NewWithClaims(...).SignedString(...)` to mint tokens
   locally, the same dummy-value macros keyed off `FieldTypeKind`'s
   internally-tagged shape ported directly from TS's own — gated
   identically: `ctx.auth_scheme == "jwt" && !ctx.scopes.is_empty()`,
   so a JWT service with no declared scope gets `auth.go` but no
   `scope_test.go`, and an OAuth2 service never gets one regardless of
   scope count (the same disclosed reason as every other target: real
   RS256 verification needs a real issuer's JWKS regardless of how
   lazily it's fetched, so a true no-infrastructure OAuth2 scope proof
   needs a fake auth adapter, future work this milestone didn't
   build). Real dependency versions (`github.com/golang-jwt/jwt/v5
   v5.3.1`, `github.com/MicahParks/keyfunc/v3 v3.8.0`, plus transitive
   `github.com/MicahParks/jwkset v0.11.0` and `golang.org/x/time
   v0.9.0`) were obtained from a scratch Go module exercising both
   packages' real API surface and `go mod tidy`, not guessed — avoiding
   a `go.sum` hash mismatch `GOFLAGS=-mod=readonly` would reject at
   `go test` time; `go.mod.j2`/`go.sum.pin` (the single checked-in
   verbatim pin file copied byte-for-byte into every generated Go
   project since M1) now carry all four unconditionally, so every Go
   golden's `go.mod`/`go.sum` churned this milestone regardless of
   whether that particular service declares `auth` — expected,
   consistent with the pin file's own established discipline, not a
   bug.

   Two real bugs found live, both via generating and building against
   the real `go` toolchain (not hypothesized):
   1. `auth.go.j2` unconditionally imported `context`, `sync`, and
      `github.com/MicahParks/keyfunc/v3` even on JWT-only (HS256)
      services that never reference them — `go vet`/`go build` failed
      with `"context" imported and not used` (and the same for `sync`
      and `keyfunc`) the moment a JWT-only example was generated.
      Fixed by wrapping all three imports in
      `{%- if c.auth_scheme == "oauth2" %}` conditionals, the same
      fix TS's own M6 needed for `createRemoteJWKSet` from `jose`.
   2. The first `resource_api.go.j2` draft called
      `auth.VerifyToken(r, store.Config())`, but
      `services.{StoreClass}` (defined in `resource_store.go.j2`)
      only holds a `db *sql.DB` field — no `Config`/`Config()`
      accessor exists, which would have been a compile error on any
      auth-gated CRUD resource. Fixed by threading `cfg
      config.Config` as an explicit extra parameter to each of the
      five handler-factory functions (`create{{ resource.name
      }}(store, cfg)` etc.), captured once as `cfg := st.Config` in
      `Register{{ resource.name }}Routes` and only when
      `resource.has_auth` — avoiding polluting the store struct with
      an unused field for the common non-auth case.

   **Disclosed scope gap vs. the plan's literal text:** `order-system
   .ciac`, this milestone's own named verification target, also
   declares `cache Redis` (`Component::Cache`), which stays refused
   for Go until M7 — unlike TS, where Cache landed at TS's own M2, so
   TS's M6 could reach order-system directly. order-system therefore
   still returns `CIAC0011` for Go this milestone; it will verify for
   real once M7 lands Cache, as M7's own milestone text already names
   it. In its place: `oauth-echo.ciac` (the plan's other named
   example, cache-free, OAuth2-only) verifies exactly as the plan's
   own "oauth-echo verifies statically" language describes, and a new
   minimal scratch example (not committed — `jwt-scope.ciac`,
   `db Postgres; auth JWT;` plus a scope-gated `crud` and a
   scope-gated `api`) supplied the live JWT+scope proof order-system
   would otherwise have provided. Among existing examples, only
   `oauth-echo` newly un-gates for Go this milestone (its golden is
   new, not modified) — `crud-notes`, `routed-media`, `typed-video`,
   and `video-platform` all also need `cache Redis` (deferred to M7);
   `dev-identity` needs `Component::Users`/Keycloak (also deferred to
   M7, per M7's own milestone text). This is a smaller immediate
   un-gating than TS's own M6 (which gained six goldens at once,
   `Cache` already being supported by then) — a real, disclosed
   consequence of Go's own milestone ordering deferring Cache to M7
   rather than M2.

   Live proof: `jwt-scope.ciac` generated via the real `ciac build
   --target go` binary, `gofmt -l .` clean, `go vet ./...` clean,
   `go build ./...` clean, `go test ./... -v` green — the generated
   `TestSecureScopeEnforced`, `TestWidgetReadScopeEnforced`, and
   `TestWidgetWriteScopeEnforced` subtests all pass: a token missing
   the required scope gets a real `403` before any Postgres call is
   touched (asserted directly), and a token carrying the granted scope
   clears the scope check and reaches the (expected, no-infrastructure)
   failure from the unreachable local Postgres — never a `401`/`403`
   — proving the check itself, exactly the "zero infrastructure" claim
   boundary every other target's own scope test documents. `oauth-echo
   .ciac` separately verified `gofmt`/`go vet`/`go build`/`go test`
   clean, with its generated `internal/auth/auth.go` reviewed by hand
   to confirm the RS256/JWKS code path (lazy `keyfunc.NewDefaultCtx`,
   `jwt.WithValidMethods([]string{"RS256"})`,
   `jwt.WithIssuer`/`jwt.WithAudience`) is structurally correct.

   Full workspace verification: `cargo fmt --all --check` clean,
   `cargo clippy --workspace --all-targets -- -D warnings` zero
   warnings (19.19s), `cargo test --workspace` green (65 suites, one
   `cargo insta test --accept` pass to regenerate the 11 Go goldens
   that churned from the go.mod/go.sum dependency additions plus the
   stale "not wired yet" doc-comment removal in `resource_*_api.go`,
   and the one genuinely new `oauth-echo` golden — all diffs reviewed
   by hand before accepting, confirming dependency/comment churn only,
   no unexpected behavioral change). Unlike TS's own M6, no stale
   gating-test assertion needed updating (`ciac-backend-go`'s M3-era
   test suite never asserted `Component::Auth` was refused, so no
   `supports_*` test rename was needed).

   **Unplanned mid-milestone incident:** `cargo build` failed with a
   linker `Bus error` (`ld terminated with signal 7`) after many
   rebuilds across this arc's session exhausted the sandbox's root
   filesystem — diagnosed via `df -h /` (13M free of 252G) and `du
   -sh` isolating `target/debug/deps` at 21G of orphaned build
   artifacts. Resolved with `cargo clean` (safe, rebuild-from-scratch,
   no data loss; 22.7GiB freed, ~35s to rebuild `ciac` afterward)
   plus `go clean -cache` and deleting several stale scratch-generated
   Go project directories; unrelated to the milestone's own code
   changes, noted for the arc's own record since it consumed real
   session time.
7. **M7 — Ontology remainder + call clients + observability
   completion.** S3/email/search/http wrappers, call clients, otel
   end-to-end (four-target trace test), metrics. multi-service-media,
   inventory-system, ontology-growth, traced-checkout, dev-identity
   verify. `--system` CI rows: go × inventory-system, × mysql-notes,
   × sim-vertical-slice — and, exploiting Go's build speed, go ×
   multi-service-media (the row Rust's time budget excluded),
   with the job comment updated to say why Go can afford it.

   **Shipped (v0.24 M7):** `supports()` moved to unconditional `true`
   — `Cache`, `ObjectStore`/`Email`/`Search`/`ExternalHttp`, `Users`,
   and `Logging`/`Metrics`/`Tracing` all reach this backend now, the
   same full-parity shape Rust's/TS's own `supports()` already have.
   `Cache` needed no dedicated wrapper package (a lazily-parsed
   `*redis.Client` field on `AppState`, wired straight into
   `resource_store.go.j2`'s new read-through-cache CRUD — cache-aside
   `Get` (cache hit returns immediately; miss falls through to the db
   and populates the cache), `Create`/`Update` refresh the cache entry,
   `Delete` evicts — mirroring Rust's/TS's own choice to keep it
   inline rather than its own module). `ObjectStore`/`Email`/`Search`/
   `ExternalHttp` each got one wrapper package per capability *kind*
   (not per named instance, matching Rust's own module shape):
   `internal/objectstore` (AWS SDK Go v2's `s3` client, path-style
   addressing so a local MinIO works with no extra config — the
   official SDK, not a hand-rolled SigV4 signer, since MinIO's S3 API
   compatibility is exactly what that SDK targets), `internal/email`
   (dependency-free `net/smtp`, matching Rust's `lettre`/TS's
   `nodemailer` shape but with no external module needed — Mailpit's
   dev SMTP needs neither TLS nor auth), `internal/search`
   (dependency-free `net/http` straight against OpenSearch's own REST
   API — `PUT /{index}/_doc/{id}`, `POST /{index}/_search` with the
   same `{"query": {"query_string": {"query": ..}}}` DSL body Rust's/
   Python's own `search.query` lowering already builds, `DELETE
   /{index}/_doc/{id}` — no official `opensearch-go` client needed for
   three verbs), `internal/httpclients` (dependency-free `net/http`,
   matching the M5-era typed call clients' own "no HTTP client
   dependency needed" precedent). `internal/clients` (one file per
   downstream `call`ed service, no `mod.rs`-equivalent aggregator
   needed — every `.go` file under one directory sharing `package
   clients` forms the package automatically) implements the typed
   HTTP call clients `_steps.go.j2`'s own `call` step arm had already
   been written, unreachable, since v0.24 M3 (`clients.New{ClassName}
   (st).{Method}(ctx, result)`) — envelope-unwrapping (`{"data": ..}`)
   matches Rust's/Python's/TS's own client shape exactly.

   **Observability, in full:** `internal/observability` gained
   `initTracing()` (`go.opentelemetry.io/otel/sdk` + `otlptracegrpc`,
   reading `OTEL_EXPORTER_OTLP_ENDPOINT` — default `http://localhost
   :4317`, the same collector port every other target's own exporter
   already targets), a process-wide `Tracer`, `Traced(ctx, name,
   carrier, fn)` (the consume-side half of broker-hop propagation —
   extracts an inbound W3C `traceparent` carrier, starts a span,
   nil-safe for a message published by a non-participating system),
   and `InjectCarrier(ctx)` (the publish-side half, mirroring Rust's
   `inject_trace_carrier`/TS's own `propagation.inject` call).
   Inbound HTTP server spans and outbound call-client spans both come
   "for free" via `go.opentelemetry.io/contrib/instrumentation/net/
   http/otelhttp` (`otelhttp.NewHandler` wraps the final mux in
   `main.go.j2`, after every route is already registered — keeping
   `routes.Router`'s own `*http.ServeMux` return type unchanged, since
   `scope_test.go.j2` calls `mux.ServeHTTP` directly), the same
   "instrumentation gives it for free" precedent TS's own
   `UndiciInstrumentation`/Python's `HTTPXClientInstrumentor` already
   set — `client.go.j2`'s own call-client methods use the same
   `propagation.HeaderCarrier` injection pattern directly rather than
   an instrumented transport, since they already had to hand-build the
   request. `queue.go.j2`'s `Publish` (both engines) gained a
   `headers map[string]string` parameter — NATS via a real
   `nats.Msg`/`nats.Header`, Kafka via `[]kgo.RecordHeader` — and
   `PublishJSON` now takes `ctx` as its first argument (threaded
   through both call sites: `_steps.go.j2`'s `publish` step and
   `lower.rs`'s own `HostSyntax::publish` leaf) to inject the active
   span before handing off to the broker. `worker.go.j2`'s own consume
   loops (both engines) extract the inbound carrier from the message's
   own headers and wrap the handler call in `observability.Traced
   ("worker.<name>", ..)` — the same span-naming convention Rust's own
   `tracing::info_span!("worker.{name}")` already established.
   `Logging`/`Metrics` needed no new wiring beyond what M1's own
   `observability.go.j2` doc comment already promised: `slog`'s JSON
   handler swap was already conditional on `has_logging`; `Metrics`
   adds `prometheus/client_golang`'s default Go/process collectors
   plus a `GET /metrics` route (`promhttp.Handler()`), mounted in
   `main.go.j2` alongside `/openapi.json`.

   **Five real bugs found live**, all via generating and building
   against the real `go` toolchain (not hypothesized) — the same
   "leaves authored ahead of their gate, exercised for the first time
   this milestone" pattern TS's own M7 found four of:
   1. `object_store.put`'s old leaf built a
      `func() ([]byte, error) { return json.Marshal(value) }()`
      sub-expression for the `Record` payload branch and passed it as
      `Put`'s *third* argument — a genuine `go vet`/`go build` failure
      ("multiple-value ... in single-value context"), since a
      multi-return call may only be a bare argument when it is the
      *sole* argument. Fixed by reusing `json_body` (the same single-
      valued Record/Json/scalar 3-way `search.index`/`http.call`
      already used) instead of a bespoke match, and giving
      `ObjectStore.Put` a single `any` parameter that marshals
      internally — eliminating the bug and a needed-but-wrong
      `needs.object_store_put` flag in `needs_json`'s own computation
      (which had forced an `encoding/json` import even on a scalar
      `object_store.put` call with no `json.` text in its own body) at
      the same time.
   2. `email_send`/`search_index`/`search_query`/`http_call`'s field
      resolution used the raw snake_case `AppState` field name
      directly (`st.email`) instead of `go_pascal`-converting it
      (`st.Email`) the way `cache_field`/`db_field` already correctly
      did — would have been an "undefined: email" compile error on
      any handler using these leaves. Found by re-reading `lower.rs`
      before writing the wrapper types it calls into, not by a build
      failure (the inconsistency was visible directly against the
      already-correct `cache_field`/`db_field` sites).
   3. `config.go.j2`'s `object_store`/`email`/`search`/`external_http`
      default-value lines were missing quotes around
      `{{ f.rust_default }}` (`envOr("S3_ENDPOINT",
      http://localhost:9000)` instead of `envOr("S3_ENDPOINT",
      "http://localhost:9000")`) — a `gofmt`-caught syntax error the
      moment `extras-verbs.ciac` (the first example to reach any of
      these four capabilities) was generated. Rust's/TS's own
      `config.rs.j2`/`config.ts.j2` already wrap `f.rust_default` in
      quotes; Go's own template, written scaffolding-ahead at an
      earlier milestone, simply never did.
   4. `route_api.go.j2`/`worker.go.j2`/`job.go.j2` never imported
      `internal/clients` on a `call` step, despite `_steps.go.j2`'s own
      `call` arm (written at v0.24 M3, unreachable until this
      milestone) already emitting `clients.New{ClassName}(st)` —
      "undefined: clients" the moment `traced-checkout.ciac` (this
      milestone's own named example) exercised a `call` step for
      real. Fixed by adding the conditional import, gated on the
      already-shared `api.call_imports`/`worker.call_imports`/
      `job.call_imports` (`Vec<CallCtx>`), to all three templates.
   5. `models.go.j2`'s `<Name>In` create/update payload struct used
      the bare field-level `go_type` filter for every field, which
      returns an *unqualified* enum type name (`VideoStatus`) —
      correct only *within* `schemas.go`, where every sibling enum is
      already in scope bare, but `models` is a different package.
      "undefined: VideoStatus" the moment `typed-video.ciac`/
      `routed-media.ciac` (both `CIAC0011`-refused for Go until this
      milestone's own `Cache` un-gating) first reached a `crud`
      resource with an enum field. Fixed with a `field.is_enum`-gated
      `schemas.` prefix (a new `needs_schemas` flag on `models.go.j2`'s
      own render context, computed in `lib.rs`) — which surfaced a
      second, related bug once the first was fixed:
      `resource_store.go.j2`'s `Create`/`Update` assigned
      `payload.Status` (now `schemas.VideoStatus`) directly into the
      row struct's own `Status string` field, which Go's assignability
      rules reject between two distinct named string types even though
      both have the same underlying type — fixed with an explicit
      `string(payload.Status)` conversion, gated on the same
      `field.is_enum`.

   **An unplanned, disclosed dependency-version discipline change:**
   rather than hand-merging each new dependency's scratch-derived
   `go.mod`/`go.sum` lines into the existing pin files (the v0.24 M6
   precedent), M7 built one comprehensive scratch Go module
   reproducing the *entire* real `go.mod.j2` dependency set (every
   pre-existing package pinned at its exact existing version via `go
   get pkg@version`, not `@latest`) plus every new package this
   milestone needed, then ran a single real `go mod tidy` and adopted
   its output verbatim as the new `go.mod.j2`/`go.sum.pin`. Found live
   why this mattered: a first attempt hand-merging just the new
   dependency lines missed that `go.opentelemetry.io/otel`'s own
   transitive graph requires *higher* minimum versions of
   `golang.org/x/net`/`x/sync`/`x/sys`/`x/text`/`x/crypto` than the
   pre-existing pin file had — a hand merge would have left `go.sum`
   missing hashes for versions the real build graph resolves to,
   a `go build -mod=readonly` failure the very first time any new
   dependency actually got exercised. The one-shot authoritative
   `go mod tidy` approach avoids this whole class of error and is
   recorded here as the discipline this arc's remaining milestones
   (and 25UpdatePlan.md's own Go-adjacent work, if any) should follow
   whenever a new dependency touches the shared transitive graph.
   Also found live: `go.opentelemetry.io/contrib/instrumentation/net/
   http/otelhttp`'s *latest* release (`v0.69.0`) requires `go >=
   1.25.0` and silently upgrades a `go 1.24.0` module's toolchain
   directive to `go1.25.12` the moment it's added — this sandbox only
   has `go1.24.7` installed, so an unpinned `@latest` would have
   broken every subsequent local build. Pinning `otelhttp@v0.63.0`
   (the contribution-repo release matching the `go.opentelemetry.io/
   otel@v1.38.0` core/SDK line already in use) keeps the toolchain at
   `go1.24.7`, unpinned from this arc's own release cadence.
   Separately, go-redis v9's newer releases ship a "maintenance
   notifications" feature (Redis Cloud/Enterprise cluster-maintenance
   push events, `MaintNotificationsConfig`, default `ModeAuto`) that
   spawns a background `CircuitBreakerManager` goroutine at
   `redis.NewClient` construction time regardless of target — caught
   live by `TestNewLeaksNoGoroutines` (the same no-infra-at-
   construction discipline that caught franz-go's own construction-
   time goroutines at v0.24 M3), fixed by explicitly setting
   `MaintNotificationsConfig: &maintnotifications.Config{Mode:
   maintnotifications.ModeDisabled}` before `redis.NewClient` — a
   plain local/dev Redis never sends these notifications, so there is
   nothing for this backend to react to.

   **Live proof:** every reachable ontology-exercising example
   generated via the real `ciac build --target go` binary and checked
   against the real toolchain (`gofmt -l .` clean, `go vet ./...`
   clean, `go build ./...` clean, `go test ./...` green, zero
   goroutine leaks): `extras-verbs` (all seven `cache`/`object_store`/
   `email`/`search`/`external_http` verbs, confirming every leaf
   resolves to its correctly-`go_pascal`-cased field), `crud-notes`/
   `dev-identity`/`video-platform` (regression check — cache-aside
   CRUD and OAuth2/`users Keycloak` unchanged by this milestone's own
   work), `typed-handlers`, `routed-media`/`typed-video` (the enum-
   field bug's own proving pair), `order-system` (this milestone's own
   named example, previously `CIAC0011`-refused for `Cache` since v0.24
   M6 — its full JWT-scope-enforcement suite, `go test ./internal/
   routes/... -v`, passes for real now: a wrong-scope token gets a
   real `403` before any Postgres call, a correctly-scoped token clears
   the check and reaches the expected no-infrastructure Postgres
   failure, never a `401`/`403`), `inventory-system` (2 services),
   `multi-service-media` (5 services), `traced-checkout` (2 services,
   `call`/`publish`/worker-consume trace propagation all present and
   type-correct — no live Jaeger/collector round-trip attempted, no
   Docker daemon in this sandbox, the same disclosure every other
   target's own OTel milestone already carries; "the trace test" this
   milestone proves is structural: real `go build` compilation of the
   full wiring, byte-for-byte header-contract parity with the other
   three targets' own publish/consume shape, confirmed directly against
   their generated `queue.*`/`worker.*` sources), `ontology-growth`.
   `dev-identity.ciac` (needs `Component::Users`) needed *zero*
   Go-specific code, confirming `ciac-codegen::model`'s own dev-issuer-
   default computation is already target-neutral — the same
   confirmation TS's own M7 recorded.

   **CI:** `go × inventory-system`, `go × mysql-notes`, `go ×
   sim-vertical-slice`, and `go × multi-service-media` rows added to
   `generated-system`'s compose-backed `--system` matrix (`actions/
   setup-go@v5` added to that job, previously Rust/Python/uv-only);
   the job's own comment records why Go's build speed affords the
   5-service system where Rust's own time budget excluded it (a Go
   compose build compiles in low single-digit seconds even uncached,
   with none of the crate-tree-recompile cost `cargo build` inside an
   uncached container carries).

   Full workspace verification: `cargo fmt --all --check` clean,
   `cargo clippy --workspace --all-targets -- -D warnings` clean,
   `cargo test --workspace` green (68 suites — `c3_openapi_is_byte_
   identical_across_targets`/`c4a_migration_sql_is_byte_identical_
   across_targets`/`c4b_declared_topology_appears_in_every_target` all
   still pass, confirming Go's newly-un-gated ontology output stays
   byte-identical to the other three targets everywhere all four now
   reach). Golden acceptance: 12 brand-new Go example trees
   (`extras-verbs`, `inventory-system`, `multi-service-media`,
   `mysql-notes`, `ontology-growth`, `order-system`, `routed-media`,
   `sim-broker-slice`, `traced-checkout`, `typed-handlers`,
   `typed-video`, `dev-identity`, `crud-notes`) plus every pre-existing
   Go golden churned (the three new dependency sets — AWS SDK v2,
   go-redis, OTel — land in the single shared `go.sum.pin` regardless
   of which capabilities any one example actually declares, the same
   established discipline v0.24 M6 already used for the JWT/JWKS
   dependency pair); all diffs reviewed by hand before accepting.
8. **M8 — Whole-repo integration.** Every example verifies or is
   reason-gated (target: zero gates); goldens complete; generated
   docs tables regenerate; `ciac dev` session test; MCP exercised;
   evolution/rename-replay against a Go tree; `generated-go` CI job
   (module-cached).

   **Shipped (v0.24 M8):** zero gates confirmed — `supports()`
   collapsed to unconditional `true` at M7 already, but this milestone
   verified it exhaustively rather than trusting the milestone
   narrative: a real sweep (`cargo run -p ciac -- build examples/*.ciac
   --target go`) over all 26 examples shows every one generates with
   no `CIAC0011`, then a second real sweep — `gofmt -l .`/`go vet ./...`
   /`go build ./...`/`go test ./...` against *every* `go.mod` directory
   this produces, not just the top-level project (33 directories once
   every multi-service system's per-service subdirectory is counted,
   matching TS's own M8 sweep shape exactly) — comes back completely
   clean, zero goroutine leaks anywhere. Golden suite: already complete
   as of M7 (all 26 examples have a Go golden; the 12 new trees M7's
   own un-gating produced cover every example that wasn't already
   golden); no template changes landed this milestone, so no golden
   churn either.

   Docs: `docs/language.md`'s provider table gained a Go column (all
   14 capability rows), matching v0.23 M8's own TypeScript-column
   precedent; `README.md`'s CIaC-concept table gained the same column,
   and its four `--target python\|rust\|typescript` CLI usage lines
   (which, found live, had never once been updated for Go despite Go
   reaching ping-parity all the way back at M1) became `python\|rust\|
   typescript\|go`.

   `ciac dev`: `dev_survives_compile_errors_and_regenerates_on_fix_go`
   added to `dev_cli.rs`, the same watch/break/fix session as the
   existing Python/TypeScript tests, against `-t go`. Confirms
   `--no-docker`'s early return never shells out to the Go toolchain
   either — the target-neutral watch/rebuild loop itself (unchanged
   since v0.13 M4) needed zero Go-specific code, the same factory
   promise TS's own M8 confirmed for its own target.

   MCP: `mcp_cli.rs`'s existing round-trip test gained real `build`
   and `verify` `tools/call` invocations against `target: go`
   (previously only reachable via the CLI, never exercised through
   the MCP surface) — `verify` runs the genuine `gofmt`/`go vet`/
   `go build`/`go test` sequence through MCP, confirming the same
   envelope shape `ciac verify --target go` prints on the command
   line. No `node_modules`-equivalent cleanup needed afterward: the Go
   module cache lives in `GOMODCACHE`, outside the generated project
   directory, unlike `node_modules`.

   Evolution/rename-replay: `rename_cli.rs` gained
   `out_replay_resolves_the_go_target_migrations_dir` — a
   `table`-bearing program built with `-t go`, confirming
   `backfill::migrations_dir` resolves through `GoBackend::
   target_info().migrations_dir` to `"migrations"` (matching Rust's/
   TS's own value, not Python's `"app/migrations"`) — the same real,
   not hypothetical, divergence-in-the-wild check TS's own M8 already
   established for its own target. The `--out` rename replay then
   confirms the migration file survives at its correctly-resolved
   path across the replayed regeneration, reading back
   `internal/schemas/schemas.go` (Go's own schema file, distinct from
   TS's `src/schemas.ts`) to confirm the rename itself landed.

   `generated-go` CI job: already existed since v0.24 M1 (with its own
   `CIAC0011`-skip branch, kept for the same reason Rust's own
   equivalent job keeps its skip branch — a future capability could
   reintroduce a real gate — not because anything is expected to hit
   it today, since zero gates remain as of M7). Gained
   `cache-dependency-path: crates/ciac-backend-go/templates/go.sum.pin`
   on its `actions/setup-go` step — previously uncached (`setup-go`
   needs an explicit dependency path to compute a cache key when the
   repo root itself has no `go.mod`/`go.sum`), now keyed on the single
   checked-in pin file every generated project across every example
   this job iterates shares verbatim, so one cache entry covers all of
   them, satisfying this milestone's own "(module-cached)" exit
   criterion.

   Full workspace verification: `cargo fmt --all --check` clean,
   `cargo clippy --workspace --all-targets -- -D warnings` clean,
   `cargo test --workspace` green (including the three new `dev_cli`/
   `mcp_cli`/`rename_cli` cases, each run individually against the
   real `ciac` binary before the full-suite pass) with zero golden
   churn.
9. **M9 — Simulation slice (gated) + version + retrospective.**
   Pillar 7's slice with exact-outcome acceptance and the refusal
   case; ratchet row; docs/simulation.md + backends.md; workspace
   version bump; arc analysis feeding 25UpdatePlan.md (the third
   cost-model data point).

   **Shipped (v0.24 M9):** the third narrow, gated `ciac sim --target
   go` bet, structurally identical to Rust's (v0.17 M11) and
   TypeScript's (v0.23 M9) own passes, with the same disclosed scope:
   only `db.insert` and broker publish/consume are faked; `error`-only
   failure actions; no `--record`/`--replay`.

   Shipped: `internal/world/world.go.j2` (new) — a hand-written
   restatement (Go cannot `include_str!` Rust source, occupying the
   same position Python's/TypeScript's own restatements do): a single
   `World` type wrapping an occurrence-counted `failureEngine`, an
   in-memory table map (`json.RawMessage` rows), and a queued-message
   slice, with `DBInsertChecked`/`PublishChecked`/`FindWhere`/
   `DrainQueue`/`QueueIsEmpty`/`UnmatchedFailureRules` — a single type
   rather than TS's separate `FakeDatabase`/`FakeQueue` classes,
   matching the plan's own sketch text more closely than the TS
   precedent does. `DBInsertChecked` accepts `row any` and marshals
   internally (reusing the M7 `object_store.Put` lesson: an
   accept-any-marshal-inside signature, not a call-site marshal
   closure) rather than the plan's own literal `json.RawMessage`
   sketch — a deliberate, disclosed deviation, not an oversight.
   `cmd/sim_runner/main.go.j2` (new) — a generated, generic scenario
   interpreter (request/publish/advance/drain/expect), driving
   `net/http/httptest` for requests (no live listener, matching Rust's
   `tower::ServiceExt::oneshot`/TS's `app.inject()`) and
   `workers.HandleMessageOnce<Worker>`/`workers.HandleTick<Job>Once`
   directly for drain/advance steps. Resolves 24UpdatePlan.md's own
   pre-registered open question #2 (sim_runner packaging): `cmd/
   sim_runner` is a second `main` package Go's own `go build ./...`/
   `go vet ./...`/`go test ./...` already discover automatically (the
   same way Cargo auto-discovers `src/bin/sim_runner.rs`), so no
   `TargetInfo.validate` change was needed at all.

   `lib.rs`: `TARGET_INFO.sim` flipped from `SimSupport::None` to
   `SimSupport::Narrow { unsupported: unsupported_sim_capabilities }`;
   `unsupported_sim_capabilities` (new, public) — a near-verbatim port
   of Rust's/TS's own, computed over `lower::scan`'s shared
   `unguarded_verbs`; `lower.rs` gained `pub(crate) use
   ciac_codegen::lower::scan;` (the one-line re-export Rust's and TS's
   own `lower.rs` already carry, missing from Go's until now) so
   `lib.rs` can call it. `world.go`/`cmd/sim_runner/main.go` emission
   gated on `ctx.has_db || ctx.has_queue`, matching Rust's/TS's own
   emission gate exactly.

   `state.go.j2`: `AppState` gained a `World *world.World` field
   (`nil` in production) and a `NewSimulation(cfg, w) *AppState`
   constructor (calls the real `New(cfg)`, then overrides `World`) —
   mirroring TS's `createSimulationState`. `queue.go.j2`'s
   `PublishJSON` gained a `w *world.World` parameter, checked first
   (`w.PublishChecked`) before falling through to the real broker
   client. `lower.rs`'s `publish` leaf and `_steps.go.j2`'s `publish`
   step both thread `st.World` through. `db_insert_tail` gained the
   world guard (`if st.World != nil { st.World.DBInsertChecked(...) }
   else { <real ExecContext> }`), reached regardless of `in_tx`.
   `transaction_stmt` gained the same guard around the real
   `BeginTx`/`Commit`/`Rollback` calls only — `{TX_HANDLE}` (`__tx`) is
   now declared unconditionally as `var __tx *sql.Tx` (typed `nil`)
   rather than `:=`-inferred from `BeginTx`'s own return, since Go
   requires the identifier to exist even on the simulation path that
   never calls `BeginTx` at all.

   **Real bugs found live** (`go build`/`go vet` against the
   zero-Docker sim-vertical-slice/query-verbs/sim-broker-slice
   examples, not hypothesized):
   1. The runner's worker-dispatch table was first written as a Go
      `switch` on the message subject. Go rejects two `case` arms
      sharing one constant at compile time (not silently-unreachable
      code, unlike Rust's `match`/TS's `if`/`else` chain) — exactly
      the shape `examples/sim-broker-slice.ciac`'s two workers on one
      stream produce (the "first worker registered for a subject
      wins" semantics `world.go`'s own doc discloses). Fixed by
      lowering to an `if`/`else`-chain with a `delivered` flag,
      preserving the same first-match-wins semantics without the
      switch's uniqueness constraint.
   2. `cmd/sim_runner/main.go.j2`'s `bytes`/`net/http`/`net/http/
      httptest`/`context` imports were unconditional; a db/queue-only
      program with no `api` (never calls `doRequest`) or with no job
      and no worker whose pipeline has steps (never calls
      `context.Background()`) left three "imported and not used"
      `go vet` failures — found sweeping all 26 examples, not from
      reading the template. Fixed by gating each import on the same
      condition that gates its only call site (`c.apis` for the first
      three; a new `sim_needs_context` flag, computed in `lib.rs` as
      `!ctx.jobs.is_empty() || ctx.workers.iter().any(|w|
      !w.steps.is_empty())`, for the fourth).
   3. `transaction_stmt`'s new `var __tx *sql.Tx` needed
      `database/sql` imported in `internal/logic/*.go`, but the
      existing `needs_sql_pkg` flag only tracked `needs.db_get`
      (`sql.ErrNoRows`) — a `transaction {}` block containing only
      `db.insert` (exactly `sim-vertical-slice.ciac`'s own
      `PlaceOrder`/`HandleOrderCreated` shape) left `database/sql`
      unimported, a real "undefined: sql" failure on the very first
      generate. Fixed with the same textual-fallback idiom
      `needs_fmt`/`needs_redis` already use: `needs.db_get ||
      body_text.contains("*sql.Tx")`.

   **Live-verified**, not just golden-generated: both checked-in
   scenarios (`sim/vertical-slice.ciac-sim.json`,
   `sim/virtual-week.ciac-sim.json`) reproduce their canonical
   outcomes byte-for-byte via `ciac sim --target go ... --json`:
   `{"ProcessOrder":3}`/`{"Reconcile":1}` and
   `{"ProcessOrder":100}`/`{"Reconcile":7}`. `order-system.ciac`
   (declares `auth` and calls `cache.delete`/`cache.set`/`db.count`/
   `db.update`) is refused with all four reasons named, matching
   Rust's/TS's own refusal shape exactly. The `sim-vertical-slice ×
   go` ratchet CI row (`.github/workflows/ci.yml`) was already present
   — added speculatively in M7's own `--system` row sweep, ahead of
   this milestone landing the actual `ciac sim --target go` support it
   assumes; only its stale "all three ... as of v0.23 M9" comment
   needed updating to "all four ... as of v0.24 M9".

   `commands.rs`: `sim_drive_go` (new), mirroring `sim_drive_rust`/
   `sim_drive_typescript` exactly — finds the single `go.mod` project
   dir, requires `cmd/sim_runner/main.go` to exist, `go build -o
   /dev/null ./cmd/sim_runner` once, then `go run ./cmd/sim_runner
   <scenario>` once per `--scenario`, parsing the last stdout line as
   `SimScenarioOutcome`. Wired into `sim_inner`'s `SimSupport::Narrow`
   match as a third `if target == "go"` arm alongside the existing
   `"rust"`/`"typescript"` arms.

   **The equivalence suite's Go-distinguishing cases**, per Pillar
   7's own list: i64 division (already covered at M4's
   `python_rust_typescript_and_go_lower_the_same_handler_body_to_equivalent_shape`
   — Go's native `/` on `int64` already truncates toward zero, no
   `Math.trunc`-style special case needed) and the absent/null/zero
   decode triple (already live-verified at M2 against a running
   binary — a missing field, an explicit `null`, and a legitimate
   zero value each got a real HTTP round-trip). Two genuinely new at
   M9: nil-slice normalization — `db.query`'s Go lowering already
   initializes its result as `[]{Record}{}` (an empty slice literal,
   never a bare `var`, which would stay `nil` and marshal to JSON
   `null`), confirmed both by a new regression test
   (`go_db_query_result_initializes_as_a_non_nil_empty_slice` in
   `typed_handler_equivalence.rs`) and live against
   `query-verbs.ciac`'s own zero-row `/list-active-api` response
   (`{"data":[],"status":"accepted"}`, never `null`). `time.Time` RFC
   3339 sub-second formatting — confirmed live with a standalone
   `encoding/json.Marshal` check: Go trims trailing fractional-second
   zeros (`.5`, `.123456`, `.1`, none at all when exactly zero) rather
   than fixed-width precision; asserted-as-documented in
   [docs/backends.md](docs/backends.md) rather than built into a new
   flagship example, since no checked-in program reaches a
   `Timestamp` field yet. `tests/tests/conformance.rs`'s own C7
   comment updated to record where each boundary case actually lives
   (live proofs plus one regression test, not a new mechanical C7
   harness — neither Rust's nor TS's own narrow-sim pass built one
   either, for the same single-target-single-case reason).

   Docs: [docs/simulation.md](docs/simulation.md) gained Go's column
   in the surface-support table and a Go paragraph mirroring TS's own
   (world/state/queue wiring, the transaction-atomicity-in-production-
   only gap, the switch-vs-if/else wrinkle) plus a "Go's protocol
   mirrors Rust's and TypeScript's own" subsection and CLI example.
   [docs/backends.md](docs/backends.md)'s Simulation section gained
   Go's own paragraph (world/state/queue wiring, the two real bugs,
   the RFC 3339 divergence note) in the same place TS's own paragraph
   already lives.

   Workspace version bumped 0.21.0 → 0.22.0 (the "number assigned at
   execution" convention v0.22 M6 established — the plan-file number
   in the filename and the crate semver are independent counters):
   `Cargo.toml` (`workspace.package.version` + all ten internal
   path-dep version pins), `editors/vscode/package.json`,
   `docs/language.md`'s title. `docs/language.md`'s provider table
   already carries Go's column (flipped incrementally across M1-M8);
   nothing new to flip at M9 since `ciac sim` support isn't part of
   that table's own columns.

   Full workspace verification: `cargo fmt --all --check` clean,
   `cargo clippy --workspace --all-targets -- -D warnings` clean,
   `cargo test --workspace` green, one golden-churn pass (`internal/
   world/world.go`, `cmd/sim_runner/main.go` newly present; `state.go`/
   `queue.go`/every `transaction {}`-using logic file churned for the
   world guard) reviewed diff-by-diff and accepted across all 21
   Go golden trees — no unexpected behavior, every removed line the
   old ungated real-infrastructure path, every added line the new
   world-guarded structure.

   **Arc retrospective, feeding 25UpdatePlan.md's third cost-model
   data point:** Go was the strongest-stdlib, best-deployment-artifact,
   most-precedented (two prior in-repo Go artifacts: the v0.8 M6
   backend spike, the Ext-backend M3 external-protocol reference) of
   the three post-factory language arcs, and the actuals bear that
   out. Every milestone shipped inside its own section without a
   CHECKPOINT go/no-go reversal (M5's factory-acceptance check passed
   cleanly on the first pass, unlike TS's own arc). The recurring
   defect class across all nine milestones was never "Go can't express
   this" — it was always a *textual* completeness gap in a shared
   scanner/flag (`needs_sql_pkg`, `needs_json`, the missing
   `internal/clients` import, the missing `schemas.` qualification,
   and now `sim_needs_context`) that a live `go vet`/`go build` sweep
   caught immediately and a golden-only workflow would have missed
   silently. `HostSyntax`'s ~50-leaf contract (22UpdatePlan.md Pillar
   3) held for the third language in a row without a single contract
   amendment this arc didn't already need for its own error-idiom
   reasons (M4) — Go's own two-Go-artifacts precedent and stdlib
   strength did not, in the end, make the *factory* cheaper than TS's
   arc; both arcs paid the same category of cost (leaf-by-leaf
   `HostSyntax` implementation, ontology wrapper packages, a generated
   sim runner), just with different concrete bug shapes per target's
   own idiom. The clearest forward signal for Java (25UpdatePlan.md):
   budget for "the shared scanner doesn't track a flag this target's
   own idiom needs" as the default defect class per milestone, not a
   surprise — it has now recurred in Rust (E0382), TypeScript (two
   hardcoded refusal-message target names), and Go (the four flags
   above) across three independent arcs, which makes it a property of
   the *factory's* flag-based design, not any one target's
   immaturity. Java's own heavier ecosystem and slower validate loop
   (Maven/Gradle compile times) is the one named risk this arc's own
   confidence note flagged in advance; nothing this arc found changes
   that assessment.

### Per-milestone exit checklists

- **M1 exits when:** the reconciliation notes are committed in this
  file; the registry line is the only external edit; ping passes
  build/vet/gofmt/test live with `CGO_ENABLED=0`; the goleak-backed
  no-infra state test passes; ping goldens committed.
- **M2 exits when:** three-engine CRUD/keyed-store goldens exist;
  sqlite-notes verifies live, zero Docker; C3 passes ×4 targets on
  M2-scope examples; the absent/null/zero boundary suite passes
  (this milestone's centerpiece).
- **M3 exits when:** the four async examples verify; the exported
  seam functions are import-tested; the cron pass-through cases
  match the Rust translation's equivalence fixtures.
- **M4 exits when:** the amendment landed goldens-first (or was
  confirmed pre-generalized) with the identity-syntax golden diff
  reviewed; every verb row goldened; domain-orders rollback proof
  passes on local sqlite; equivalence suite ×4.
- **M5 exits when:** the cost table (with TS actuals as baseline) is
  committed; C1–C5 green ×4; go/no-go sentence recorded for both
  this plan's remainder and 25's start.
- **M6 exits when:** the httptest scope suite passes with zero
  infrastructure; textual parity of the OAuth2 exclusion comment.
- **M7 exits when:** ontology examples verify; trace test ×4; four
  `--system` rows merged including multi-service-media × go with the
  budget note updated.
- **M8 exits when:** zero unexplained gates; dev/MCP/evolution
  transcripts attached; `generated-go` green.
- **M9 exits when:** canonical outcomes byte-exact; refusal reasons
  named; ratchet row merged; docs + version + retrospective done.

## Open questions resolved at implementation (pre-registered)

1. **Workers' migration posture** (apply vs wait-for-ledger) —
   decided by reading what Python's workers process actually does,
   M4; recorded in the template comment.
2. **sim_runner packaging** (third binary vs test-harness driver) —
   decided in M9 against how `TargetInfo.validate` treats extra
   binaries; recorded with the decision.
3. **chi vs pure ServeMux** — if 1.22 patterns cover every generated
   route shape without chi's grouping, chi may be dropped for
   zero-dep routing; measured in M1 against the full route-shape
   inventory (path params, method splits), decided once, recorded.
4. **franz-go consumer-group tuning defaults** (session timeouts,
   rebalance strategy) — pinned in M3 to values that match the
   delivery semantics the system tests assert, recorded in
   `queue.go`'s comments.

## Verification strategy

Standard per-milestone discipline: fmt/clippy/test workspace green;
goldens reviewed diff-by-diff; live proofs as named with
Docker-delegation honesty. Go-specific standing checks: `gofmt -l`
empty on every generated tree — generated code is emitted
pre-formatted and a test asserts gofmt idempotence (formatting is
part of the golden bytes, not a post-pass); `CGO_ENABLED=0` in every
validator invocation (the static-binary guarantee as an executable
assertion); `go vet` in validators and staticcheck in CI;
goleak on the no-infra construction test. go.sum is generated and
snapshotted (the lockfile-equivalent determinism rule); dependency
versions exact in go.mod. Conformance rows extend automatically via
the registry.

The proof ledger by layer, so each claim has a named oracle:

| Claim | Oracle |
| --- | --- |
| generated code compiles/lints/tests | validators (live locally — the Go toolchain is a plain install) |
| wire contract equals other targets | C3 OpenAPI byte-equality; C7 boundary decode/encode |
| topology equals other targets | C4 (subjects/groups/retries/schedules/tables/migration content) |
| logic behavior equals other targets | the behavioral equivalence suite (plan 23's spec + Go's cases) |
| broker delivery/channels/capability round-trips work for real | generated system tests via the four `--system` CI rows (Docker-delegated, honestly) |
| lazy init holds | goleak-backed no-infra state test |
| scope mechanism holds with zero infra | generated httptest suite |
| sim outcomes match the canon | M9's exact-outcome acceptance |
| fake≠real drift is caught | ratchet row (sim assertions vs system-test round-trips on sim-vertical-slice) |
| static binary stays static | CGO_ENABLED=0 everywhere + distroless runtime image having no libc to lean on |

## Explicit cuts

No GORM/ent/sqlc modes. No gRPC surface (CIaC `call` is HTTP; gRPC
is a language-neutral future question, not a Go-plan question). No
Go workspaces/multi-module layouts for multi-service systems —
per-service directories like every target. No generics-heavy
generated APIs: generated code optimizes for reading like plain Go.
No cgo under any flag. No sim record/replay. No zap/zerolog
providers (slog decision recorded with its revisit seam).

## Risks

- **The error-idiom amendment destabilizes lower_core.** Mitigated
  by M4's hard ordering: amendment + existing-target byte-identical
  proof lands before any Go leaf consumes it; the identity-syntax
  golden makes the contract change itself reviewable.
- **Zero-value/null decode bugs.** The plan's top-ranked defect
  class, countered structurally (pointer-only-for-Option as a
  lower_core invariant, generated presence checks, harness boundary
  cases) rather than by review vigilance.
- **franz-go/modernc vs raw install-base leaders (segmentio/mattn).**
  Both trade install-base rank for the CGO-free story; both recorded
  with named fallbacks behind one-module seams.
- **DSN containment leaks.** Config-local by construction; the
  system tests' real connection round-trips are the tripwire.
- **gorilla/websocket governance regresses again.** Recorded
  fallback (coder/websocket) behind the one-file channel module.

## Milestone dependencies and parallelism

M1→M2→M3→M4→M5 are sequential (each un-gates components the next
consumes); M6 and M7 are independent of each other after M5 and may
interleave; M8 requires M6+M7; M9 requires M8. Within M4, the
contract-amendment commit strictly precedes any Go leaf commit — the
one intra-milestone ordering this plan hard-codes, because it is the
one place a shortcut could silently damage the other three targets.
Nothing in this plan blocks concurrent repo work outside the new
crate except the amendment's brief window in shared `lower_core`.

## Confidence and handoff

High — the strongest stdlib assist of the three plans, the best
deployment artifact of all five backends, two prior in-repo Go
artifacts, and a factory already hardened by one full language arc.
The handoff to 25UpdatePlan.md is the M5/M9 measured reports and the
by-then twice-validated `HostSyntax` contract; Java — the heaviest
ecosystem and slowest validate loop of the three — deliberately
inherits the most-proven factory rather than road-testing it.
