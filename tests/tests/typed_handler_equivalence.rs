//! v0.7 M4: "lowering equivalence" property check, scoped to what's
//! practically verifiable in a sandbox where neither backend has a real
//! behavioral-test seam yet (07UpdatePlan.md's own goal — "two hosts,
//! one meaning" — held to the full standard of running fakes/mocks
//! against both hosts and diffing outputs is M3's/M4's explicit
//! Non-goal; see the M3 and M4 plans). This compiles the *same* source
//! through both backends in one test and asserts structural parity
//! directly against each other, not just against independent fixed
//! strings: same table/column names, same field names, same verb call
//! counts. A real behavioral-output equivalence suite (running
//! generated fakes against both hosts) is future work once a
//! mock/trait seam exists for the Rust runtime wrappers.
//!
//! `23UpdatePlan.md` M4 extends this structural-parity style (not the
//! full JSON-fixture/three-runner-mechanism "specified" design that
//! plan's own text sketches — a disclosed, pragmatic scope reduction:
//! that fuller system doesn't exist yet even for the two established
//! targets, so growing today's real, working mechanism to a third
//! target is the concrete step, not blocking on a rewrite) to a third
//! target, TypeScript, via a second canonical example
//! ([`DIVISION_EXAMPLE`]) scoped to capabilities TS actually supports
//! this milestone.
//!
//! `24UpdatePlan.md` M4 extends the same [`DIVISION_EXAMPLE`] to a
//! fourth target, Go — `db Postgres`-only, matching the scope every
//! other target already needed for this example, so no new capability
//! gating question arises. Go's own two named divergences pin the
//! opposite side of Pillar 2's own table from TS's: `Int / Int` needs
//! no `Math.trunc`-style special case at all (Go's native `/` on
//! `int64` already truncates toward zero, identical to Rust's `i64`
//! division — confirmed by reading `GoSyntax::binary`'s own doc), and
//! `Json` indexing panics with a Go-idiomatic `KeyError: '<key>'`
//! message after an explicit `json.Unmarshal` (the `Json` HIR type's
//! Go representation is `json.RawMessage`, not `any` — a concrete
//! `[]byte`-backed type needing an explicit decode before a map-key
//! lookup is possible, unlike TS's `unknown`/Python's `dict[str,
//! Any]`, both already directly indexable).
//!
//! `25UpdatePlan.md` M4 extends the same [`DIVISION_EXAMPLE`] to a
//! fifth target, Java. `Int / Int` needs no special case either —
//! Java's `long / long` already truncates toward zero, the same as
//! Go's/Rust's native `/`. `Json` indexing panics with a Java-idiomatic
//! `Schemas.indexOrThrow(base, key)` call, throwing the shared
//! `BadRequestException` with the same `KeyError: '<key>'` message
//! text every other target's own leaf carries — found live while
//! exercising this very example against JDBC generation: Postgres's
//! `jsonb` column additionally needs the placeholder to carry an
//! explicit `?::jsonb` cast (a bound `String` alone is rejected —
//! "column .. is of type jsonb but expression is of type character
//! varying"), a JDBC-specific trap none of the other four targets'
//! own drivers have (each already knows the bound value's JSON-ness
//! from its own client-side type wrapper).

use ciac_backend_go::GoBackend;
use ciac_backend_java::JavaBackend;
use ciac_backend_python::PythonBackend;
use ciac_backend_rust::RustBackend;
use ciac_backend_ts::TsBackend;
use ciac_codegen::{Backend, GenOptions};
use ciac_integration_tests::compile;

const CANONICAL_EXAMPLE: &str = r#"
service MediaExample;

use {
    db Postgres;
    object_store S3;
}

record Video {
    id: Uuid;
    title: String;
    status: enum { Pending, Ready };
}

error NotFound {
    id: Uuid;
}

table Videos: Video;

handler StoreVideo(v: Video) -> Video {
    let key = "videos/" + v.id;
    object_store.put(key, v);
    let inserted = db.insert(Videos, v);
    let ready = if inserted.status == Pending {
        inserted { status: Ready }
    } else {
        inserted
    };
    let described = match ready.status {
        Ready -> { return ready; }
        _ -> { fail NotFound(v.id); }
    };
    return described;
}

api Upload: Video {
    method: POST;
    path: "/videos";
}

