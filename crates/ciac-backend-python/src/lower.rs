//! Direct lowering of the typed HIR (`ciac_ir::hir`) into Python source.
//!
//! These are pure string-producing functions with no template
//! involvement, per 07UpdatePlan.md's own direction: templates stay
//! presentational, and the HIR→target mapping lives in Rust code where
//! it can be matched exhaustively against the HIR's closed shape.
//!
//! Control-flow expressions (`if`/`match`) and `db.insert` don't fit a
//! single Python expression (they need statements: an `if`/`elif` chain,
//! or an `add`+`commit`+value sequence), so they're lowered wherever they
//! appear as a block's tail value — a `let` value, a `return` operand, or
//! a bare statement — via [`lower_tail`]. Everywhere else, [`py_expr`]
//! produces a single Python expression string; it panics if handed one of
//! those three shapes directly, which would mean they occurred nested
//! inside another expression — not a shape the language actually
//! produces (control flow is always block-shaped, never a sub-expression
//! of a binary operator or a call argument).

use ciac_codegen::model as context;
use ciac_ir::{
    BinOp, Builtin, HandlerBody, HirExpr, HirStmt, HirType, NodeId, NormalizedIr, RecordId,
    TableId, UnOp, Verb,
};
use heck::ToSnakeCase;
use serde::Serialize;

/// Everything referenced by a handler body that the emitting backend
/// needs to know about to write imports and a constructor: which
/// runtime pieces it touches, and which schema/model types it names.
#[derive(Debug, Default)]
pub struct Needs {
    pub db: bool,
    pub cache: bool,
    pub json: bool,
    pub uuid: bool,
    pub datetime: bool,
    pub queue: bool,
    /// Precise verb usage, for the behavioral test's mock assertions.
    pub db_insert: bool,
    pub db_get: bool,
    pub cache_get: bool,
    pub cache_set: bool,
    pub object_store_put: bool,
    pub object_store_get: bool,
    pub object_store_list: bool,
    /// A `db.query`/`db.count`/`db.delete_where` appears in the body
    /// (v0.14 M2) — the behavioral test configures `session.execute`'s
    /// mock return shape only when this is set. `select`/`func`/`delete`
    /// imports are tracked per-symbol below instead (only importing what
    /// a given handler actually spells, per `ruff`'s unused-import
    /// check).
    pub sa_query: bool,
    pub sa_select: bool,
    pub sa_func: bool,
    pub sa_delete: bool,
    pub tables: Vec<TableId>,
    pub records: Vec<RecordId>,
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

    fn ty(&mut self, ty: &HirType) {
        match ty {
            HirType::Record(id) => self.record(*id),
            HirType::Option(inner) | HirType::List(inner) => self.ty(inner),
            _ => {}
        }
    }
}

/// Scans a handler's params, return type, and body for everything
/// [`Needs`] tracks. A record import comes from a verb call's own
/// return/argument types (`needs.ty(ty)`, called at every verb-call
/// site below), not from merely touching a table:
/// `db.delete`/`db.count`/`db.delete_where` touch a table but return
/// `Bool`/`Int`, needing the table's *model* class (`model_imports`,
/// tracked by `needs.table`) but not its record schema.
pub fn scan(body: &HandlerBody) -> Needs {
    let mut needs = Needs::default();
    for (_, ty) in &body.params {
        needs.ty(ty);
    }
    needs.ty(&body.return_ty);
    if let Some(stmts) = &body.body {
        scan_block(stmts, &mut needs);
    }
    needs
}

fn scan_block(stmts: &[HirStmt], needs: &mut Needs) {
    for stmt in stmts {
        match stmt {
            HirStmt::Let { value, .. } | HirStmt::Expr(value) => scan_expr(value, needs),
            HirStmt::Return(Some(value)) => scan_expr(value, needs),
            HirStmt::Return(None) => {}
            HirStmt::Fail { error, args } => {
                needs.record(*error);
                for arg in args {
                    scan_expr(arg, needs);
                }
            }
            HirStmt::Publish { value, .. } => {
                needs.queue = true;
                scan_expr(value, needs);
            }
        }
    }
}

