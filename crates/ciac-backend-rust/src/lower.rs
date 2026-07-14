//! Direct lowering of the typed HIR (`ciac_ir::hir`) into Rust source.
//!
//! Simpler than the Python backend's `lower.rs` in one respect: Rust's
//! `if`/`match` are real expressions and `{ stmts; tail }` is a real
//! block-expression, so `if`/`match`-as-a-`let`-value and `db.insert`'s
//! "run a statement, then yield the original value" both lower as plain
//! nested expressions. There's no need for Python's `Sink`/`lower_tail`
//! split, which exists only because Python statements aren't
//! expressions.
//!
//! One thing Rust needs that Python didn't: enum values are a *named*
//! type (`VideoStatus::Ready`), not a bare string, so a bare
//! [`ciac_ir::HirExpr::EnumLit`] can't be lowered in isolation — its
//! enclosing context (a comparison LHS, a record field) is what tells
//! us which named enum it belongs to. See `field_access_enum_name`.

use ciac_codegen::model as context;
use ciac_ir::{
    BinOp, Builtin, FieldType, HandlerBody, HirArm, HirExpr, HirStmt, HirType, NormalizedIr,
    RecordId, TableId, UnOp, Verb,
};
use heck::{ToPascalCase, ToSnakeCase};
use serde::Serialize;

