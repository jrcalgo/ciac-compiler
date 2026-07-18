//! The shared HIR `Needs` scanner (v0.22 M3 — `22UpdatePlan.md` Pillar
//! 3, Part 1; the plan's own pre-agreed fallback scope, recorded in
//! that file's M3 milestone note: unify the scanner and keep
//! per-backend leaf/tail lowering, since that's where every
//! correctness-bearing duplication risk actually lives).
//!
//! Both backends' `lower.rs` used to carry their own, independently
//! hand-maintained `scan`/`scan_block`/`scan_expr` walking the same
//! HIR shape to answer the same question — "what does this handler
//! body touch?" — for different reasons (Python: precise verb
//! booleans for its generated behavioral test's mock assertions; Rust:
//! `db_get_tables`/`enums` for import lists and, since v0.17 M11,
//! `unguarded_verbs` for `ciac sim`'s capability-coverage refusal).
//! That's the one instance of duplicated walker logic this plan names
//! as an actual correctness risk, not just a style complaint: a
//! backend that forgets to push a verb onto its own `unguarded_verbs`
//! silently mis-scopes `ciac sim`'s own refusal. Computing the union
//! of both backends' needs in one traversal, once, removes that class
//! of bug structurally — a backend that doesn't read a given field
//! just doesn't read it; the field can no longer silently fall out of
//! sync with the walk, because there's only one walk.
//!
//! Moved verbatim into `lower/scan.rs` when the flat `lower.rs` became
//! a directory to hold Pillar 3's Parts 2-3 (`dispatch.rs`/
//! `host_syntax.rs`/`identity.rs`) alongside it — no logic here
//! changed as part of that move; see `lower/mod.rs`'s own doc comment
//! for the full picture.

use ciac_ir::{
    FieldType, HandlerBody, HirExpr, HirStmt, HirType, NormalizedIr, RecordId, TableId, Verb,
};
use heck::ToPascalCase;

/// Everything either bundled backend's `lower.rs` needs to know about a
/// handler body's params/return/statements, in one place. Not every
/// field matters to every target — that's expected, not a smell: this
/// is the union of two real backends' real needs, and a third backend
/// (or a fourth) is expected to read only the subset it cares about,
/// the same way Python already ignores `db_get_tables`/`enums`/
/// `unguarded_verbs` and Rust already ignores `db_insert`/`cache_get`/
/// `sa_query`/etc.
#[derive(Debug, Default)]
pub struct Needs {
    pub db: bool,
    pub cache: bool,
    /// Python-specific: whether `json.dumps`/`json.loads` needs
    /// importing (a scalar `cache.get`/`cache.set`/`object_store.get`
    /// payload goes through it; a record payload uses its own
    /// serializer instead — see the call sites below).
    pub json: bool,
    pub uuid: bool,
    pub datetime: bool,
    pub queue: bool,
    /// Occurrence count, not just presence (Python's generated
    /// behavioral test asserts the exact `session.add` call count — a
    /// `transaction` block may legitimately insert into more than one
    /// table, v0.16).
    pub db_insert: usize,
    pub db_get: bool,
    pub cache_get: bool,
    pub cache_set: bool,
    pub object_store_put: bool,
    pub object_store_get: bool,
    pub object_store_list: bool,
    /// A `db.query`/`db.count`/`db.delete_where` appears in the body
    /// (Python's behavioral test configures `session.execute`'s mock
    /// return shape only when this is set).
    pub sa_query: bool,
    pub sa_select: bool,
    pub sa_func: bool,
    pub sa_delete: bool,
    pub tables: Vec<TableId>,
    /// Rust-specific: tables read via `db.get`/`db.query` — the only
    /// verbs whose lowering spells the table's model type as a Rust
    /// type name (`sqlx::query_as::<_, Model>`); every other db verb
    /// binds by raw SQL and field name only, so importing the model
    /// type for them would be an unused import under `-D warnings`.
    pub db_get_tables: Vec<TableId>,
    pub records: Vec<RecordId>,
    /// Rust-specific: named enum types (`VideoStatus`) actually
    /// spelled out in the lowered body via an enum-literal comparison
    /// or record field value — deliberately not "every enum any
    /// referenced record has" (see `field_access_enum_name`'s own doc).
    pub enums: Vec<String>,
    /// Every verb this body calls that v0.17 M11's `SimWorld` does not
    /// fake (`db.insert`/broker `publish` excepted) — `ciac sim`'s own
    /// capability-coverage refusal reads this list.
    pub unguarded_verbs: Vec<&'static str>,
}

