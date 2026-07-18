//! v0.8 M2: blueprint expansion (`ciac-sema/src/blueprints.rs`),
//! exercised through the same `ciac_sema::analyze` entry point real
//! programs use (blueprint expansion runs as `analyze`'s first step,
//! so there's nothing extra to wire up here).

use ciac_codegen::GenOptions;
use ciac_diagnostics::ErrorCode;
use ciac_integration_tests::{compile, full_parity_backends};

const AUDITED_CRUD: &str = r#"
service S;

record Video { id: Uuid; title: String; }
record User { id: Uuid; title: String; }
record AuditEvent { id: Uuid; title: String; }

blueprint AuditedCrud<R: record> {
    params { prefix: String; }
    use { db main Postgres; }
    crud Resource: R;
    stream Audited: AuditEvent;
    handler AfterWrite(r: R) -> R {
        return r;
    }
}
"#;

#[test]
fn two_expansions_of_the_same_blueprint_are_hygienic() {
    let src = format!(
        "{AUDITED_CRUD}\nservice Catalog {{ expand AuditedCrud<Video> {{ prefix: \"/v1\"; }} }}\n\
         service Accounts {{ expand AuditedCrud<User> {{ prefix: \"/v1\"; }} }}\n"
    );
    let (ir, diags) = compile(&src);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("expands and type-checks");

    // Both expansions generate on both backends without collision.
    for backend in full_parity_backends() {
        backend
            .generate(&ir, &GenOptions::default())
            .unwrap_or_else(|err| panic!("{}: {err}", backend.id()));
    }
}

#[test]
fn unknown_blueprint_is_ciac0048() {
    let src = format!("{AUDITED_CRUD}\nservice Catalog {{ expand NotReal<Video>; }}\n");
    let (ir, diags) = compile(&src);
    assert!(ir.is_none());
    assert!(diags.codes().contains(&ErrorCode::UnknownBlueprint));
}

#[test]
fn unknown_param_is_ciac0049() {
    let src = format!(
        "{AUDITED_CRUD}\nservice Catalog {{ expand AuditedCrud<Video> {{ wrong: \"x\"; }} }}\n"
    );
    let (ir, diags) = compile(&src);
    assert!(ir.is_none());
    assert!(diags.codes().contains(&ErrorCode::BlueprintArityMismatch));
}

#[test]
fn missing_param_is_ciac0049() {
    let src = format!("{AUDITED_CRUD}\nservice Catalog {{ expand AuditedCrud<Video>; }}\n");
    let (ir, diags) = compile(&src);
    assert!(ir.is_none());
    assert!(diags.codes().contains(&ErrorCode::BlueprintArityMismatch));
}

#[test]
fn wrong_param_value_type_is_ciac0049() {
    let src = format!(
        "{AUDITED_CRUD}\nservice Catalog {{ expand AuditedCrud<Video> {{ prefix: 5; }} }}\n"
    );
    let (ir, diags) = compile(&src);
    assert!(ir.is_none());
    assert!(diags.codes().contains(&ErrorCode::BlueprintArityMismatch));
}

#[test]
fn non_record_type_argument_is_ciac0050() {
    let src = format!(
        "{AUDITED_CRUD}\nservice Catalog {{ expand AuditedCrud<NotARecord> {{ prefix: \"/v1\"; }} }}\n"
    );
    let (ir, diags) = compile(&src);
    assert!(ir.is_none());
    assert!(diags
        .codes()
        .contains(&ErrorCode::BlueprintConstraintViolation));
}

#[test]
fn same_blueprint_same_type_arg_twice_is_a_duplicate_declaration() {
    let src = format!(
        "{AUDITED_CRUD}\nservice Catalog {{\n\
         expand AuditedCrud<Video> {{ prefix: \"/a\"; }}\n\
         expand AuditedCrud<Video> {{ prefix: \"/b\"; }}\n}}\n"
    );
    let (ir, diags) = compile(&src);
    assert!(ir.is_none());
    assert!(diags.codes().contains(&ErrorCode::DuplicateDeclaration));
}

