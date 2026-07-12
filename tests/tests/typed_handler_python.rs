//! v0.7 M3: Python graduates from gating typed handlers (M2) to actually
//! emitting them. String-contains assertions in the spirit of
//! `tests/tests/cron_vectors.rs` — a full golden snapshot comes in M6
//! once an example adopts the v0.7 syntax.

use ciac_backend_python::PythonBackend;
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
fn python_emits_a_runnable_typed_handler() {
    let (ir, diags) = compile(CANONICAL_EXAMPLE);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let project = PythonBackend
        .generate(&ir, &GenOptions::default())
        .expect("python must build a typed inline handler (M3)");

    let logic = project
        .get("app/logic/store_video.py")
        .expect("inline handler body lowers to a compiler-owned logic file");
    assert!(
        logic.contains("class StoreVideo:"),
        "expected a StoreVideo class: {logic}"
    );
    assert!(
        logic.contains("await self.object_store.put("),
        "expected the object_store.put verb call lowered: {logic}"
    );
    assert!(
        logic.contains("self.session.add(Videos(**v.model_dump()))"),
        "expected the db.insert verb call lowered against the Videos model: {logic}"
    );
    assert!(
        logic.contains("await self.session.commit()"),
        "expected db.insert to commit: {logic}"
    );
    assert!(
        logic.contains("model_copy(update="),
        "expected the functional record update lowered via Pydantic's model_copy: {logic}"
    );
    assert!(
        logic.contains("raise NotFound("),
        "expected `fail NotFound(..)` lowered to a raise: {logic}"
    );
    assert!(
        logic.contains("async def handle(self, v: Video) -> Video:"),
        "expected the real typed signature, not a generic payload: {logic}"
    );

    // The pipeline call site must import from `app.logic`, not
    // `app.services` — the inline handler is compiler-owned, unlike a
    // classic/`extern` handler's seeded stub.
    let api = project
        .get("app/api/upload.py")
        .expect("api route file is generated");
    assert!(
        api.contains("from app.logic.store_video import StoreVideo"),
        "expected the route to import the typed handler from app.logic: {api}"
    );

    // `table Videos: Video;` gets a real SQLAlchemy model, registered on
    // the same `Base` as CRUD resources.
    let models = project
        .get("app/models.py")
        .expect("a declared table generates app/models.py");
    assert!(
        models.contains("class Videos(Base):"),
        "expected a Videos model: {models}"
    );
    assert!(
        models.contains("__tablename__ = \"videos\""),
        "expected the table's own name, not the record's: {models}"
    );

    // The error record becomes a raisable exception, not a Pydantic model.
    let schemas = project
        .get("app/schemas.py")
        .expect("schemas are generated");
    assert!(
        schemas.contains("class NotFound(Exception):"),
        "expected NotFound to be an Exception subclass: {schemas}"
    );

    // A generated behavioral test exercises the lowering against mocks.
    let test = project
        .get("tests/test_logic_store_video.py")
        .expect("an inline handler gets a generated behavioral test");
    assert!(
        test.contains("from app.logic.store_video import StoreVideo"),
        "expected the test to import the handler under test: {test}"
    );
    assert!(
        test.contains("session.add.assert_called_once()")
            && test.contains("session.commit.assert_awaited_once()"),
        "expected assertions on the mocked db.insert call: {test}"
    );
    assert!(
        test.contains("object_store.put.assert_awaited_once()"),
        "expected assertions on the mocked object_store.put call: {test}"
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
fn python_emits_a_typed_seeded_stub_for_extern_handlers() {
    let (ir, diags) = compile(EXTERN_EXAMPLE);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let project = PythonBackend
        .generate(&ir, &GenOptions::default())
        .expect("python must build an extern handler stub (M3)");

    assert!(
        project.get("app/logic/store_video.py").is_none(),
        "an extern handler has no body to lower into app/logic"
    );
    let stub = project
        .get("app/services/store_video.py")
        .expect("extern handler gets a seeded stub under app/services, like classic handlers");
    assert!(
        stub.contains("async def handle(self, v: Video) -> Video:"),
        "expected the real typed signature on the stub: {stub}"
    );
    assert!(
        stub.contains("raise NotImplementedError"),
        "expected a NotImplementedError body: {stub}"
    );

    let api = project
        .get("app/api/upload.py")
        .expect("api route is generated");
    assert!(
        api.contains("from app.services.store_video import StoreVideo"),
        "expected the route to import the seeded stub from app.services: {api}"
    );
}