impl Needs {
    fn record(&mut self, id: RecordId) {
        if !self.records.contains(&id) {
            self.records.push(id);
        }
    }

    fn table(&mut self, id: TableId) {
        if !self.tables.contains(&id) {
            self.tables.push(id);
        }
    }

    fn db_get_table(&mut self, id: TableId) {
        if !self.db_get_tables.contains(&id) {
            self.db_get_tables.push(id);
        }
    }

    fn enum_name(&mut self, name: String) {
        if !self.enums.contains(&name) {
            self.enums.push(name);
        }
    }

    fn ty(&mut self, ty: &HirType) {
        match ty {
            HirType::Record(id) => self.record(*id),
            HirType::Option(inner) | HirType::List(inner) => self.ty(inner),
            _ => {}
        }
    }
}

/// Recovers a field access's *named* enum type (e.g. `VideoStatus` for
/// `inserted.status`) from the base's record type — the only place
/// this information exists, since [`HirType::Enum`] is structural (a
/// bare variant set), not nominal. `None` when `expr` isn't a field
/// access on a record, or the field isn't an enum. Rust-specific
/// consumption (a bare enum literal needs a named Rust type to attach
/// to); computed here regardless because it's part of the same
/// traversal, and Python simply never reads `Needs::enums`.
pub fn field_access_enum_name(ir: &NormalizedIr, expr: &HirExpr) -> Option<String> {
    let HirExpr::FieldAccess { base, field, .. } = expr else {
        return None;
    };
    let HirType::Record(id) = base.ty() else {
        return None;
    };
    let record = ir.record(id);
    let matched = record.fields.iter().find(|f| &f.name == field)?;
    matches!(matched.ty, FieldType::Enum { .. })
        .then(|| format!("{}{}", record.name, field.to_pascal_case()))
}

/// Scans a handler's params, return type, and body for everything
/// [`Needs`] tracks — the one traversal both backends' `render` now
/// call, in place of each carrying its own copy.
pub fn scan(ir: &NormalizedIr, body: &HandlerBody) -> Needs {
    let mut needs = Needs::default();
    for (_, ty) in &body.params {
        needs.ty(ty);
    }
    needs.ty(&body.return_ty);
    if let Some(stmts) = &body.body {
        scan_block(ir, stmts, &mut needs);
    }
    needs
}

fn scan_block(ir: &NormalizedIr, stmts: &[HirStmt], needs: &mut Needs) {
    for stmt in stmts {
        match stmt {
            HirStmt::Let { value, .. } | HirStmt::Expr(value) => scan_expr(ir, value, needs),
            HirStmt::Return(Some(value)) => scan_expr(ir, value, needs),
            HirStmt::Return(None) => {}
            HirStmt::Fail { error, args } => {
                needs.record(*error);
                for arg in args {
                    scan_expr(ir, arg, needs);
                }
            }
            HirStmt::Publish { value, .. } => {
                needs.queue = true;
                scan_expr(ir, value, needs);
            }
            HirStmt::Transaction { body } => scan_block(ir, body, needs),
        }
    }
}