#[test]
fn duplicate_blueprint_declaration_is_a_duplicate_declaration() {
    let src = format!("{AUDITED_CRUD}\n{AUDITED_CRUD}\n");
    let (ir, diags) = compile(&src);
    assert!(ir.is_none());
    assert!(diags.codes().contains(&ErrorCode::DuplicateDeclaration));
}

/// v0.8 M3: `std/crud.ciac`'s `Crud<R: record>` blueprint must generate
/// byte-identically to hand-written `crud Videos: Video;` — proof that
/// the param-driven hygiene naming (a declared name matching a `String`
/// param substitutes the literal value, not the type-arg suffix) lets a
/// blueprint faithfully wrap the `crud` primitive rather than merely
/// approximate it.
#[test]
fn std_crud_blueprint_is_byte_identical_to_hand_written_crud() {
    use ciac_codegen::GenOptions;
    use ciac_integration_tests::{compile_file, full_parity_backends, project_dump};
    use std::path::{Path, PathBuf};

    fn tmp(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ciac-std-crud-equivalence-{}-{}-{label}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    const RECORD: &str = "record Video { id: Uuid; title: String; }\n";
    const USE_DB: &str = "use { db Postgres; }\n";

    let hand_dir = tmp("hand");
    let hand_entry = write(
        &hand_dir,
        "entry.ciac",
        &format!("service S;\n{USE_DB}{RECORD}crud Videos: Video;\n"),
    );

    let std_dir = tmp("std");
    let std_entry = write(
        &std_dir,
        "entry.ciac",
        &format!(
            "service S;\n{USE_DB}{RECORD}import \"std/crud.ciac\";\nexpand Crud<Video> {{ name: \"Videos\"; }}\n"
        ),
    );

    let hand_ir = compile_file(&hand_entry);
    let std_ir = compile_file(&std_entry);

    for backend in full_parity_backends() {
        let hand_project = backend
            .generate(&hand_ir, &GenOptions::default())
            .unwrap_or_else(|err| panic!("{}: {err}", backend.id()));
        let std_project = backend
            .generate(&std_ir, &GenOptions::default())
            .unwrap_or_else(|err| panic!("{}: {err}", backend.id()));
        assert_eq!(
            project_dump(&hand_project),
            project_dump(&std_project),
            "{}: std.Crud must generate byte-identically to hand-written crud",
            backend.id()
        );
    }

    std::fs::remove_dir_all(&hand_dir).ok();
    std::fs::remove_dir_all(&std_dir).ok();
}

/// v0.14 M5: `BlueprintItem` grows to cover `record`/`table`/`api`/
/// `worker`/`pipeline`, enough for a body to declare a complete,
/// self-contained api-plus-pipeline shape (not just `crud`/`stream`/
/// `handler` as in v0.8). This exercises all five new item kinds in
/// one blueprint and proves the expansion generates cleanly on both
/// backends.
const SELF_CONTAINED_API: &str = r#"
service S;

use { db Postgres; queue NATS; }

blueprint Ingest<Payload: record> {
    params { path: String; }

    record Receipt { id: Uuid; }
    table Receipts: Receipt;
    stream Accepted: Payload;

    handler Accept(payload: Payload) -> Payload {
        db.insert(Receipts, Receipt { id: Uuid.new() });
        publish Accepted(payload);
        return payload;
    }

    api Endpoint: Payload {
        method: POST;
        path: path;
    }
    pipeline Endpoint:
        Accept
        -> Return;

    worker Drain on Accepted;
}
"#;

#[test]
fn blueprint_body_supports_record_table_api_worker_pipeline() {
    let src = format!(
        "{SELF_CONTAINED_API}\nrecord Order {{ id: Uuid; total: Int; }}\n\
         expand Ingest<Order> {{ path: \"/orders\"; }}\n"
    );
    let (ir, diags) = compile(&src);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("expands and type-checks");

    for backend in full_parity_backends() {
        backend
            .generate(&ir, &GenOptions::default())
            .unwrap_or_else(|err| panic!("{}: {err}", backend.id()));
    }
}

/// Two expansions of the same api-plus-pipeline blueprint must not
/// collide: each expansion's `pipeline` has to keep pointing at *its
/// own* (hygienically renamed) `api`, its own table, and its own
/// stream — the sharp edge the v0.14 plan called out by name.
#[test]
fn two_expansions_of_a_self_contained_api_blueprint_are_hygienic() {
    let src = format!(
        "{SELF_CONTAINED_API}\nrecord Order {{ id: Uuid; total: Int; }}\n\
         record Refund {{ id: Uuid; amount: Int; }}\n\
         expand Ingest<Order> {{ path: \"/orders\"; }}\n\
         expand Ingest<Refund> {{ path: \"/refunds\"; }}\n"
    );
    let (ir, diags) = compile(&src);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("expands and type-checks");

    for backend in full_parity_backends() {
        backend
            .generate(&ir, &GenOptions::default())
            .unwrap_or_else(|err| panic!("{}: {err}", backend.id()));
    }
}

/// v0.14 M5: a blueprint's `params` now reach into handler-body
/// expressions (not just attribute values and declaration-name
/// overrides) — the substituted literal has to actually change the
/// generated comparison, not just parse.
#[test]
fn blueprint_params_reach_handler_body_expressions() {
    const CAPPED: &str = r#"
service S;
blueprint Capped<Payload: record> {
    params { limit: Int; }
    handler CheckCap(payload: Payload) -> Bool {
        return 1 >= limit;
    }
    api Check: Payload;
    pipeline Check:
        CheckCap
        -> Return;
}
"#;
    let low =
        format!("{CAPPED}\nrecord Item {{ id: Uuid; }}\nexpand Capped<Item> {{ limit: 1; }}\n");
    let (ir, diags) = compile(&low);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("expands and type-checks");
    for backend in full_parity_backends() {
        backend
            .generate(&ir, &GenOptions::default())
            .unwrap_or_else(|err| panic!("{}: {err}", backend.id()));
    }
}

/// v0.14 M5 std library: a webhook receiver — `api` + `stream` +
/// `handler` that publishes the validated payload. Proof that the
/// blueprint actually generates real, buildable output on both
/// backends (there's no hand-written equivalent to diff against, so
/// this is the behavioral half of the "byte-equivalence-or-behavioral"
/// discipline `std/crud.ciac` established).
#[test]
fn std_webhook_blueprint_generates_on_both_backends() {
    use ciac_integration_tests::compile_file;
    use std::path::PathBuf;

    let dir = std::env::temp_dir().join(format!(
        "ciac-std-webhook-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry: PathBuf = dir.join("entry.ciac");
    std::fs::write(
        &entry,
        "service S;\nuse { queue NATS; }\nimport \"std/webhook.ciac\";\n\
         record Event { id: Uuid; kind: String; }\n\
         expand Webhook<Event> { path: \"/hooks/x\"; }\n",
    )
    .unwrap();

    let ir = compile_file(&entry);
    for backend in full_parity_backends() {
        backend
            .generate(&ir, &GenOptions::default())
            .unwrap_or_else(|err| panic!("{}: {err}", backend.id()));
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// v0.14 M5 std library: a request-capped api — table-backed quota
/// (`db.count`/`db.insert`) guarding a typed handler, with the cap
/// itself supplied per `expand` site via `params` reaching the
/// handler body (see `blueprint_params_reach_handler_body_expressions`).
#[test]
fn std_rate_limited_api_blueprint_generates_on_both_backends() {
    use ciac_integration_tests::compile_file;
    use std::path::PathBuf;

    let dir = std::env::temp_dir().join(format!(
        "ciac-std-rate-limited-api-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let entry: PathBuf = dir.join("entry.ciac");
    std::fs::write(
        &entry,
        "service S;\nuse { db Postgres; }\nimport \"std/rate-limited-api.ciac\";\n\
         record Comment { body: String; }\n\
         expand RateLimitedApi<Comment> { path: \"/comments\"; limit: 100; }\n",
    )
    .unwrap();

    let ir = compile_file(&entry);
    for backend in full_parity_backends() {
        backend
            .generate(&ir, &GenOptions::default())
            .unwrap_or_else(|err| panic!("{}: {err}", backend.id()));
    }
    std::fs::remove_dir_all(&dir).ok();
}
