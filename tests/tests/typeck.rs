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

// v0.14 M1: the extended closed verb set — query/mutation verbs across
// every capability, list-typed results, and the `where` predicate
// grammar. Front-end only (typeck), per 14UpdatePlan.md's M1 scope; no
// codegen backend implements these yet.

const V014_M1_EXAMPLE: &str = r#"
service QueryExample;

use {
    db Postgres;
    cache Redis;
    object_store S3;
    email SES;
    search OpenSearch;
    external_http Generic { base_url: "https://api.example.com"; }
}

record Note {
    id: Uuid;
    title: String;
    active: Bool;
}

table Notes: Note;

handler ListActive() -> [Note] {
    return db.query(Notes) where active == true && title contains "x";
}

handler CountAll() -> Int {
    return db.count(Notes);
}

handler DeleteInactive() -> Int {
    return db.delete_where(Notes) where active == false;
}

handler UpdateOne(id: Uuid, n: Note) -> Note {
    let updated = db.update(Notes, id, n);
    let removed = db.delete(Notes, id);
    cache.delete("k");
    let listed = object_store.list("prefix/");
    object_store.delete("k");
    email.send("a@b.com", "hi", "body");
    search.index("doc1", n.id);
    let results = search.query("q");
    let resp = external_http.request("http://x", n.id);
    return n;
}

// Handlers only become graph nodes (and so are findable via
// `find_handler_body`) once a pipeline references them — mirrors every
// other typed-handler test/example in this file.
api ListActiveApi;
pipeline ListActiveApi: ListActive -> Return;

api CountAllApi;
pipeline CountAllApi: CountAll -> Return;

api DeleteInactiveApi;
pipeline DeleteInactiveApi: DeleteInactive -> Return;

api UpdateOneApi: Note;
pipeline UpdateOneApi: UpdateOne -> Return;
"#;

#[test]
fn v014_m1_extended_verb_set_type_checks() {
    let (ir, diags) = compile(V014_M1_EXAMPLE);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let list_active = find_handler_body(&ir, "ListActive");
    assert!(matches!(list_active.return_ty, HirType::List(_)));
    let stmts = list_active.body.as_ref().unwrap();
    let HirStmt::Return(Some(HirExpr::Query {
        verb: Verb::DbQuery(_),
        predicate: Some(pred),
        ty: HirType::List(_),
        ..
    })) = &stmts[0]
    else {
        panic!("expected a `db.query` with a predicate, got {stmts:?}");
    };
    assert_eq!(pred.terms.len(), 2);

    let count_all = find_handler_body(&ir, "CountAll");
    let stmts = count_all.body.as_ref().unwrap();
    assert!(matches!(
        &stmts[0],
        HirStmt::Return(Some(HirExpr::Query {
            verb: Verb::DbCount(_),
            predicate: None,
            ty: HirType::Int,
            ..
        }))
    ));

    let delete_inactive = find_handler_body(&ir, "DeleteInactive");
    let stmts = delete_inactive.body.as_ref().unwrap();
    assert!(matches!(
        &stmts[0],
        HirStmt::Return(Some(HirExpr::Query {
            verb: Verb::DbDeleteWhere(_),
            predicate: Some(_),
            ty: HirType::Int,
            ..
        }))
    ));

    let update_one = find_handler_body(&ir, "UpdateOne");
    let stmts = update_one.body.as_ref().unwrap();
    let verbs: Vec<Verb> = stmts
        .iter()
        .filter_map(|s| match s {
            HirStmt::Let {
                value: HirExpr::VerbCall { verb, .. },
                ..
            }
            | HirStmt::Expr(HirExpr::VerbCall { verb, .. }) => Some(*verb),
            _ => None,
        })
        .collect();
    assert!(matches!(verbs[0], Verb::DbUpdate(_)));
    assert!(matches!(verbs[1], Verb::DbDelete(_)));
    assert!(matches!(verbs[2], Verb::CacheDelete));
    assert!(matches!(verbs[3], Verb::ObjectStoreList));
    assert!(matches!(verbs[4], Verb::ObjectStoreDelete));
    assert!(matches!(verbs[5], Verb::EmailSend));
    assert!(matches!(verbs[6], Verb::SearchIndex));
    assert!(matches!(verbs[7], Verb::SearchQuery));
    assert!(matches!(verbs[8], Verb::HttpCall));
}

#[test]
fn where_clause_on_non_query_verb_is_ciac0052() {
    let source = r#"
service BadWhere;
use { cache Redis; }
handler F() -> Int {
    cache.get("k") where k == "x";
    return 0;
}
"#;
    let (ir, diags) = compile(source);
    assert!(ir.is_none());
    assert!(
        diags
            .iter()
            .any(|d| d.code == ErrorCode::QueryModifierNotSupported),
        "expected QueryModifierNotSupported, got {:?}",
        diags.codes()
    );
}

#[test]
fn list_type_as_record_field_is_ciac0053() {
    let source = r#"
service BadListField;
record Tagged { tags: [String]; }
"#;
    let (ir, diags) = compile(source);
    assert!(ir.is_none());
    assert_eq!(diags.codes(), vec![ErrorCode::UnsupportedFieldType]);
}