fn scan_expr(expr: &HirExpr, needs: &mut Needs) {
    match expr {
        HirExpr::Local { ty, .. } => needs.ty(ty),
        HirExpr::FieldAccess { base, ty, .. } => {
            scan_expr(base, needs);
            needs.ty(ty);
        }
        HirExpr::Index { base, index } => {
            scan_expr(base, needs);
            scan_expr(index, needs);
        }
        HirExpr::RecordCons {
            record,
            base_value,
            fields,
        } => {
            needs.record(*record);
            if let Some(base) = base_value {
                scan_expr(base, needs);
            }
            for (_, value) in fields {
                scan_expr(value, needs);
            }
        }
        HirExpr::Binary { lhs, rhs, ty, .. } => {
            scan_expr(lhs, needs);
            scan_expr(rhs, needs);
            needs.ty(ty);
        }
        HirExpr::Unary { expr, ty, .. } => {
            scan_expr(expr, needs);
            needs.ty(ty);
        }
        HirExpr::If {
            cond,
            then_branch,
            else_branch,
            ty,
        } => {
            scan_expr(cond, needs);
            scan_block(then_branch, needs);
            scan_block(else_branch, needs);
            needs.ty(ty);
        }
        HirExpr::Match {
            scrutinee,
            arms,
            ty,
        } => {
            scan_expr(scrutinee, needs);
            for arm in arms {
                scan_block(&arm.body, needs);
            }
            needs.ty(ty);
        }
        HirExpr::VerbCall { verb, args, ty, .. } => {
            match verb {
                Verb::DbInsert(table) => {
                    needs.db = true;
                    needs.db_insert = true;
                    needs.table(*table);
                }
                Verb::DbGet(table) => {
                    needs.db = true;
                    needs.db_get = true;
                    needs.table(*table);
                }
                Verb::CacheGet => {
                    needs.cache = true;
                    needs.cache_get = true;
                    needs.json = true;
                }
                Verb::CacheSet => {
                    needs.cache = true;
                    needs.cache_set = true;
                    // Mirrors `json_encode`'s own condition below: a
                    // non-record value goes through `json.dumps`, a
                    // record through `model_dump_json` (no `json`
                    // import needed) — get this wrong and it's either
                    // an unused import (record case) or a `NameError`
                    // at runtime (scalar case).
                    if !matches!(args[1].ty(), HirType::Record(_)) {
                        needs.json = true;
                    }
                }
                Verb::ObjectStoreGet => {
                    needs.object_store_get = true;
                    needs.json = true;
                }
                Verb::ObjectStorePut => needs.object_store_put = true,
                Verb::DbUpdate(table) | Verb::DbDelete(table) => {
                    needs.db = true;
                    needs.table(*table);
                }
                Verb::CacheDelete => needs.cache = true,
                Verb::ObjectStoreList => needs.object_store_list = true,
                Verb::ObjectStoreDelete => {}
                Verb::EmailSend | Verb::SearchIndex | Verb::SearchQuery | Verb::HttpCall => {}
                Verb::DbQuery(_) | Verb::DbCount(_) | Verb::DbDeleteWhere(_) => {
                    unreachable!("typeck only ever constructs these via HirExpr::Query")
                }
            }
            for arg in args {
                scan_expr(arg, needs);
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
                }
                Verb::DbCount(table) => {
                    needs.sa_select = true;
                    needs.sa_func = true;
                    needs.table(*table);
                }
                Verb::DbDeleteWhere(table) => {
                    needs.sa_delete = true;
                    needs.table(*table);
                }
                _ => unreachable!("HirExpr::Query only ever carries a db query verb"),
            }
            if let Some(predicate) = predicate {
                for term in &predicate.terms {
                    scan_expr(&term.value, needs);
                }
            }
            needs.ty(ty);
        }
        HirExpr::BuiltinCall(Builtin::UuidNew) => needs.uuid = true,
        HirExpr::BuiltinCall(Builtin::TimestampNow) => needs.datetime = true,
        HirExpr::IntLit(_)
        | HirExpr::FloatLit(_)
        | HirExpr::StrLit(_)
        | HirExpr::BoolLit(_)
        | HirExpr::EnumLit { .. } => {}
    }
}