pipeline Upload:
    StoreVideo
    -> Return;
"#;

fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

#[test]
fn python_and_rust_lower_the_same_handler_body_to_equivalent_shape() {
    let (ir, diags) = compile(CANONICAL_EXAMPLE);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let py_project = PythonBackend
        .generate(&ir, &GenOptions::default())
        .expect("python must build the typed inline handler");
    let rust_project = RustBackend
        .generate(&ir, &GenOptions::default())
        .expect("rust must build the typed inline handler");

    let py_logic = py_project
        .get("app/logic/store_video.py")
        .expect("python emits the compiler-owned logic file");
    let rust_logic = rust_project
        .get("src/logic/store_video.rs")
        .expect("rust emits the compiler-owned logic file");

    // Same table, same columns: both hosts insert into the table the
    // `.ciac` source declared. Python constructs the row via a splat
    // (`**v.model_dump()`, so it never names fields individually);
    // Rust's raw SQL spells out every column, so it's the one that can
    // assert the full field list.
    assert!(py_logic.contains("Videos(**v.model_dump())"));
    assert!(rust_logic.contains("sqlx::query(\"INSERT INTO videos (id, title, status)"));

    // Same verb call counts: one object_store.put, one db.insert (commit
    // on the Python side, execute on the Rust side — each host's own
    // spelling of "run the query"), one raised/returned error.
    assert_eq!(
        count(py_logic, "self.object_store.put("),
        1,
        "python: expected exactly one object_store.put"
    );
    assert_eq!(
        count(rust_logic, "self.object_store.put("),
        1,
        "rust: expected exactly one object_store.put"
    );
    assert_eq!(
        count(py_logic, "await self.session.commit()"),
        1,
        "python: expected exactly one db.insert commit"
    );
    assert_eq!(
        count(rust_logic, ".execute(self.db)"),
        1,
        "rust: expected exactly one db.insert execute"
    );
    assert_eq!(
        count(py_logic, "raise NotFound("),
        1,
        "python: expected exactly one fail -> raise"
    );
    assert_eq!(
        count(rust_logic, "NotFound {"),
        1,
        "rust: expected exactly one fail -> Err(NotFound{{..}})"
    );

    // Same control shape: both hosts branch once on `status == Pending`
    // and dispatch once on the resulting enum (`match`/`if` — Python's
    // `if`/`elif` chain for `match`, Rust's native `match`).
    assert_eq!(count(py_logic, "Pending"), count(rust_logic, "Pending"));
    assert_eq!(count(py_logic, "Ready"), count(rust_logic, "Ready"));

    // Same real, typed signature — no generic payload/dict fallback on
    // either side.
    assert!(py_logic.contains("def handle(self, v: Video) -> Video:"));
    assert!(rust_logic.contains("fn handle(&self, v: Video) -> anyhow::Result<Video>"));

    // Same error record shape: raisable on both hosts, carrying the same
    // field.
    let py_schemas = py_project.get("app/schemas.py").expect("python schemas");
    let rust_schemas = rust_project.get("src/schemas.rs").expect("rust schemas");
    assert!(py_schemas.contains("class NotFound(Exception):"));
    assert!(rust_schemas.contains("thiserror::Error"));
    assert!(py_schemas.contains("id: str"));
    assert!(rust_schemas.contains("pub id: String,"));
}

/// A second canonical example, deliberately scoped to `db`-only
/// capabilities (no `object_store`) so it can include a third target:
/// `23UpdatePlan.md` M4 pulls `HostSyntax` leaves forward for
/// TypeScript, but `Component::ObjectStore` itself stays
/// `CIAC0011`-refused there until M7 (the wrapper client, not the leaf,
/// is M7's job — see `TsBackend::supports`'s own doc), so the
/// `CANONICAL_EXAMPLE` above can't add TS without changing what it's
/// actually testing. This example instead targets the two named,
/// *documented* divergence cases Pillar 2 flags for the equivalence
/// suite: `Int / Int` (Python stays true division; Rust's native `/`
/// and TS's `Math.trunc(a / b)` both truncate toward zero, matching
/// `i64`) and `Json` indexing (Python's bare `base[key]`, relying on
/// the language's own `KeyError`; TS's optional-chained access plus an
/// explicit thrown `KeyError`-shaped error, since JS has no equivalent
/// built-in).
const DIVISION_EXAMPLE: &str = r#"
service DivisionExample;

