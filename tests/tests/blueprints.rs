//! v0.8 M2: blueprint expansion (`ciac-sema/src/blueprints.rs`),
//! exercised through the same `ciac_sema::analyze` entry point real
//! programs use (blueprint expansion runs as `analyze`'s first step,
//! so there's nothing extra to wire up here).

use ciac_codegen::GenOptions;
use ciac_diagnostics::ErrorCode;
use ciac_integration_tests::{backends, compile};

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
    for backend in backends() {
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