/// Python type annotation for a HIR type. Record types need
/// `from app.schemas import <name>` at the call site — see [`Needs`].
pub fn py_type(ir: &NormalizedIr, ty: &HirType) -> String {
    match ty {
        HirType::Str | HirType::Uuid => "str".to_owned(),
        HirType::Int => "int".to_owned(),
        HirType::Float => "float".to_owned(),
        HirType::Bool => "bool".to_owned(),
        HirType::Timestamp => "datetime".to_owned(),
        HirType::Json => "dict[str, Any]".to_owned(),
        HirType::Enum { variants } => format!(
            "Literal[{}]",
            variants
                .iter()
                .map(|v| format!("{v:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        HirType::Record(id) => ir.record(*id).name.clone(),
        HirType::Option(inner) => format!("{} | None", py_type(ir, inner)),
        HirType::List(inner) => format!("list[{}]", py_type(ir, inner)),
        HirType::Unit | HirType::Never => "None".to_owned(),
    }
}

fn slot_name(body: &HandlerBody, slot: u32) -> String {
    let slot = slot as usize;
    if slot < body.params.len() {
        body.params[slot].0.clone()
    } else {
        format!("v{slot}")
    }
}

fn model_class_name(ir: &NormalizedIr, table: TableId) -> String {
    ir.table(table).name.clone()
}

fn record_class_name(ir: &NormalizedIr, record: RecordId) -> String {
    ir.record(record).name.clone()
}

fn stream_subject(ir: &NormalizedIr, stream: NodeId) -> String {
    match &ir.node(stream).component {
        ciac_ir::Component::Stream { subject, .. } => subject.clone(),
        other => unreachable!("publish target is a stream, found {other:?}"),
    }
}

/// A single Python expression for `value`, JSON-encoded (records use
/// their compact Pydantic serializer; everything else round-trips
/// through `json.dumps`).
fn json_encode(ir: &NormalizedIr, body: &HandlerBody, value: &HirExpr) -> String {
    if matches!(value.ty(), HirType::Record(_)) {
        format!("{}.model_dump_json()", py_expr(ir, body, value))
    } else {
        format!("json.dumps({})", py_expr(ir, body, value))
    }
}

/// OpenSearch has no per-document collection concept in the language
/// yet (v0.14 M3/M4) — every `search.index`/`search.query` call on a
/// given instance shares this one index name.
const SEARCH_INDEX_NAME: &str = "documents";

/// A Python *value* (not a JSON string — for `search.index`'s
/// `document` and `external_http.request`'s `json=` body, both of
/// which take a real dict) for `value`, which `search.index`/
/// `external_http.request` accept as any type per their closed verb
/// signature (mirroring `cache.set`/`object_store.put`'s untyped
/// payload): records become their field dict, `Json` values pass
/// through as-is, and anything else (a bare string/number/bool) is
/// wrapped so the body is always a JSON object.
fn json_body(ir: &NormalizedIr, body: &HandlerBody, value: &HirExpr) -> String {
    match value.ty() {
        HirType::Record(_) => format!("{}.model_dump(mode=\"json\")", py_expr(ir, body, value)),
        HirType::Json => py_expr(ir, body, value),
        _ => format!("{{\"value\": {}}}", py_expr(ir, body, value)),
    }
}

/// Lowers a single expression to a Python expression string. Panics on
/// `If`/`Match`/`db.insert` — see the module doc comment.
pub fn py_expr(ir: &NormalizedIr, body: &HandlerBody, expr: &HirExpr) -> String {
    match expr {
        HirExpr::If { .. } | HirExpr::Match { .. } => {
            unreachable!("control flow must be lowered via lower_tail, not nested in an expression")
        }
        HirExpr::VerbCall {
            verb: Verb::DbInsert(_) | Verb::DbUpdate(_) | Verb::DbDelete(_),
            ..
        } => unreachable!(
            "db.insert/update/delete must be lowered via lower_tail, not nested in an expression"
        ),
        HirExpr::Query { .. } => {
            unreachable!("db.query/count/delete_where must be lowered via lower_tail, not nested in an expression")
        }
        HirExpr::Local { slot, .. } => slot_name(body, *slot),
        HirExpr::IntLit(n) => n.to_string(),
        HirExpr::FloatLit(f) => format!("{f}"),
        HirExpr::StrLit(s) => format!("{s:?}"),
        HirExpr::BoolLit(b) => if *b { "True" } else { "False" }.to_owned(),
        HirExpr::BuiltinCall(Builtin::UuidNew) => "str(uuid4())".to_owned(),
        HirExpr::BuiltinCall(Builtin::TimestampNow) => "datetime.now(timezone.utc)".to_owned(),
        HirExpr::EnumLit { variant, .. } => format!("{variant:?}"),
        HirExpr::FieldAccess { base, field, .. } => {
            format!("{}.{field}", py_expr(ir, body, base))
        }
        HirExpr::Index { base, index } => {
            format!("{}[{}]", py_expr(ir, body, base), py_expr(ir, body, index))
        }
        HirExpr::RecordCons {
            record,
            base_value,
            fields,
        } => match base_value {
            None => {
                let args = fields
                    .iter()
                    .map(|(name, value)| format!("{name}={}", py_expr(ir, body, value)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{}({args})", record_class_name(ir, *record))
            }
            Some(base) => {
                let updates = fields
                    .iter()
                    .map(|(name, value)| format!("{name:?}: {}", py_expr(ir, body, value)))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{}.model_copy(update={{{updates}}})",
                    py_expr(ir, body, base)
                )
            }
        },
        HirExpr::Binary { op, lhs, rhs, .. } => py_binary(ir, body, *op, lhs, rhs),
        HirExpr::Unary { op, expr, .. } => {
            let inner = py_expr(ir, body, expr);
            match op {
                UnOp::Neg => format!("(-{inner})"),
                UnOp::Not => format!("(not {inner})"),
            }
        }
        HirExpr::VerbCall { verb, args, .. } => py_verb_expr(ir, body, *verb, args),
    }
}

fn py_binary(
    ir: &NormalizedIr,
    body: &HandlerBody,
    op: BinOp,
    lhs: &HirExpr,
    rhs: &HirExpr,
) -> String {
    let lhs_ty = lhs.ty();
    let rhs_ty = rhs.ty();
    let lhs_s = py_expr(ir, body, lhs);
    let rhs_s = py_expr(ir, body, rhs);
    if op == BinOp::Add {
        return if lhs_ty == HirType::Str && rhs_ty != HirType::Str {
            format!("({lhs_s} + str({rhs_s}))")
        } else if rhs_ty == HirType::Str && lhs_ty != HirType::Str {
            format!("(str({lhs_s}) + {rhs_s})")
        } else {
            format!("({lhs_s} + {rhs_s})")
        };
    }
    let py_op = match op {
        BinOp::Add => unreachable!("handled above"),
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Eq => "==",
        BinOp::NotEq => "!=",
        BinOp::Lt => "<",
        BinOp::LtEq => "<=",
        BinOp::Gt => ">",
        BinOp::GtEq => ">=",
        BinOp::And => "and",
        BinOp::Or => "or",
    };
    format!("({lhs_s} {py_op} {rhs_s})")
}

/// Lowers every verb call that fits a single Python expression —
/// everything except `db.insert` (statement-shaped; see [`lower_tail`]).
fn py_verb_expr(ir: &NormalizedIr, body: &HandlerBody, verb: Verb, args: &[HirExpr]) -> String {
    match verb {
        Verb::DbInsert(_) | Verb::DbUpdate(_) | Verb::DbDelete(_) => {
            unreachable!("db.insert/update/delete must be lowered via lower_tail")
        }
        Verb::DbGet(table) => {
            let model = model_class_name(ir, table);
            let record = record_class_name(ir, ir.table(table).record);
            let key = py_expr(ir, body, &args[0]);
            format!(
                "({record}.model_validate(_row, from_attributes=True) \
                 if (_row := await self.session.get({model}, str({key}))) is not None else None)"
            )
        }
        Verb::CacheGet => {
            let key = py_expr(ir, body, &args[0]);
            format!(
                "(json.loads(_cv) if (_cv := await self.cache.get({key})) is not None else None)"
            )
        }
        Verb::CacheSet => {
            let key = py_expr(ir, body, &args[0]);
            let value = json_encode(ir, body, &args[1]);
            format!("(await self.cache.set({key}, {value}))")
        }
        Verb::ObjectStorePut => {
            let key = py_expr(ir, body, &args[0]);
            let payload = if matches!(args[1].ty(), HirType::Record(_)) {
                format!("{}.model_dump_json().encode()", py_expr(ir, body, &args[1]))
            } else {
                format!("str({}).encode()", py_expr(ir, body, &args[1]))
            };
            format!("(await self.object_store.put({key}, {payload}))")
        }
        Verb::ObjectStoreGet => {
            let key = py_expr(ir, body, &args[0]);
            format!("json.loads(await self.object_store.get({key}))")
        }
        Verb::CacheDelete => {
            let key = py_expr(ir, body, &args[0]);
            format!("(await self.cache.delete({key}))")
        }
        Verb::ObjectStoreDelete => {
            let key = py_expr(ir, body, &args[0]);
            format!("(await self.object_store.delete({key}))")
        }
        Verb::ObjectStoreList => {
            let prefix = py_expr(ir, body, &args[0]);
            format!("(await self.object_store.list({prefix}))")
        }
        Verb::EmailSend => {
            let to = py_expr(ir, body, &args[0]);
            let subject = py_expr(ir, body, &args[1]);
            let body_arg = py_expr(ir, body, &args[2]);
            format!("(await self.email.send({to}, {subject}, {body_arg}))")
        }
        Verb::SearchIndex => {
            let doc_id = py_expr(ir, body, &args[0]);
            let document = json_body(ir, body, &args[1]);
            format!("(await self.search.index({SEARCH_INDEX_NAME:?}, {doc_id}, {document}))")
        }
        Verb::SearchQuery => {
            let query = py_expr(ir, body, &args[0]);
            format!(
                "(await self.search.search({SEARCH_INDEX_NAME:?}, {{\"query\": {{\"query_string\": {{\"query\": {query}}}}}}}))"
            )
        }
        Verb::HttpCall => {
            let url = py_expr(ir, body, &args[0]);
            let json_arg = json_body(ir, body, &args[1]);
            format!("(await self.http.post({url}, json={json_arg})).json()")
        }
        Verb::DbQuery(_) | Verb::DbCount(_) | Verb::DbDeleteWhere(_) => {
            unreachable!("typeck only ever constructs these via HirExpr::Query")
        }
    }
}

/// Where a block's tail value goes: assigned to a `let`-bound name,
/// `return`ed, or discarded (a bare statement-position `if`/`match`).
enum Sink {
    Assign(String),
    Return,
    Discard,
}

fn apply_sink(sink: &Sink, value: &str, indent: &str, out: &mut Vec<String>) {
    match sink {
        Sink::Assign(name) => out.push(format!("{indent}{name} = {value}")),
        Sink::Return => out.push(format!("{indent}return {value}")),
        Sink::Discard => out.push(format!("{indent}{value}")),
    }
}

/// Renders the `.where(..)` chain for a `db.query`/`count`/`delete_where`
/// predicate (v0.14 M2) — one `.where(Model.field <op> value)` per term,
/// SQLAlchemy ANDs chained `.where()` calls together. Empty string (no
/// filtering) when there's no `where` clause.
fn py_where_chain(
    ir: &NormalizedIr,
    body: &HandlerBody,
    model: &str,
    predicate: &Option<ciac_ir::HirPredicate>,
) -> String {
    let Some(predicate) = predicate else {
        return String::new();
    };
    predicate
        .terms
        .iter()
        .map(|term| {
            let field = &term.field;
            // `field == True`/`field == False` is `ruff`'s E712: a bare
            // (or negated) column already reads as a boolean filter in
            // SQLAlchemy, same as Python's own preference for `if x:`
            // over `if x == True:`.
            match (term.op, &term.value) {
                (ciac_ir::PredOp::Eq, HirExpr::BoolLit(true))
                | (ciac_ir::PredOp::NotEq, HirExpr::BoolLit(false)) => {
                    format!(".where({model}.{field})")
                }
                (ciac_ir::PredOp::Eq, HirExpr::BoolLit(false))
                | (ciac_ir::PredOp::NotEq, HirExpr::BoolLit(true)) => {
                    format!(".where(~{model}.{field})")
                }
                (op, value) => {
                    let value = py_expr(ir, body, value);
                    match op {
                        ciac_ir::PredOp::Eq => format!(".where({model}.{field} == {value})"),
                        ciac_ir::PredOp::NotEq => format!(".where({model}.{field} != {value})"),
                        ciac_ir::PredOp::Lt => format!(".where({model}.{field} < {value})"),
                        ciac_ir::PredOp::LtEq => format!(".where({model}.{field} <= {value})"),
                        ciac_ir::PredOp::Gt => format!(".where({model}.{field} > {value})"),
                        ciac_ir::PredOp::GtEq => format!(".where({model}.{field} >= {value})"),
                        ciac_ir::PredOp::Contains => {
                            format!(".where({model}.{field}.contains({value}))")
                        }
                    }
                }
            }
        })
        .collect()
}

/// Lowers a full handler body into indented Python source lines, each
/// already prefixed with `indent` plus whatever extra nesting its own
/// control flow needs.
pub fn lower_body(ir: &NormalizedIr, body: &HandlerBody, indent: &str) -> Vec<String> {
    let mut out = Vec::new();
    let stmts = body.body.as_deref().unwrap_or(&[]);
    lower_block(ir, body, stmts, indent, &Sink::Discard, &mut out);
    if out.is_empty() {
        out.push(format!("{indent}pass"));
    }
    out
}

fn lower_block(
    ir: &NormalizedIr,
    body: &HandlerBody,
    stmts: &[HirStmt],
    indent: &str,
    sink: &Sink,
    out: &mut Vec<String>,
) {
    if stmts.is_empty() {
        out.push(format!("{indent}pass"));
        return;
    }
    for (i, stmt) in stmts.iter().enumerate() {
        let is_last = i + 1 == stmts.len();
        if is_last {
            if let HirStmt::Expr(e) = stmt {
                if e.ty() != HirType::Never {
                    lower_tail(ir, body, e, indent, sink, out);
                    continue;
                }
            }
        }
        lower_stmt(ir, body, stmt, indent, out);
        // The type checker still type-checks (and the HIR still
        // contains) statements after one that diverges — e.g. a
        // `return described;` following a `let described = match { ..
        // every arm returns/fails .. };` — since it never actually
        // assigns `described`, lowering it would reference a name that
        // was never bound. Those statements are unreachable at runtime
        // either way, so stop emitting once a statement diverges.
        if stmt_diverges(stmt) {
            return;
        }
    }
}

fn stmt_diverges(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Return(_) | HirStmt::Fail { .. } => true,
        HirStmt::Let { value, .. } | HirStmt::Expr(value) => value.ty() == HirType::Never,
        HirStmt::Publish { .. } => false,
    }
}

/// Lowers an expression that's the tail value of a block — the value of
/// a `let`, the operand of a `return`, or a bare statement-position
/// expression — into `sink`. Recurses through `if`/`match` so a
/// diverging branch's own tail lands in the same `sink`.
fn lower_tail(
    ir: &NormalizedIr,
    body: &HandlerBody,
    expr: &HirExpr,
    indent: &str,
    sink: &Sink,
    out: &mut Vec<String>,
) {
    match expr {
        HirExpr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            out.push(format!("{indent}if {}:", py_expr(ir, body, cond)));
            let inner = format!("{indent}    ");
            lower_block(ir, body, then_branch, &inner, sink, out);
            out.push(format!("{indent}else:"));
            lower_block(ir, body, else_branch, &inner, sink, out);
        }
        HirExpr::Match {
            scrutinee, arms, ..
        } => {
            let scrut = py_expr(ir, body, scrutinee);
            let inner = format!("{indent}    ");
            for (i, arm) in arms.iter().enumerate() {
                let cond = match &arm.variant {
                    Some(v) => format!("{scrut} == {v:?}"),
                    None => "True".to_owned(),
                };
                let kw = if i == 0 { "if" } else { "elif" };
                out.push(format!("{indent}{kw} {cond}:"));
                lower_block(ir, body, &arm.body, &inner, sink, out);
            }
        }
        HirExpr::VerbCall {
            verb: Verb::DbInsert(table),
            args,
            ..
        } => {
            let model = model_class_name(ir, *table);
            let value = py_expr(ir, body, &args[0]);
            out.push(format!(
                "{indent}self.session.add({model}(**{value}.model_dump()))"
            ));
            out.push(format!("{indent}await self.session.commit()"));
            match sink {
                Sink::Assign(name) => out.push(format!("{indent}{name} = {value}")),
                Sink::Return => out.push(format!("{indent}return {value}")),
                Sink::Discard => {}
            }
        }
        HirExpr::VerbCall {
            verb: Verb::DbUpdate(table),
            args,
            ..
        } => {
            let model = model_class_name(ir, *table);
            let record = record_class_name(ir, ir.table(*table).record);
            let key = py_expr(ir, body, &args[0]);
            let value = py_expr(ir, body, &args[1]);
            out.push(format!(
                "{indent}_row = await self.session.get({model}, str({key}))"
            ));
            out.push(format!("{indent}if _row is not None:"));
            out.push(format!(
                "{indent}    for _k, _v in {value}.model_dump().items():"
            ));
            out.push(format!("{indent}        setattr(_row, _k, _v)"));
            out.push(format!("{indent}    await self.session.commit()"));
            apply_sink(
                sink,
                &format!("{record}.model_validate(_row, from_attributes=True)"),
                &format!("{indent}    "),
                out,
            );
            out.push(format!("{indent}else:"));
            apply_sink(sink, "None", &format!("{indent}    "), out);
        }
        HirExpr::VerbCall {
            verb: Verb::DbDelete(table),
            args,
            ..
        } => {
            let model = model_class_name(ir, *table);
            let key = py_expr(ir, body, &args[0]);
            out.push(format!(
                "{indent}_row = await self.session.get({model}, str({key}))"
            ));
            out.push(format!("{indent}if _row is not None:"));
            out.push(format!("{indent}    await self.session.delete(_row)"));
            out.push(format!("{indent}    await self.session.commit()"));
            apply_sink(sink, "True", &format!("{indent}    "), out);
            out.push(format!("{indent}else:"));
            apply_sink(sink, "False", &format!("{indent}    "), out);
        }
        HirExpr::Query {
            verb: Verb::DbQuery(table),
            predicate,
            ..
        } => {
            let model = model_class_name(ir, *table);
            let record = record_class_name(ir, ir.table(*table).record);
            let where_chain = py_where_chain(ir, body, &model, predicate);
            out.push(format!("{indent}_stmt = select({model}){where_chain}"));
            out.push(format!(
                "{indent}_rows = (await self.session.execute(_stmt)).scalars().all()"
            ));
            apply_sink(
                sink,
                &format!("[{record}.model_validate(_r, from_attributes=True) for _r in _rows]"),
                indent,
                out,
            );
        }
        HirExpr::Query {
            verb: Verb::DbCount(table),
            predicate,
            ..
        } => {
            let model = model_class_name(ir, *table);
            let where_chain = py_where_chain(ir, body, &model, predicate);
            out.push(format!(
                "{indent}_stmt = select(func.count()).select_from({model}){where_chain}"
            ));
            apply_sink(
                sink,
                "(await self.session.execute(_stmt)).scalar_one()",
                indent,
                out,
            );
        }
        HirExpr::Query {
            verb: Verb::DbDeleteWhere(table),
            predicate,
            ..
        } => {
            let model = model_class_name(ir, *table);
            let where_chain = py_where_chain(ir, body, &model, predicate);
            out.push(format!("{indent}_stmt = sql_delete({model}){where_chain}"));
            out.push(format!(
                "{indent}_result = await self.session.execute(_stmt)"
            ));
            out.push(format!("{indent}await self.session.commit()"));
            apply_sink(sink, "_result.rowcount", indent, out);
        }
        HirExpr::Query { verb, .. } => {
            unreachable!("HirExpr::Query only ever carries a db query verb, found {verb:?}")
        }
        _ => {
            let e = py_expr(ir, body, expr);
            apply_sink(sink, &e, indent, out);
        }
    }
}

fn lower_stmt(
    ir: &NormalizedIr,
    body: &HandlerBody,
    stmt: &HirStmt,
    indent: &str,
    out: &mut Vec<String>,
) {
    match stmt {
        HirStmt::Let { slot, value } => {
            let name = slot_name(body, *slot);
            lower_tail(ir, body, value, indent, &Sink::Assign(name), out);
        }
        HirStmt::Expr(e) => lower_tail(ir, body, e, indent, &Sink::Discard, out),
        HirStmt::Return(None) => out.push(format!("{indent}return")),
        HirStmt::Return(Some(e)) => lower_tail(ir, body, e, indent, &Sink::Return, out),
        HirStmt::Fail { error, args } => {
            let exc = record_class_name(ir, *error);
            let arg_strs: Vec<String> = args.iter().map(|a| py_expr(ir, body, a)).collect();
            out.push(format!("{indent}raise {exc}({})", arg_strs.join(", ")));
        }
        HirStmt::Publish { stream, value } => {
            let subject = stream_subject(ir, *stream);
            let payload = if matches!(value.ty(), HirType::Record(_)) {
                format!("{}.model_dump(mode=\"json\")", py_expr(ir, body, value))
            } else {
                py_expr(ir, body, value)
            };
            out.push(format!("{indent}await publish({subject:?}, {payload})"));
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ParamCtx {
    pub name: String,
    pub ty: String,
}

/// Everything `logic.py.j2` needs to render one typed handler's file —
/// inline (compiler-owned, `app/logic/<module>.py`) or `extern` (seeded,
/// `app/services/<module>.py`, `body` is just a `NotImplementedError`).
#[derive(Debug, Serialize)]
pub struct LogicFileCtx {
    pub class_name: String,
    pub module: String,
    pub is_extern: bool,
    pub params: Vec<ParamCtx>,
    pub return_type: String,
    pub needs_db: bool,
    pub needs_cache: bool,
    pub needs_queue: bool,
    pub needs_json: bool,
    pub needs_uuid: bool,
    pub needs_datetime: bool,
    /// `select`/`func`/`delete as sql_delete`, filtered to what this
    /// handler actually uses — see `Needs::sa_select`/`sa_func`/
    /// `sa_delete`.
    pub sa_query_imports: Vec<String>,
    pub extras: Vec<context::ExtraDepCtx>,
    pub schema_imports: Vec<String>,
    pub model_imports: Vec<String>,
    pub body: Vec<String>,
}

/// Builds the render context for one typed handler node. `name` is the
/// handler's declared name (`node.component.name()`).
pub fn render(ir: &NormalizedIr, name: &str, hir: &HandlerBody) -> LogicFileCtx {
    let needs = scan(hir);
    let bindings = context::hir_bindings(ir, hir);
    let access = context::access_of(&bindings);
    let extras = context::extras_of(&bindings);

    let mut schema_imports: Vec<String> = needs
        .records
        .iter()
        .map(|id| record_class_name(ir, *id))
        .collect();
    schema_imports.sort();
    let mut model_imports: Vec<String> = needs
        .tables
        .iter()
        .map(|id| model_class_name(ir, *id))
        .collect();
    model_imports.sort();

    let params = hir
        .params
        .iter()
        .map(|(n, ty)| ParamCtx {
            name: n.clone(),
            ty: py_type(ir, ty),
        })
        .collect();

    let body = match &hir.body {
        Some(_) => lower_body(ir, hir, "        "),
        None => vec!["        raise NotImplementedError".to_owned()],
    };

    let mut sa_query_imports = Vec::new();
    if needs.sa_select {
        sa_query_imports.push("select".to_owned());
    }
    if needs.sa_func {
        sa_query_imports.push("func".to_owned());
    }
    if needs.sa_delete {
        sa_query_imports.push("delete as sql_delete".to_owned());
    }

    LogicFileCtx {
        class_name: name.to_owned(),
        module: name.to_snake_case(),
        is_extern: hir.body.is_none(),
        params,
        return_type: py_type(ir, &hir.return_ty),
        needs_db: access.db.is_some(),
        needs_cache: access.cache_expr.is_some(),
        needs_queue: needs.queue,
        needs_json: needs.json,
        needs_uuid: needs.uuid,
        needs_datetime: needs.datetime,
        sa_query_imports,
        extras,
        schema_imports,
        model_imports,
        body,
    }
}

/// Records the generated behavioral test actually spells by name — only
/// a bare `Record` (both `dummy_value` and `assert_result` reference the
/// class name directly). `Option<Record>`/`List<Record>` deliberately
/// don't recurse: their dummy values (`None`/`[]`) and assertions
/// (`assert_result` skips `Option`; the `List` case only checks
/// `isinstance(result, list)`) never spell the inner record's name, so
/// importing it would be an unused import (`ruff` F401).
fn collect_record_ids(ty: &HirType, out: &mut Vec<RecordId>) {
    if let HirType::Record(id) = ty {
        if !out.contains(id) {
            out.push(*id);
        }
    }
}

fn dummy_field(ty: &ciac_ir::FieldType) -> String {
    use ciac_ir::FieldType;
    match ty {
        FieldType::Str => "\"test\"".to_owned(),
        FieldType::Int => "0".to_owned(),
        FieldType::Float => "0.0".to_owned(),
        FieldType::Bool => "True".to_owned(),
        FieldType::Uuid => "str(uuid4())".to_owned(),
        FieldType::Timestamp => "datetime.now(timezone.utc)".to_owned(),
        FieldType::Json => "{}".to_owned(),
        FieldType::Enum { variants } => {
            format!("{:?}", variants.first().cloned().unwrap_or_default())
        }
    }
}

/// A throwaway Python literal of type `ty`, for the generated behavioral
/// test's payload — not a fixture generator, just enough to construct a
/// well-typed argument for `handle()`.
fn dummy_value(ir: &NormalizedIr, ty: &HirType) -> String {
    match ty {
        HirType::Str => "\"test\"".to_owned(),
        HirType::Int => "0".to_owned(),
        HirType::Float => "0.0".to_owned(),
        HirType::Bool => "True".to_owned(),
        HirType::Uuid => "str(uuid4())".to_owned(),
        HirType::Timestamp => "datetime.now(timezone.utc)".to_owned(),
        HirType::Json => "{}".to_owned(),
        HirType::Enum { variants } => {
            format!("{:?}", variants.first().cloned().unwrap_or_default())
        }
        HirType::Record(id) => {
            let record = ir.record(*id);
            let args = record
                .fields
                .iter()
                .map(|f| format!("{}={}", f.name, dummy_field(&f.ty)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{}({args})", record.name)
        }
        HirType::List(_) => "[]".to_owned(),
        HirType::Option(_) | HirType::Unit | HirType::Never => "None".to_owned(),
    }
}

fn dummy_value_needs(ir: &NormalizedIr, ty: &HirType, uuid: &mut bool, datetime: &mut bool) {
    use ciac_ir::FieldType;
    match ty {
        HirType::Uuid => *uuid = true,
        HirType::Timestamp => *datetime = true,
        HirType::Record(id) => {
            for field in &ir.record(*id).fields {
                match field.ty {
                    FieldType::Uuid => *uuid = true,
                    FieldType::Timestamp => *datetime = true,
                    _ => {}
                }
            }
        }
        HirType::Option(inner) | HirType::List(inner) => {
            dummy_value_needs(ir, inner, uuid, datetime)
        }
        _ => {}
    }
}

fn assert_result(ir: &NormalizedIr, ty: &HirType) -> Option<String> {
    match ty {
        HirType::Record(id) => Some(format!(
            "    assert isinstance(result, {})",
            record_class_name(ir, *id)
        )),
        HirType::Str => Some("    assert isinstance(result, str)".to_owned()),
        HirType::Int => Some("    assert isinstance(result, int)".to_owned()),
        HirType::Float => Some("    assert isinstance(result, float)".to_owned()),
        HirType::Bool => Some("    assert isinstance(result, bool)".to_owned()),
        HirType::List(_) => Some("    assert isinstance(result, list)".to_owned()),
        HirType::Uuid | HirType::Timestamp | HirType::Json | HirType::Enum { .. } => None,
        HirType::Option(_) | HirType::Unit | HirType::Never => None,
    }
}

/// A generated `pytest` exercising one inline handler's lowered body
/// against mocked runtime dependencies: proves the lowering calls the
/// right runtime APIs. `None` for `extern` handlers (no body to test).
pub fn render_test(ir: &NormalizedIr, hir: &HandlerBody, ctx: &LogicFileCtx) -> Option<String> {
    hir.body.as_ref()?;
    let needs = scan(hir);
    let mut lines = Vec::new();
    lines.push(format!(
        "\"\"\"Generated behavioral test for `{}`. Regenerated on every build.\n\nExercises the lowered handler body against mocked runtime dependencies;\nreal persistence round-trips are `ciac verify --live`'s job, not this test.\n\"\"\"",
        ctx.class_name
    ));
    lines.push("import pytest".to_owned());
    // Only import what the mocked-dependency setup below actually uses —
    // a handler with no capability calls (e.g. a pure transform) needs
    // neither, and an unused import fails `ruff check`.
    let needs_async_mock =
        ctx.needs_db || ctx.needs_cache || ctx.needs_queue || !ctx.extras.is_empty();
    let needs_magic_mock = ctx.needs_db;
    match (needs_async_mock, needs_magic_mock) {
        (true, true) => lines.push("from unittest.mock import AsyncMock, MagicMock".to_owned()),
        (true, false) => lines.push("from unittest.mock import AsyncMock".to_owned()),
        (false, true) => lines.push("from unittest.mock import MagicMock".to_owned()),
        (false, false) => {}
    }
    lines.push(String::new());

    let mut uuid = false;
    let mut datetime = false;
    for (_, ty) in &hir.params {
        dummy_value_needs(ir, ty, &mut uuid, &mut datetime);
    }
    if uuid {
        lines.push("from uuid import uuid4".to_owned());
    }
    if datetime {
        lines.push("from datetime import datetime, timezone".to_owned());
    }
    lines.push(format!(
        "from app.logic.{} import {}",
        ctx.module, ctx.class_name
    ));
    // Only the records the test itself names (constructing the payload,
    // asserting the result) — not `ctx.schema_imports`, which also
    // covers records only ever touched inside the (mocked-out) body,
    // e.g. an error `raise`d past the mocks.
    let mut test_records: Vec<RecordId> = Vec::new();
    for (_, ty) in &hir.params {
        collect_record_ids(ty, &mut test_records);
    }
    collect_record_ids(&hir.return_ty, &mut test_records);
    let mut test_schema_imports: Vec<String> = test_records
        .iter()
        .map(|id| record_class_name(ir, *id))
        .collect();
    test_schema_imports.sort();
    for name in &test_schema_imports {
        lines.push(format!("from app.schemas import {name}"));
    }
    lines.push(String::new());
    lines.push(String::new());
    lines.push("@pytest.mark.anyio".to_owned());
    let test_params = if ctx.needs_queue { "monkeypatch" } else { "" };
    lines.push(format!(
        "async def test_{}_handle({test_params}) -> None:",
        ctx.module
    ));

    // `publish` is a bare module-level function (`from app.queue import
    // publish`), not a constructor-injected dependency like
    // `session`/`cache`/the extras — a real connection attempt would
    // need a live broker, so it's patched at the handler module's own
    // name binding instead of passed as a kwarg.
    if ctx.needs_queue {
        lines.push("    mock_publish = AsyncMock()".to_owned());
        lines.push(format!(
            "    monkeypatch.setattr(\"app.logic.{}.publish\", mock_publish)",
            ctx.module
        ));
    }

    let mut kwargs = Vec::new();
    if ctx.needs_db {
        lines.push("    session = AsyncMock()".to_owned());
        lines.push("    session.add = MagicMock()".to_owned());
        if needs.db_get {
            lines.push("    session.get = AsyncMock(return_value=None)".to_owned());
        }
        if needs.sa_query {
            // `db.query`/`count`/`delete_where` go through
            // `session.execute(..)`, not `session.get(..)` — its return
            // value needs `.scalars().all()`/`.scalar_one()`/`.rowcount`
            // configured so the lowered body's isinstance assertions
            // below (list/int) actually hold; MagicMock's `__iter__`
            // already defaults to `iter([])`, but `.scalar_one()` and
            // `.rowcount` default to a bare `MagicMock`, not an `int`.
            lines.push("    exec_result = MagicMock()".to_owned());
            lines.push("    exec_result.scalar_one.return_value = 0".to_owned());
            lines.push("    exec_result.rowcount = 0".to_owned());
            lines.push("    session.execute = AsyncMock(return_value=exec_result)".to_owned());
        }
        kwargs.push("session=session".to_owned());
    }
    if ctx.needs_cache {
        lines.push("    cache = AsyncMock()".to_owned());
        if needs.cache_get {
            lines.push("    cache.get = AsyncMock(return_value=None)".to_owned());
        }
        kwargs.push("cache=cache".to_owned());
    }
    for extra in &ctx.extras {
        let var = &extra.param;
        lines.push(format!("    {var} = AsyncMock()"));
        if extra.kind == "object_store" && needs.object_store_get {
            lines.push(format!("    {var}.get = AsyncMock(return_value=b\"null\")"));
        }
        if extra.kind == "object_store" && needs.object_store_list {
            lines.push(format!("    {var}.list = AsyncMock(return_value=[])"));
        }
        kwargs.push(format!("{var}={var}"));
    }
    lines.push(format!(
        "    handler = {}({})",
        ctx.class_name,
        kwargs.join(", ")
    ));
    lines.push(String::new());

    let args = hir
        .params
        .iter()
        .map(|(_, ty)| dummy_value(ir, ty))
        .collect::<Vec<_>>()
        .join(", ");
    lines.push(format!("    result = await handler.handle({args})"));
    lines.push(String::new());
    if let Some(assertion) = assert_result(ir, &hir.return_ty) {
        lines.push(assertion);
    }
    if needs.db_insert {
        lines.push("    session.add.assert_called_once()".to_owned());
        lines.push("    session.commit.assert_awaited_once()".to_owned());
    }
    if needs.db_get {
        lines.push("    session.get.assert_awaited_once()".to_owned());
    }
    if needs.sa_query {
        lines.push("    session.execute.assert_awaited_once()".to_owned());
    }
    if needs.cache_set {
        lines.push("    cache.set.assert_awaited_once()".to_owned());
    }
    if needs.cache_get {
        lines.push("    cache.get.assert_awaited_once()".to_owned());
    }
    if ctx.needs_queue {
        lines.push("    mock_publish.assert_awaited_once()".to_owned());
    }
    for extra in &ctx.extras {
        if extra.kind == "object_store" {
            let var = &extra.param;
            if needs.object_store_put {
                lines.push(format!("    {var}.put.assert_awaited_once()"));
            }
            if needs.object_store_get {
                lines.push(format!("    {var}.get.assert_awaited_once()"));
            }
            if needs.object_store_list {
                lines.push(format!("    {var}.list.assert_awaited_once()"));
            }
        }
    }

    Some(lines.join("\n") + "\n")
}