use {
    db Postgres;
}

record Payload {
    id: Uuid;
    total: Int;
    count: Int;
    extra: Json;
}

table Payloads: Payload;

handler ComputeAverage(p: Payload) -> Payload {
    let avg = p.total / p.count;
    p.extra["label"];
    let updated = p { total: avg };
    let inserted = db.insert(Payloads, updated);
    return inserted;
}

api Compute: Payload {
    method: POST;
    path: "/compute";
}

pipeline Compute:
    ComputeAverage
    -> Return;
"#;

#[test]
fn python_rust_typescript_go_and_java_lower_the_same_handler_body_to_equivalent_shape() {
    let (ir, diags) = compile(DIVISION_EXAMPLE);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let py_project = PythonBackend
        .generate(&ir, &GenOptions::default())
        .expect("python must build the typed inline handler");
    let rust_project = RustBackend
        .generate(&ir, &GenOptions::default())
        .expect("rust must build the typed inline handler");
    let ts_project = TsBackend
        .generate(&ir, &GenOptions::default())
        .expect("typescript must build the typed inline handler");
    let go_project = GoBackend
        .generate(&ir, &GenOptions::default())
        .expect("go must build the typed inline handler");
    let java_project = JavaBackend
        .generate(&ir, &GenOptions::default())
        .expect("java must build the typed inline handler");

    let py_logic = py_project
        .get("app/logic/compute_average.py")
        .expect("python emits the compiler-owned logic file");
    let rust_logic = rust_project
        .get("src/logic/compute_average.rs")
        .expect("rust emits the compiler-owned logic file");
    let ts_logic = ts_project
        .get("src/logic/compute_average.ts")
        .expect("typescript emits the compiler-owned logic file");
    let go_logic = go_project
        .get("internal/logic/compute_average.go")
        .expect("go emits the compiler-owned logic file");
    let java_logic = java_project
        .get("src/main/java/com/ciac/divisionexample/logic/ComputeAverage.java")
        .expect("java emits the compiler-owned logic file");

    // The `Int / Int` divergence, named and pinned rather than
    // discovered: Python's `/` is true division (never truncates);
    // Rust's native `/` and TS's `Math.trunc(a / b)` both truncate
    // toward zero, matching `i64` — an intentional, documented
    // cross-target difference, not a bug in either lowering.
    assert!(
        py_logic.contains("(p.total / p.count)") && !py_logic.contains("Math.trunc"),
        "python: Int/Int division must stay true division, found: {py_logic}"
    );
    assert!(
        rust_logic.contains("(p.total / p.count)"),
        "rust: Int/Int division truncates natively via `/`, found: {rust_logic}"
    );
    assert!(
        ts_logic.contains("Math.trunc(p.total / p.count)"),
        "typescript: Int/Int division must lower to Math.trunc for i64-truncation parity \
         with Rust, found: {ts_logic}"
    );
    // Go's native `/` on `int64` already truncates toward zero, the
    // same as Rust's `i64` division — no `Math.trunc`-style special
    // case needed, a real simplification over TS's own leaf.
    assert!(
        go_logic.contains("(p.Total / p.Count)") && !go_logic.contains("trunc"),
        "go: Int/Int division truncates natively via `/`, found: {go_logic}"
    );
    // Java's native `/` on `long` also already truncates toward zero —
    // the same simplification Go's own leaf gets, no `Math.trunc`-style
    // special case needed.
    assert!(
        java_logic.contains("(p.total() / p.count())") && !java_logic.contains("trunc"),
        "java: Int/Int division truncates natively via `/`, found: {java_logic}"
    );

    // The `Json` indexing divergence: Python relies on the language's
    // own `KeyError` for a missing key; TS has no equivalent built-in,
    // so the leaf must synthesize an equivalent thrown error rather
    // than silently propagating `undefined`. Go's `Json` HIR type is
    // `json.RawMessage` (a concrete `[]byte`), not `any`, so its own
    // leaf must `json.Unmarshal` before a key lookup is even possible,
    // then panics with the same `KeyError: '<key>'` message text TS's
    // thrown error carries.
    assert!(py_logic.contains(r#"p.extra["label"]"#));
    assert!(
        ts_logic.contains("KeyError: 'label'") && ts_logic.contains("throw new Error"),
        "typescript: Json indexing must throw a KeyError-shaped error on a missing key, \
         found: {ts_logic}"
    );
    assert!(
        go_logic.contains("json.Unmarshal(p.Extra, &__m)")
            && go_logic.contains("KeyError: 'label'"),
        "go: Json indexing must unmarshal into a map and panic with a KeyError-shaped \
         message on a missing key, found: {go_logic}"
    );
    // Java's `Json` HIR type is `JsonNode` — already directly
    // indexable (like TS's `unknown`/Python's `dict`), so no explicit
    // decode step is needed; only the missing-key throw itself.
    assert!(
        java_logic.contains(r#"Schemas.indexOrThrow(p.extra(), "label")"#),
        "java: Json indexing must throw a KeyError-shaped error on a missing key via \
         Schemas.indexOrThrow, found: {java_logic}"
    );

    // Same table, same columns, all five hosts: Python's ORM splat
    // never names fields individually, so only Rust's, TS's, Go's, and
    // Java's raw SQL assert the full column list. Java's own Postgres
    // `jsonb` column additionally needs the `extra` placeholder to
    // carry an explicit `::jsonb` cast (found live: a bound `String`
    // alone is rejected by Postgres's own type check) — none of the
    // other four targets' drivers need this, each already knowing the
    // bound value's JSON-ness from its own client-side type wrapper.
    assert!(py_logic.contains("Payloads(**v2.model_dump())"));
    assert!(rust_logic.contains("sqlx::query(\"INSERT INTO payloads (id, total, count, extra)"));
    assert!(ts_logic.contains(r#""INSERT INTO payloads (id, total, count, extra)"#));
    assert!(go_logic.contains("INSERT INTO payloads (id, total, count, extra)"));
    assert!(
        java_logic
            .contains("INSERT INTO payloads (id, total, count, extra) VALUES (?, ?, ?, ?::jsonb)"),
        "java: db.insert's Json column placeholder must carry an explicit ::jsonb cast, \
         found: {java_logic}"
    );

    // Same verb call counts across all five: one db.insert (commit/
    // execute/query/ExecContext/update — each host's own spelling of
    // "run the query").
    assert_eq!(
        count(py_logic, "await self.session.commit()"),
        1,
        "python: expected exactly one db.insert commit"
    );
    assert_eq!(
        count(rust_logic, ".execute(self.db)"),
        1,
        "rust: expected exactly one db.insert execute"
    );
    assert_eq!(
        count(ts_logic, "INSERT INTO payloads"),
        1,
        "typescript: expected exactly one db.insert query"
    );
    assert_eq!(
        count(go_logic, "INSERT INTO payloads"),
        1,
        "go: expected exactly one db.insert ExecContext"
    );
    assert_eq!(
        count(java_logic, "INSERT INTO payloads"),
        1,
        "java: expected exactly one db.insert update"
    );

    // Same real, typed signature on every host — no generic payload/
    // dict fallback anywhere.
    assert!(py_logic.contains("def handle(self, p: Payload) -> Payload:"));
    assert!(rust_logic.contains("fn handle(&self, p: Payload) -> anyhow::Result<Payload>"));
    assert!(ts_logic.contains("async handle(p: Payload): Promise<Payload>"));
    assert!(go_logic.contains("func ComputeAverage(ctx context.Context, st *state.AppState, p schemas.Payload) (schemas.Payload, error)"));
    assert!(
        java_logic.contains("public Payload handle(Payload p)"),
        "java: expected a real, typed `handle` signature, found: {java_logic}"
    );
}

/// v0.24 M9: the "nil-slice `List`" divergence-ledger row (Pillar 7's
/// table: "normalized to `[]`, disclosed"). A bare Go `var out []T`
/// stays `nil` until the first `append`, and `encoding/json` marshals
/// a `nil` slice as `null`, not `[]` — a real trap Python's/Rust's/
/// TS's own empty-list JSON shape doesn't have. `db.query`'s Go
/// lowering avoids it by initializing the result with an empty *slice
/// literal* (`[]T{}`, never left as a bare `var`) before the scan loop
/// runs. Confirmed structurally here and, separately, live at v0.24
/// M9 against `examples/single-service/query-verbs.ciac`: a zero-row `POST
/// /list-active-api` over a real SQLite file returned
/// `{"data":[],"status":"accepted"}`, never `{"data":null,...}`.
#[test]
fn go_db_query_result_initializes_as_a_non_nil_empty_slice() {
    const SOURCE: &str = r#"
service ListProbe;

use {
    db SQLite;
}

record Note {
    id: Uuid;
    title: String;
}

record NoFilter {
    marker: Bool;
}

table Notes: Note;

handler ListAll(f: NoFilter) -> [Note] {
    return db.query(Notes);
}

api ListAllApi: NoFilter;
pipeline ListAllApi: ListAll -> Return;
"#;
    let (ir, diags) = compile(SOURCE);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let go_project = GoBackend
        .generate(&ir, &GenOptions::default())
        .expect("go must build the list-returning handler");
    let go_logic = go_project
        .get("internal/logic/list_all.go")
        .expect("go emits the compiler-owned logic file");

    assert!(
        go_logic.contains("[]schemas.Note{}"),
        "go: db.query's result slice must be initialized as an empty slice literal, not left \
         as a bare `var` (which stays nil, and a nil slice marshals to JSON `null` instead of \
         `[]`); found: {go_logic}"
    );
    assert!(
        !go_logic.contains("var __out"),
        "go: the result slice must never be declared via a bare `var` (nil by default); \
         found: {go_logic}"
    );
}

/// v0.25 M9: Java's own world-guard shape, structurally confirmed the
/// same way Go's nil-slice test above is (not just live-verified
/// against `sim-vertical-slice.ciac`, though that live proof exists
/// too — see 25UpdatePlan.md M9's Shipped notes). `db.insert` (bare
/// and inside `transaction {}`) and the pipeline-level `publish` step
/// must each branch on `world != null`, routing to `World`'s fake
/// table/queue instead of the real `JdbcClient`/`Queue` path — the
/// mechanism `ciac sim --target java` depends on to run with zero
/// infrastructure reachable.
#[test]
fn java_db_insert_and_publish_are_world_guarded() {
    const SOURCE: &str = r#"
service WorldGuardExample;

use {
    db Postgres;
    queue NATS;
}

record Order {
    id: Uuid;
    total: Float;
}
table Orders: Order;

stream OrderCreated: Order;

handler PlaceOrder(order: Order) -> Order {
    transaction {
        db.insert(Orders, order);
    }
    return order;
}

api PlaceOrderApi: Order {
    method: POST;
    path: "/orders";
}
pipeline PlaceOrderApi:
    PlaceOrder
    -> publish OrderCreated
    -> Return;
"#;
    let (ir, diags) = compile(SOURCE);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let java_project = JavaBackend
        .generate(&ir, &GenOptions::default())
        .expect("java must build a transaction+publish handler");
    let java_logic = java_project
        .get("src/main/java/com/ciac/worldguardexample/logic/PlaceOrder.java")
        .expect("java emits the compiler-owned logic file");

    assert!(
        java_logic.contains("if (world != null) {"),
        "java: db.insert must branch on `world != null`; found: {java_logic}"
    );
    assert!(
        java_logic.contains("world.dbInsertChecked(\"orders\", "),
        "java: the world branch must route through World.dbInsertChecked with the SQL table \
         name; found: {java_logic}"
    );
    assert!(
        java_logic.contains("INSERT INTO orders"),
        "java: the real (non-world) branch must still emit the genuine INSERT; found: \
         {java_logic}"
    );
    assert!(
        java_logic.contains("Runnable __txBody"),
        "java: `transaction {{}}` must wrap its body once in a `Runnable`, shared unchanged by \
         both the world and real branches, rather than duplicating (and needing to reflow the \
         indentation of) the framework-supplied `inner_lines`; found: {java_logic}"
    );

    let queue_java = java_project
        .get("src/main/java/com/ciac/worldguardexample/state/Queue.java")
        .expect("java emits Queue.java when queue_engine is set");
    assert!(
        queue_java.contains("if (world != null) {"),
        "java: Queue.publishJson must check world first -- the single choke point every \
         `queue.publishJson` call site (pipeline `publish` steps AND the `publish <Stream>(..)` \
         HIR leaf) shares, so neither call site needs its own world-awareness; found: \
         {queue_java}"
    );
    assert!(
        queue_java.contains("world.publishChecked(subject, "),
        "java: Queue's world branch must route through World.publishChecked; found: {queue_java}"
    );
}
