//! v0.7 M2: positive type-checking tests exercising the resulting HIR
//! directly (not just pass/fail), plus the one diagnostic that can't live
//! in `tests/ui/` because it's a warning, not an error
//! (`tests/tests/negative.rs` requires `ir.is_none()`).

use ciac_diagnostics::{ErrorCode, Severity};
use ciac_integration_tests::compile;
use ciac_ir::{Component, HirExpr, HirStmt, HirType, Verb};

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

fn find_handler_body<'a>(ir: &'a ciac_ir::NormalizedIr, name: &str) -> &'a ciac_ir::HandlerBody {
    ir.nodes()
        .find_map(|node| match &node.component {
            Component::Service {
                name: n,
                signature: Some(hir),
            } if n == name => Some(hir),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no type-checked handler named `{name}`"))
}

#[test]
fn canonical_example_produces_the_expected_hir_shape() {
    let (ir, diags) = compile(CANONICAL_EXAMPLE);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let body = find_handler_body(&ir, "StoreVideo");
    assert_eq!(body.params.len(), 1);
    assert_eq!(body.params[0].0, "v");
    assert!(matches!(body.return_ty, HirType::Record(_)));

    let stmts = body.body.as_ref().expect("inline body");
    assert_eq!(
        stmts.len(),
        6,
        "let key, object_store.put, let inserted, let ready, let described, return"
    );

    // `let key = "videos/" + v.id;` — Str + Uuid implicit stringify.
    let HirStmt::Let { value, .. } = &stmts[0] else {
        panic!("expected let key");
    };
    assert_eq!(value.ty(), HirType::Str);

    // `object_store.put(key, v);` is a bare statement, not a `let` — it
    // must be folded into the *previous* let's surrounding block? No:
    // it's its own statement between `let key` and `let inserted`, so
    // the statement count above already accounts for it. Reassert here
    // that a verb call resolves to a `VerbCall` HIR node somewhere in
    // the body.
    let has_object_store_put = stmts.iter().any(|s| {
        matches!(
            s,
            HirStmt::Expr(HirExpr::VerbCall {
                verb: Verb::ObjectStorePut,
                ..
            })
        )
    });
    assert!(
        has_object_store_put,
        "expected an ObjectStorePut verb call in {stmts:?}"
    );

    // `let inserted = db.insert(Videos, v);`
    let HirStmt::Let {
        value:
            HirExpr::VerbCall {
                verb: Verb::DbInsert(_),
                ty,
                ..
            },
        ..
    } = &stmts[2]
    else {
        panic!(
            "expected `let inserted = db.insert(..)`, got {:?}",
            stmts[2]
        );
    };
    assert!(matches!(ty, HirType::Record(_)));

    // `return described;`
    assert!(matches!(stmts.last(), Some(HirStmt::Return(Some(_)))));
}

#[test]
fn unused_let_binding_is_a_warning_not_an_error() {
    let source = r#"
service UnusedLetProbe;

record Video { id: Uuid; }

handler F(v: Video) -> Video {
    let unused = v.id;
    return v;
}
"#;
    let (ir, diags) = compile(source);
    assert!(
        !diags.has_errors(),
        "an unused let must not fail compilation: {:?}",
        diags.codes()
    );
    assert!(
        ir.is_some(),
        "well-typed program (warning aside) produces IR"
    );
    let unused_let = diags
        .iter()
        .find(|d| d.code == ErrorCode::UnusedLet)
        .unwrap_or_else(|| panic!("expected UnusedLet, got {:?}", diags.codes()));
    assert_eq!(unused_let.severity, Severity::Warning);
}
