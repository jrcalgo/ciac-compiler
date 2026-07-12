//! v0.7 M4: Rust graduates from gating typed handlers (M2) to actually
//! emitting them, the same way Python did in M3. String-contains
//! assertions in the spirit of `tests/tests/cron_vectors.rs` — a full
//! golden snapshot comes in M6 once an example adopts the v0.7 syntax.

use ciac_backend_rust::RustBackend;
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

#[test]
fn rust_emits_a_runnable_typed_handler() {
    let (ir, diags) = compile(CANONICAL_EXAMPLE);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let project = RustBackend
        .generate(&ir, &GenOptions::default())
        .expect("rust must build a typed inline handler (M4)");

    let logic = project
        .get("src/logic/store_video.rs")
        .expect("inline handler body lowers to a compiler-owned logic file");
    assert!(
        logic.contains("pub struct StoreVideo<'a>"),
        "expected a StoreVideo struct: {logic}"
    );
    assert!(
        logic.contains("self.object_store.put(&"),
        "expected the object_store.put verb call lowered: {logic}"
    );
    assert!(
        logic.contains("sqlx::query(\"INSERT INTO videos"),
        "expected the db.insert verb call lowered against the videos table: {logic}"
    );
    assert!(
        logic.contains(".execute(self.db)"),
        "expected db.insert to execute against the borrowed pool: {logic}"
    );
    assert!(
        logic.contains("..v2") || logic.contains(", ..v"),
        "expected the functional record update lowered via Rust's struct-update syntax: {logic}"
    );
    assert!(
        logic.contains("Err(NotFound {") && logic.contains("}.into())"),
        "expected `fail NotFound(..)` lowered to an Err(..).into(): {logic}"
    );
    assert!(
        logic.contains("pub async fn handle(&self, v: Video) -> anyhow::Result<Video>"),
        "expected the real typed signature, not a generic payload: {logic}"
    );

    // The pipeline call site must import from `crate::logic`, not
    // `crate::services` — the inline handler is compiler-owned, unlike a
    // classic/`extern` handler's seeded stub.
    let route = project
        .get("src/routes/upload.rs")
        .expect("route file is generated");
    assert!(
        route.contains("use crate::logic::store_video::StoreVideo;"),
        "expected the route to import the typed handler from crate::logic: {route}"
    );

    // `table Videos: Video;` gets a real SQLx model, registered with the
    // same `ensure_schema()` as CRUD resources, plus the row -> schema
    // conversion `db.get` needs (exercised by a separate probe below).
    let models = project
        .get("src/models.rs")
        .expect("a declared table generates src/models.rs");
    assert!(
        models.contains("pub struct Videos {"),
        "expected a Videos model: {models}"
    );
    assert!(
        models.contains("impl TryFrom<Videos> for Video"),
        "expected a row -> schema TryFrom conversion: {models}"
    );
    // `table` schema is provisioned by `ciac build`'s migration differ
    // (v0.7 M5), not the backend itself — `RustBackend::generate` alone
    // (as called directly here) never sees the previous manifest, so
    // `ensure_schema` in src/db.rs stays scoped to CRUD resources (none
    // in this example) and emits no table-specific SQL.
    let db = project.get("src/db.rs").expect("src/db.rs is generated");
    assert!(
        !db.contains("CREATE TABLE"),
        "table schema now comes from generated migrations, not ensure_schema: {db}"
    );

    // The error record becomes a raisable `std::error::Error`, not a
    // plain data struct.
    let schemas = project
        .get("src/schemas.rs")
        .expect("schemas are generated");
    assert!(
        schemas.contains("thiserror::Error"),
        "expected NotFound to derive thiserror::Error: {schemas}"
    );

    // A named enum gets the reverse (`String -> enum`) parse `db.get`
    // needs, alongside the existing `as_str()`.
    assert!(
        schemas.contains("pub fn from_str(s: &str) -> anyhow::Result<Self>"),
        "expected a from_str reverse-parse on VideoStatus: {schemas}"
    );
}

const EXTERN_EXAMPLE: &str = r#"
service ExternHandlerProbe;

record Video {
    id: Uuid;
}

extern handler StoreVideo(v: Video) -> Video;

api Upload: Video {
    method: POST;
    path: "/videos";
}

pipeline Upload:
    StoreVideo
    -> Return;
"#;

#[test]
fn rust_emits_a_typed_seeded_stub_for_extern_handlers() {
    let (ir, diags) = compile(EXTERN_EXAMPLE);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let project = RustBackend
        .generate(&ir, &GenOptions::default())
        .expect("rust must build an extern handler stub (M4)");

    assert!(
        project.get("src/logic/store_video.rs").is_none(),
        "an extern handler has no body to lower into src/logic"
    );
    let stub = project
        .get("src/services/store_video.rs")
        .expect("extern handler gets a seeded stub under src/services, like classic handlers");
    assert!(
        stub.contains("pub async fn handle(&self, v: Video) -> anyhow::Result<Video>"),
        "expected the real typed signature on the stub: {stub}"
    );
    assert!(
        stub.contains("not implemented"),
        "expected a not-implemented body: {stub}"
    );

    let route = project
        .get("src/routes/upload.rs")
        .expect("route is generated");
    assert!(
        route.contains("use crate::services::store_video::StoreVideo;"),
        "expected the route to import the seeded stub from crate::services: {route}"
    );
}
