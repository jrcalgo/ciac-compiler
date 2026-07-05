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

use ciac_backend_python::PythonBackend;
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