fn scan_expr(ir: &NormalizedIr, expr: &HirExpr, needs: &mut Needs) {
    match expr {
        HirExpr::Local { ty, .. } => needs.ty(ty),
        HirExpr::FieldAccess { base, ty, .. } => {
            scan_expr(ir, base, needs);
            needs.ty(ty);
        }
        HirExpr::Index { base, index } => {
            scan_expr(ir, base, needs);
            scan_expr(ir, index, needs);
        }
        HirExpr::RecordCons {
            record,
            base_value,
            fields,
        } => {
            needs.record(*record);
            if let Some(base) = base_value {
                scan_expr(ir, base, needs);
            }
            for (name, value) in fields {
                if matches!(value, HirExpr::EnumLit { .. }) {
                    let enum_name = format!("{}{}", ir.record(*record).name, name.to_pascal_case());
                    needs.enum_name(enum_name);
                }
                scan_expr(ir, value, needs);
            }
        }
        HirExpr::Binary { lhs, rhs, ty, .. } => {
            if let Some(enum_name) = field_access_enum_name(ir, lhs) {
                if matches!(rhs.as_ref(), HirExpr::EnumLit { .. }) {
                    needs.enum_name(enum_name);
                }
            }
            scan_expr(ir, lhs, needs);
            scan_expr(ir, rhs, needs);
            needs.ty(ty);
        }
        HirExpr::Unary { expr, ty, .. } => {
            scan_expr(ir, expr, needs);
            needs.ty(ty);
        }
        HirExpr::If {
            cond,
            then_branch,
            else_branch,
            ty,
        } => {
            scan_expr(ir, cond, needs);
            scan_block(ir, then_branch, needs);
            scan_block(ir, else_branch, needs);
            needs.ty(ty);
        }
        HirExpr::Match {
            scrutinee,
            arms,
            ty,
        } => {
            if let Some(enum_name) = field_access_enum_name(ir, scrutinee) {
                needs.enum_name(enum_name);
            }
            scan_expr(ir, scrutinee, needs);
            for arm in arms {
                scan_block(ir, &arm.body, needs);
            }
            needs.ty(ty);
        }
        HirExpr::VerbCall { verb, args, ty, .. } => {
            match verb {
                Verb::DbInsert(table) => {
                    needs.db = true;
                    needs.db_insert += 1;
                    needs.table(*table);
                }
                Verb::DbGet(table) => {
                    needs.db = true;
                    needs.db_get = true;
                    needs.table(*table);
                    needs.db_get_table(*table);
                    needs.unguarded_verbs.push("db.get");
                }
                Verb::CacheGet => {
                    needs.cache = true;
                    needs.cache_get = true;
                    needs.json = true;
                    needs.unguarded_verbs.push("cache.get");
                }
                Verb::CacheSet => {
                    needs.cache = true;
                    needs.cache_set = true;
                    // Mirrors Python's own `json_encode`: a non-record
                    // value goes through `json.dumps`, a record through
                    // `model_dump_json` (no `json` import needed).
                    if !matches!(args[1].ty(), HirType::Record(_)) {
                        needs.json = true;
                    }
                    needs.unguarded_verbs.push("cache.set");
                }
                Verb::ObjectStoreGet => {
                    needs.object_store_get = true;
                    needs.json = true;
                    needs.unguarded_verbs.push("object_store.get");
                }
                Verb::ObjectStorePut => {
                    needs.object_store_put = true;
                    needs.unguarded_verbs.push("object_store.put");
                }
                Verb::DbUpdate(table) => {
                    needs.db = true;
                    needs.table(*table);
                    needs.unguarded_verbs.push("db.update");
                }
                Verb::DbDelete(table) => {
                    needs.db = true;
                    needs.table(*table);
                    needs.unguarded_verbs.push("db.delete");
                }
                Verb::CacheDelete => {
                    needs.cache = true;
                    needs.unguarded_verbs.push("cache.delete");
                }
                Verb::ObjectStoreList => {
                    needs.object_store_list = true;
                    needs.unguarded_verbs.push("object_store.list");
                }
                Verb::ObjectStoreDelete => needs.unguarded_verbs.push("object_store.delete"),
                Verb::EmailSend => needs.unguarded_verbs.push("email.send"),
                Verb::SearchIndex => needs.unguarded_verbs.push("search.index"),
                Verb::SearchQuery => needs.unguarded_verbs.push("search.query"),
                Verb::HttpCall => needs.unguarded_verbs.push("http.call"),
                Verb::DbQuery(_) | Verb::DbCount(_) | Verb::DbDeleteWhere(_) => {
                    unreachable!("typeck only ever constructs these via HirExpr::Query")
                }
            }
            for arg in args {
                scan_expr(ir, arg, needs);
            }
            needs.ty(ty);
        }
        HirExpr::Query {
            verb,
            predicate,
            ty,
            ..
        } => {
            needs.db = true;
            needs.sa_query = true;
            match verb {
                Verb::DbQuery(table) => {
                    needs.sa_select = true;
                    needs.table(*table);
                    needs.db_get_table(*table);
                    needs.unguarded_verbs.push("db.query");
                }
                Verb::DbCount(table) => {
                    needs.sa_select = true;
                    needs.sa_func = true;
                    needs.table(*table);
                    needs.unguarded_verbs.push("db.count");
                }
                Verb::DbDeleteWhere(table) => {
                    needs.sa_delete = true;
                    needs.table(*table);
                    needs.unguarded_verbs.push("db.delete_where");
                }
                _ => unreachable!("HirExpr::Query only ever carries a db query verb"),
            }
            if let Some(predicate) = predicate {
                for term in &predicate.terms {
                    scan_expr(ir, &term.value, needs);
                }
            }
            needs.ty(ty);
        }
        HirExpr::BuiltinCall(ciac_ir::Builtin::UuidNew) => needs.uuid = true,
        HirExpr::BuiltinCall(ciac_ir::Builtin::TimestampNow) => needs.datetime = true,
        HirExpr::IntLit(_)
        | HirExpr::FloatLit(_)
        | HirExpr::StrLit(_)
        | HirExpr::BoolLit(_)
        | HirExpr::EnumLit { .. } => {}
    }
}