/// Everything referenced by a handler body that the emitting backend
/// needs to know about to write `use`s and a constructor. No per-verb
/// breakdown (`db_insert` vs `db_get`, ...) — that only existed on the
/// Python side to drive generated mock-test assertions, and Rust has no
/// generated behavioral test in this milestone (see the M4 plan's
/// Non-goals: no mock/trait seam for `sqlx::PgPool`/`redis::Client`/
/// `ObjectStore` exists yet).
#[derive(Debug, Default)]
pub struct Needs {
    pub db: bool,
    pub cache: bool,
    pub queue: bool,
    pub uuid: bool,
    pub datetime: bool,
    pub tables: Vec<TableId>,
    /// Tables read via `db.get`/`db.query` (v0.14 M2) — the only verbs
    /// whose lowering spells the table's model type as a Rust type name
    /// (`sqlx::query_as::<_, Model>`). Every other db verb — `insert`,
    /// `update`, `delete`, `count`, `delete_where` — binds by raw SQL
    /// and field name only, never naming the model type, so importing
    /// it for them would be an unused import under `-D warnings`.
    pub db_get_tables: Vec<TableId>,
    pub records: Vec<RecordId>,
    /// Named enum types (`VideoStatus`) actually spelled out in the
    /// lowered body — via an enum-literal comparison or record field
    /// value (see `field_access_enum_name`). Deliberately *not* "every
    /// enum any referenced record has": a record with an enum field
    /// doesn't mean the body ever names that enum type directly (e.g. a
    /// `db.get` never does — the conversion is fully inside
    /// `models.rs`'s `TryFrom` impl), and importing an unused one is an
    /// `unused_imports` error under `-D warnings`.
    pub enums: Vec<String>,
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

/// Scans a handler's params, return type, and body for everything
/// [`Needs`] tracks. A record import comes from a verb call's own
/// return/argument types (`needs.ty(ty)`, called at every verb-call
/// site below), not from merely touching a table — see
/// `Needs::db_get_tables`'s doc for which verbs need the model import.
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
                    needs.table(*table);
                }
                Verb::DbGet(table) => {
                    needs.db = true;
                    needs.table(*table);
                    if !needs.db_get_tables.contains(table) {
                        needs.db_get_tables.push(*table);
                    }
                }
                Verb::CacheGet | Verb::CacheSet => needs.cache = true,
                Verb::ObjectStorePut | Verb::ObjectStoreGet => {}
                Verb::DbUpdate(table) | Verb::DbDelete(table) => {
                    needs.db = true;
                    needs.table(*table);
                }
                Verb::CacheDelete => needs.cache = true,
                Verb::ObjectStoreDelete | Verb::ObjectStoreList => {}
                Verb::EmailSend | Verb::SearchIndex | Verb::SearchQuery | Verb::HttpCall => {}
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
            match verb {
                Verb::DbQuery(table) => {
                    needs.table(*table);
                    if !needs.db_get_tables.contains(table) {
                        needs.db_get_tables.push(*table);
                    }
                }
                Verb::DbCount(table) | Verb::DbDeleteWhere(table) => needs.table(*table),
                _ => unreachable!("HirExpr::Query only ever carries a db query verb"),
            }
            if let Some(predicate) = predicate {
                for term in &predicate.terms {
                    scan_expr(ir, &term.value, needs);
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

/// Rust type annotation for a HIR type. A bare (field-less) `Enum` type
/// has no named Rust type to reach for — the language surface never
/// puts one directly in a param/return position (enums only ever show
/// up as record fields, resolved through `field_access_enum_name`), so
/// this panics rather than silently emitting something wrong.
pub fn rust_type(ir: &NormalizedIr, ty: &HirType) -> String {
    match ty {
        HirType::Str | HirType::Uuid => "String".to_owned(),
        HirType::Int => "i64".to_owned(),
        HirType::Float => "f64".to_owned(),
        HirType::Bool => "bool".to_owned(),
        HirType::Timestamp => "chrono::DateTime<chrono::Utc>".to_owned(),
        HirType::Json => "serde_json::Value".to_owned(),
        HirType::Enum { .. } => {
            unreachable!("a bare enum type never appears in a param/return position")
        }
        HirType::Record(id) => ir.record(*id).name.clone(),
        HirType::Option(inner) => format!("Option<{}>", rust_type(ir, inner)),
        HirType::List(inner) => format!("Vec<{}>", rust_type(ir, inner)),
        HirType::Unit | HirType::Never => "()".to_owned(),
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

fn stream_subject(ir: &NormalizedIr, stream: ciac_ir::NodeId) -> String {
    match &ir.node(stream).component {
        ciac_ir::Component::Stream { subject, .. } => subject.clone(),
        other => unreachable!("publish target is a stream, found {other:?}"),
    }
}

/// A Rust float literal that's actually valid as `f64` — `format!("{f}")`
/// on `1.0_f64` prints `"1"`, which Rust parses as an integer literal.
fn rust_float_lit(f: f64) -> String {
    let s = format!("{f}");
    if s.contains(['.', 'e', 'E']) || s == "inf" || s == "-inf" || s == "NaN" {
        s
    } else {
        format!("{s}.0")
    }
}

/// Recovers a field access's *named* enum type (e.g. `VideoStatus` for
/// `inserted.status`) from the base's record type — the only place this
/// information exists, since [`HirType::Enum`] is structural (a bare
/// variant set), not nominal. `None` when `expr` isn't a field access on
/// a record, or the field isn't an enum.
fn field_access_enum_name(ir: &NormalizedIr, expr: &HirExpr) -> Option<String> {
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

fn indent(text: &str, pad: &str) -> String {
    text.lines()
        .map(|line| format!("{pad}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// `rust_binary`/`rust_expr` always wrap a value in `(..)` so it composes
/// safely when nested inside another expression. An `if`/`match`
/// condition is never nested that way, so the outermost pair is
/// redundant there — and redundant parens are `unused_parens`, promoted
/// to a hard error by `-D warnings`.
fn strip_outer_parens(s: String) -> String {
    if !s.starts_with('(') || !s.ends_with(')') {
        return s;
    }
    let mut depth = 0i32;
    for (i, ch) in s.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && i != s.len() - 1 {
                    // The first `(` closes before the string ends, so
                    // the leading/trailing chars aren't a single
                    // wrapping pair (e.g. `(a) + (b)`) — leave it alone.
                    return s;
                }
            }
            _ => {}
        }
    }
    s[1..s.len() - 1].to_owned()
}

/// Lowers a single expression to a Rust expression string.
pub fn rust_expr(ir: &NormalizedIr, body: &HandlerBody, expr: &HirExpr) -> String {
    match expr {
        HirExpr::Local { slot, .. } => slot_name(body, *slot),
        HirExpr::IntLit(n) => n.to_string(),
        HirExpr::FloatLit(f) => rust_float_lit(*f),
        HirExpr::StrLit(s) => format!("{s:?}.to_owned()"),
        HirExpr::BoolLit(b) => if *b { "true" } else { "false" }.to_owned(),
        HirExpr::BuiltinCall(Builtin::UuidNew) => "uuid::Uuid::new_v4().to_string()".to_owned(),
        HirExpr::BuiltinCall(Builtin::TimestampNow) => "chrono::Utc::now()".to_owned(),
        HirExpr::EnumLit { variant, .. } => unreachable!(
            "bare enum literal `{variant}` must be lowered at its use site \
             (comparison/record field), not as a standalone expression"
        ),
        HirExpr::FieldAccess { base, field, .. } => {
            format!("{}.{field}", rust_expr(ir, body, base))
        }
        HirExpr::Index { base, index } => {
            let key = match index.as_ref() {
                HirExpr::StrLit(s) => format!("{s:?}"),
                other => rust_expr(ir, body, other),
            };
            format!("{}[{key}]", rust_expr(ir, body, base))
        }
        HirExpr::RecordCons {
            record,
            base_value,
            fields,
        } => rust_record_cons(ir, body, *record, base_value.as_deref(), fields),
        HirExpr::Binary { op, lhs, rhs, .. } => rust_binary(ir, body, *op, lhs, rhs),
        HirExpr::Unary { op, expr, .. } => {
            let inner = rust_expr(ir, body, expr);
            match op {
                UnOp::Neg => format!("(-{inner})"),
                UnOp::Not => format!("(!{inner})"),
            }
        }
        HirExpr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => format!(
            "if {} {{\n{}\n}} else {{\n{}\n}}",
            strip_outer_parens(rust_expr(ir, body, cond)),
            indent(&rust_block(ir, body, then_branch, Tail::Plain), "    "),
            indent(&rust_block(ir, body, else_branch, Tail::Plain), "    "),
        ),
        HirExpr::Match {
            scrutinee, arms, ..
        } => rust_match(ir, body, scrutinee, arms),
        HirExpr::VerbCall { verb, args, .. } => rust_verb_expr(ir, body, *verb, args),
        HirExpr::Query {
            verb, predicate, ..
        } => rust_query_expr(ir, body, *verb, predicate),
    }
}

fn rust_record_cons(
    ir: &NormalizedIr,
    body: &HandlerBody,
    record: RecordId,
    base_value: Option<&HirExpr>,
    fields: &[(String, HirExpr)],
) -> String {
    let record_name = record_class_name(ir, record);
    let field_strs: Vec<String> = fields
        .iter()
        .map(|(name, value)| {
            if let HirExpr::EnumLit { variant, .. } = value {
                let enum_name = format!("{record_name}{}", name.to_pascal_case());
                format!("{name}: {enum_name}::{variant}")
            } else {
                format!("{name}: {}", rust_expr(ir, body, value))
            }
        })
        .collect();
    match base_value {
        None => format!("{record_name} {{ {} }}", field_strs.join(", ")),
        Some(base) => format!(
            "{record_name} {{ {}, ..{} }}",
            field_strs.join(", "),
            rust_expr(ir, body, base)
        ),
    }
}

fn rust_binary(
    ir: &NormalizedIr,
    body: &HandlerBody,
    op: BinOp,
    lhs: &HirExpr,
    rhs: &HirExpr,
) -> String {
    if matches!(op, BinOp::Eq | BinOp::NotEq) {
        if let HirExpr::EnumLit { variant, .. } = rhs {
            let enum_name = field_access_enum_name(ir, lhs)
                .expect("enum comparison LHS must be a record field access");
            let op_s = if op == BinOp::Eq { "==" } else { "!=" };
            return format!(
                "({} {op_s} {enum_name}::{variant})",
                rust_expr(ir, body, lhs)
            );
        }
    }
    let lhs_ty = lhs.ty();
    let rhs_ty = rhs.ty();
    let lhs_s = rust_expr(ir, body, lhs);
    let rhs_s = rust_expr(ir, body, rhs);
    if op == BinOp::Add && (lhs_ty == HirType::Str || rhs_ty == HirType::Str) {
        // `String + String` doesn't compile and mixed-type numeric
        // stringification is finicky; `format!` handles any
        // `Display`-able pair uniformly.
        return format!("format!(\"{{}}{{}}\", {lhs_s}, {rhs_s})");
    }
    let py_op = match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Eq => "==",
        BinOp::NotEq => "!=",
        BinOp::Lt => "<",
        BinOp::LtEq => "<=",
        BinOp::Gt => ">",
        BinOp::GtEq => ">=",
        BinOp::And => "&&",
        BinOp::Or => "||",
    };
    format!("({lhs_s} {py_op} {rhs_s})")
}

fn rust_match(
    ir: &NormalizedIr,
    body: &HandlerBody,
    scrutinee: &HirExpr,
    arms: &[HirArm],
) -> String {
    let enum_name = field_access_enum_name(ir, scrutinee)
        .expect("match scrutinee must be a record field access");
    let scrut_s = rust_expr(ir, body, scrutinee);
    let mut out = format!("match {scrut_s} {{\n");
    for arm in arms {
        let pattern = match &arm.variant {
            Some(v) => format!("{enum_name}::{v}"),
            None => "_".to_owned(),
        };
        let arm_body = indent(&rust_block(ir, body, &arm.body, Tail::Plain), "        ");
        out.push_str(&format!("    {pattern} => {{\n{arm_body}\n    }}\n"));
    }
    out.push('}');
    out
}

/// Lowers every verb call — all of them fit a single Rust expression
/// (unlike Python, `db.insert`'s statement-then-value shape is just a
/// block-expression).
fn rust_verb_expr(ir: &NormalizedIr, body: &HandlerBody, verb: Verb, args: &[HirExpr]) -> String {
    match verb {
        Verb::DbInsert(table) => {
            let engine = db_engine_of(ir, body);
            let table_snake = ir.table(table).name.to_snake_case();
            let record_ctx = context::build_record(ir, ir.table(table).record);
            let value = rust_expr(ir, body, &args[0]);
            let binds: String = record_ctx
                .fields
                .iter()
                .map(|f| {
                    if f.is_enum {
                        format!("\n        .bind(__row.{}.as_str())", f.name)
                    } else {
                        format!("\n        .bind(&__row.{})", f.name)
                    }
                })
                .collect();
            // Cloned into `__row` rather than moving `{value}` straight
            // into the block: the source value (typically a plain local
            // like `v`) may still be referenced later in the handler
            // body (e.g. a `fail` after the insert), and Rust — unlike
            // Python — would reject that as a use of a moved value.
            format!(
                "{{\n    let __row = {value}.clone();\n    sqlx::query(\"INSERT INTO {table_snake} ({cols}) VALUES ({phs})\"){binds}\n        .execute(self.db)\n        .await?;\n    __row\n}}",
                cols = record_ctx.select_cols,
                phs = ciac_codegen::template::sqlph(&record_ctx.insert_placeholders, engine),
            )
        }
        Verb::DbGet(table) => {
            let engine = db_engine_of(ir, body);
            let model = model_class_name(ir, table);
            let record_name = record_class_name(ir, ir.table(table).record);
            let table_snake = ir.table(table).name.to_snake_case();
            let record_ctx = context::build_record(ir, ir.table(table).record);
            let key = rust_expr(ir, body, &args[0]);
            format!(
                "{{\n    let row: Option<{model}> = sqlx::query_as(\"SELECT {cols} FROM {table_snake} WHERE id = {ph}\")\n        .bind({key}.to_string())\n        .fetch_optional(self.db)\n        .await?;\n    row.map({record_name}::try_from).transpose()?\n}}",
                cols = record_ctx.select_cols,
                ph = ciac_codegen::template::sqlph("$1", engine),
            )
        }
        Verb::CacheGet => {
            let key = rust_expr(ir, body, &args[0]);
            format!(
                "{{\n    let mut conn = self.cache.get_multiplexed_async_connection().await?;\n    let raw: Option<String> = conn.get(&{key}).await?;\n    match raw {{\n        Some(s) => Some(serde_json::from_str(&s)?),\n        None => None,\n    }}\n}}"
            )
        }
        Verb::CacheSet => {
            let key = rust_expr(ir, body, &args[0]);
            let value = rust_expr(ir, body, &args[1]);
            format!(
                "{{\n    let mut conn = self.cache.get_multiplexed_async_connection().await?;\n    let _: () = conn.set(&{key}, serde_json::to_string(&{value})?).await?;\n}}"
            )
        }
        Verb::ObjectStorePut => {
            let key = rust_expr(ir, body, &args[0]);
            let payload = if matches!(args[1].ty(), HirType::Record(_)) {
                format!("&serde_json::to_vec(&{})?", rust_expr(ir, body, &args[1]))
            } else {
                format!("{}.to_string().as_bytes()", rust_expr(ir, body, &args[1]))
            };
            format!("self.object_store.put(&{key}, {payload}).await?")
        }
        Verb::ObjectStoreGet => {
            let key = rust_expr(ir, body, &args[0]);
            format!("serde_json::from_slice(&self.object_store.get(&{key}).await?)?")
        }
        Verb::DbUpdate(table) => {
            let engine = db_engine_of(ir, body);
            let table_snake = ir.table(table).name.to_snake_case();
            let record_ctx = context::build_record(ir, ir.table(table).record);
            let key = rust_expr(ir, body, &args[0]);
            let value = rust_expr(ir, body, &args[1]);
            let binds: String = record_ctx
                .fields
                .iter()
                .filter(|f| f.name != "id")
                .map(|f| {
                    if f.is_enum {
                        format!("\n        .bind(__row.{}.as_str())", f.name)
                    } else {
                        format!("\n        .bind(&__row.{})", f.name)
                    }
                })
                .collect();
            // Same clone-into-`__row` reasoning as `db.insert` above: the
            // source value may still be referenced later in the body.
            format!(
                "{{\n    let __row = {value}.clone();\n    let __updated = sqlx::query(\"UPDATE {table_snake} SET {assignments} WHERE id = {where_ph}\"){binds}\n        .bind({key}.to_string())\n        .execute(self.db)\n        .await?;\n    if __updated.rows_affected() == 0 {{ None }} else {{ Some(__row) }}\n}}",
                assignments = ciac_codegen::template::sqlph(&record_ctx.update_assignments, engine),
                where_ph = ciac_codegen::template::sqlph(&record_ctx.update_where, engine),
            )
        }
        Verb::DbDelete(table) => {
            let engine = db_engine_of(ir, body);
            let table_snake = ir.table(table).name.to_snake_case();
            let key = rust_expr(ir, body, &args[0]);
            format!(
                "{{\n    let __deleted = sqlx::query(\"DELETE FROM {table_snake} WHERE id = {ph}\")\n        .bind({key}.to_string())\n        .execute(self.db)\n        .await?;\n    __deleted.rows_affected() > 0\n}}",
                ph = ciac_codegen::template::sqlph("$1", engine),
            )
        }
        Verb::CacheDelete => {
            let key = rust_expr(ir, body, &args[0]);
            format!(
                "{{\n    let mut conn = self.cache.get_multiplexed_async_connection().await?;\n    let _: () = conn.del(&{key}).await?;\n}}"
            )
        }
        Verb::ObjectStoreDelete => {
            let key = rust_expr(ir, body, &args[0]);
            format!("self.object_store.delete(&{key}).await?")
        }
        Verb::ObjectStoreList => {
            let prefix = rust_expr(ir, body, &args[0]);
            format!("self.object_store.list(&{prefix}).await?")
        }
        Verb::EmailSend => {
            let to = rust_expr(ir, body, &args[0]);
            let subject = rust_expr(ir, body, &args[1]);
            let body_arg = rust_expr(ir, body, &args[2]);
            format!("self.email.send(&{to}, &{subject}, &{body_arg}).await?")
        }
        Verb::SearchIndex => {
            let doc_id = rust_expr(ir, body, &args[0]);
            let document = rust_json_value(ir, body, &args[1]);
            format!("self.search.index({SEARCH_INDEX_NAME:?}, &{doc_id}, &{document}).await?")
        }
        Verb::SearchQuery => {
            let query = rust_expr(ir, body, &args[0]);
            format!(
                "self.search.search({SEARCH_INDEX_NAME:?}, &serde_json::json!({{\"query\": {{\"query_string\": {{\"query\": {query}}}}}}})).await?"
            )
        }
        Verb::HttpCall => {
            let url = rust_expr(ir, body, &args[0]);
            let json_val = rust_json_value(ir, body, &args[1]);
            format!(
                "self.http.post(&{url}).json(&{json_val}).send().await?.json::<serde_json::Value>().await?"
            )
        }
        Verb::DbQuery(_) | Verb::DbCount(_) | Verb::DbDeleteWhere(_) => {
            unreachable!("typeck only ever constructs these via HirExpr::Query")
        }
    }
}

/// OpenSearch has no index concept in ciac's language model, so every
/// `search.index`/`search.query` call targets one hardcoded index —
/// mirrors the Python backend's own copy of this constant.
const SEARCH_INDEX_NAME: &str = "documents";

/// Renders `value` as a `serde_json::Value`-typed argument for
/// `search.index`/`external_http.request`'s untyped payload params.
fn rust_json_value(ir: &NormalizedIr, body: &HandlerBody, value: &HirExpr) -> String {
    match value.ty() {
        HirType::Record(_) => format!("serde_json::to_value(&{})?", rust_expr(ir, body, value)),
        HirType::Json => rust_expr(ir, body, value),
        _ => format!(
            "serde_json::json!({{\"value\": {}}})",
            rust_expr(ir, body, value)
        ),
    }
}

/// Builds a ` WHERE ..` clause (empty string if there's no predicate)
/// and the ordered `.bind(..)` expressions it needs (v0.14 M2). Written
/// with Postgres-style `$N` placeholders and rewritten per-engine by
/// `sqlph`, same as every other SQL string this backend emits. A bare
/// enum comparison binds the variant's string form directly (`"Ready"`)
/// rather than going through `rust_expr`, which panics on a standalone
/// `EnumLit` — there's no named Rust enum type to resolve it against
/// here (`term.field` is a raw SQL column name, not a `FieldAccess`).
fn rust_where_clause(
    ir: &NormalizedIr,
    body: &HandlerBody,
    predicate: &Option<ciac_ir::HirPredicate>,
    engine: &str,
) -> (String, Vec<String>) {
    let Some(predicate) = predicate else {
        return (String::new(), Vec::new());
    };
    let mut conditions = Vec::with_capacity(predicate.terms.len());
    let mut binds = Vec::with_capacity(predicate.terms.len());
    for (i, term) in predicate.terms.iter().enumerate() {
        let idx = i + 1;
        let bind_expr = match &term.value {
            HirExpr::EnumLit { variant, .. } => format!("{variant:?}"),
            _ => {
                let value = rust_expr(ir, body, &term.value);
                if term.field_ty == HirType::Uuid {
                    format!("{value}.to_string()")
                } else {
                    value
                }
            }
        };
        let field = &term.field;
        let op = match term.op {
            ciac_ir::PredOp::Eq => "=",
            ciac_ir::PredOp::NotEq => "!=",
            ciac_ir::PredOp::Lt => "<",
            ciac_ir::PredOp::LtEq => "<=",
            ciac_ir::PredOp::Gt => ">",
            ciac_ir::PredOp::GtEq => ">=",
            ciac_ir::PredOp::Contains => "LIKE",
        };
        conditions.push(format!("{field} {op} ${idx}"));
        if term.op == ciac_ir::PredOp::Contains {
            binds.push(format!("format!(\"%{{}}%\", {bind_expr})"));
        } else {
            binds.push(bind_expr);
        }
    }
    let sql =
        ciac_codegen::template::sqlph(&format!(" WHERE {}", conditions.join(" AND ")), engine);
    (sql, binds)
}

/// Lowers `db.query`/`db.count`/`db.delete_where` (v0.14 M2), with or
/// without a `where` clause.
fn rust_query_expr(
    ir: &NormalizedIr,
    body: &HandlerBody,
    verb: Verb,
    predicate: &Option<ciac_ir::HirPredicate>,
) -> String {
    let engine = db_engine_of(ir, body);
    match verb {
        Verb::DbQuery(table) => {
            let table_snake = ir.table(table).name.to_snake_case();
            let model = model_class_name(ir, table);
            let record_name = record_class_name(ir, ir.table(table).record);
            let record_ctx = context::build_record(ir, ir.table(table).record);
            let (where_sql, binds) = rust_where_clause(ir, body, predicate, engine);
            let bind_lines: String = binds
                .iter()
                .map(|b| format!("\n        .bind({b})"))
                .collect();
            format!(
                "{{\n    let __rows: Vec<{model}> = sqlx::query_as(\"SELECT {cols} FROM {table_snake}{where_sql}\"){bind_lines}\n        .fetch_all(self.db)\n        .await?;\n    __rows.into_iter().map({record_name}::try_from).collect::<Result<Vec<_>, _>>()?\n}}",
                cols = record_ctx.select_cols,
            )
        }
        Verb::DbCount(table) => {
            let table_snake = ir.table(table).name.to_snake_case();
            let (where_sql, binds) = rust_where_clause(ir, body, predicate, engine);
            let bind_lines: String = binds
                .iter()
                .map(|b| format!("\n        .bind({b})"))
                .collect();
            format!(
                "{{\n    let __count: i64 = sqlx::query_scalar(\"SELECT COUNT(*) FROM {table_snake}{where_sql}\"){bind_lines}\n        .fetch_one(self.db)\n        .await?;\n    __count\n}}"
            )
        }
        Verb::DbDeleteWhere(table) => {
            let table_snake = ir.table(table).name.to_snake_case();
            let (where_sql, binds) = rust_where_clause(ir, body, predicate, engine);
            let bind_lines: String = binds
                .iter()
                .map(|b| format!("\n        .bind({b})"))
                .collect();
            format!(
                "{{\n    let __deleted = sqlx::query(\"DELETE FROM {table_snake}{where_sql}\"){bind_lines}\n        .execute(self.db)\n        .await?;\n    __deleted.rows_affected() as i64\n}}"
            )
        }
        _ => unreachable!("HirExpr::Query only ever carries a db query verb"),
    }
}

fn stmt_diverges(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::Return(_) | HirStmt::Fail { .. } => true,
        HirStmt::Let { value, .. } | HirStmt::Expr(value) => value.ty() == HirType::Never,
        HirStmt::Publish { .. } | HirStmt::Transaction { .. } => false,
    }
}

/// Where a block's tail statement (if it's a bare `HirStmt::Expr`) ends
/// up: `None` for a mid-block statement (always `expr;`), `Plain` for a
/// nested `if`/`match` branch's own tail (just the bare value, feeding
/// the enclosing expression), `Wrapped` for the function body's own
/// tail (needs `Ok(..)` — `handle()` returns `anyhow::Result<T>`).
#[derive(Clone, Copy)]
enum Tail {
    None,
    Plain,
    Wrapped,
}

fn rust_stmt(ir: &NormalizedIr, body: &HandlerBody, stmt: &HirStmt, tail: Tail) -> String {
    match stmt {
        HirStmt::Let { slot, value } => {
            if value.ty() == HirType::Never {
                // Every path through `value` returns/fails, so it never
                // actually produces anything to bind — a `let` here
                // would be an unused-variable warning, promoted to a
                // hard error by `-D warnings`. Just run it.
                format!("{};", rust_expr(ir, body, value))
            } else {
                format!(
                    "let {} = {};",
                    slot_name(body, *slot),
                    rust_expr(ir, body, value)
                )
            }
        }
        HirStmt::Expr(e) => {
            let e_s = rust_expr(ir, body, e);
            match tail {
                Tail::None => format!("{e_s};"),
                Tail::Plain => e_s,
                Tail::Wrapped => format!("Ok({e_s})"),
            }
        }
        HirStmt::Return(None) => "return Ok(());".to_owned(),
        HirStmt::Return(Some(e)) => format!("return Ok({});", rust_expr(ir, body, e)),
        HirStmt::Fail { error, args } => {
            let name = record_class_name(ir, *error);
            let record = ir.record(*error);
            let field_inits: Vec<String> = record
                .fields
                .iter()
                .zip(args)
                .map(|(f, a)| format!("{}: {}", f.name, rust_expr(ir, body, a)))
                .collect();
            format!(
                "return Err({name} {{ {} }}.into());",
                field_inits.join(", ")
            )
        }
        HirStmt::Publish { stream, value } => {
            let subject = stream_subject(ir, *stream);
            format!(
                "self.queue.publish({subject:?}, serde_json::to_vec(&{})?).await?;",
                rust_expr(ir, body, value)
            )
        }
        // v0.16 M6 (assessed, deliberately deferred): sema fully
        // validates `transaction` blocks, but making this genuinely
        // atomic needs every `db.*` verb inside to execute against a
        // held `sqlx::Transaction<'_, _>` instead of `self.db` — sqlx's
        // `Transaction` has no `Deref`-to-pool trick that lets already-
        // generated `.execute(self.db)` call sites transparently keep
        // working, so real atomicity means threading an executor choice
        // through `rust_expr`'s entire recursive call graph (30+ sites:
        // every arm that can nest a db verb, not just the db-verb arms
        // themselves) rather than the one `tx: bool` flag that sufficed
        // for the Python backend's uniform `self.session`. That's a
        // materially larger, riskier change to code every existing Rust
        // example depends on, so it's tracked as a follow-up rather than
        // attempted here. Interim: lower the body exactly as if
        // unwrapped — correct per-statement, just not yet atomic across
        // the whole block. See docs/language.md's transactions section.
        HirStmt::Transaction { body: inner } => {
            let note =
                "// NOTE: this block is not yet atomic on the Rust backend (see docs/language.md)";
            let inner_lines = rust_block(ir, body, inner, Tail::None);
            format!("{note}\n{inner_lines}")
        }
    }
}

/// Lowers a `Vec<HirStmt>` block into Rust statement lines, joined with
/// `\n`. Statements after one that diverges are dropped — Rust's
/// `-D warnings` promotes `unreachable_code` to a hard build failure, so
/// this isn't optional cosmetics (mirrors the same truncation in
/// Python's `lower.rs`).
fn rust_block(ir: &NormalizedIr, body: &HandlerBody, stmts: &[HirStmt], tail: Tail) -> String {
    if stmts.is_empty() {
        return match tail {
            Tail::Wrapped => "Ok(())".to_owned(),
            _ => "()".to_owned(),
        };
    }
    let mut lines = Vec::new();
    for (i, stmt) in stmts.iter().enumerate() {
        let is_last = i + 1 == stmts.len();
        lines.push(rust_stmt(
            ir,
            body,
            stmt,
            if is_last { tail } else { Tail::None },
        ));
        if stmt_diverges(stmt) {
            break;
        }
    }
    lines.join("\n")
}

/// Lowers a full handler body into the `handle()` method's Rust source
/// (the function-body's own tail gets `Ok(..)`-wrapped).
pub fn lower_body(ir: &NormalizedIr, body: &HandlerBody) -> String {
    let stmts = body.body.as_deref().unwrap_or(&[]);
    rust_block(ir, body, stmts, Tail::Wrapped)
}

#[derive(Debug, Serialize)]
pub struct ParamCtx {
    pub name: String,
    pub ty: String,
}

/// Everything `logic.rs.j2` needs to render one typed handler's file —
/// inline (compiler-owned, `src/logic/<module>.rs`) or `extern` (seeded,
/// `src/services/<module>.rs`, `body` is just a stand-in `Err(..)`).
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
    pub rust_db_field: Option<String>,
    pub rust_cache_field: Option<String>,
    /// Engine of the bound db instance (v0.13 M1): selects the sqlx
    /// pool type in `logic.rs.j2`. `postgres` when no db is bound.
    pub db_engine: String,
    pub extras: Vec<context::ExtraDepCtx>,
    pub schema_imports: Vec<String>,
    pub model_imports: Vec<String>,
    pub body: String,
}

/// Engine of the handler's bound db instance, straight from the IR
/// node its binding edge points at — `postgres` when unbound.
fn db_engine_of(ir: &NormalizedIr, body: &HandlerBody) -> &'static str {
    for id in body.capability_nodes() {
        if let ciac_ir::Component::Database { engine, .. } = &ir.node(id).component {
            return match engine {
                ciac_ir::DbEngine::MySql => "mysql",
                ciac_ir::DbEngine::Sqlite => "sqlite",
                ciac_ir::DbEngine::Postgres => "postgres",
            };
        }
    }
    "postgres"
}

/// Builds the render context for one typed handler node. `name` is the
/// handler's declared name (`node.component.name()`).
pub fn render(ir: &NormalizedIr, name: &str, hir: &HandlerBody) -> LogicFileCtx {
    let needs = scan(ir, hir);
    let bindings = context::hir_bindings(ir, hir);
    let access = context::access_of(&bindings);
    let extras = context::extras_of(&bindings);

    let mut schema_imports: Vec<String> = needs
        .records
        .iter()
        .map(|id| record_class_name(ir, *id))
        .chain(needs.enums.iter().cloned())
        .collect();
    schema_imports.sort();
    schema_imports.dedup();
    let mut model_imports: Vec<String> = needs
        .db_get_tables
        .iter()
        .map(|id| model_class_name(ir, *id))
        .collect();
    model_imports.sort();

    let params = hir
        .params
        .iter()
        .map(|(n, ty)| ParamCtx {
            name: n.clone(),
            ty: rust_type(ir, ty),
        })
        .collect();

    let body = match &hir.body {
        Some(_) => lower_body(ir, hir),
        None => "Err(anyhow::anyhow!(\"not implemented\"))".to_owned(),
    };

    LogicFileCtx {
        class_name: name.to_owned(),
        module: name.to_snake_case(),
        is_extern: hir.body.is_none(),
        params,
        return_type: rust_type(ir, &hir.return_ty),
        needs_db: access.db.is_some(),
        needs_cache: access.cache_expr.is_some(),
        needs_queue: needs.queue,
        rust_db_field: access.rust_db_field,
        rust_cache_field: access.rust_cache_field,
        db_engine: db_engine_of(ir, hir).to_owned(),
        extras,
        schema_imports,
        model_imports,
        body,
    }
}
