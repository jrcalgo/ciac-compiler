//! Direct lowering of the typed HIR (`ciac_ir::hir`) into Rust source.
//!
//! The walker (block/tail shaping, precedence, enum-literal use-site
//! recovery, float-literal fidelity, divergence truncation) lives in
//! `ciac_codegen::lower` (`22UpdatePlan.md` Pillar 3, Parts 2-3);
//! [`RustSyntax`] supplies only the leaf constructors genuinely
//! specific to this target — raw SQL text (not an ORM), `Ok(..)`
//! wrapping (`handle()` returns `anyhow::Result<T>`), and the E0382
//! clone discipline described below.
//!
//! One thing Rust needs that a `Statement`-oriented target doesn't:
//! enum values are a *named* type (`VideoStatus::Ready`), not a bare
//! string, so a bare [`ciac_ir::HirExpr::EnumLit`] can't be lowered in
//! isolation — its enclosing context (a comparison LHS, a record
//! field) is what tells us which named enum it belongs to. See
//! [`ciac_codegen::lower::field_access_enum_name`].

pub(crate) use ciac_codegen::lower::scan;
use ciac_codegen::lower::{
    self, fidelity_checked_float, indent_lines, strip_outer_parens, HostSyntax, IndexKey,
    LoweredPredicate, MatchArm, Orientation, PredValue, Wrap,
};
use ciac_codegen::model as context;
use ciac_ir::{
    BinOp, HandlerBody, HirExpr, HirType, NormalizedIr, PredOp, RecordId, TableId, UnOp, Verb,
};
use heck::ToSnakeCase;
use serde::Serialize;

/// Rust type annotation for a HIR type — a handler *signature*
/// concern (param/return types), not part of the `HostSyntax` body
/// contract. A bare (field-less) `Enum` type has no named Rust type
/// to reach for — the language surface never puts one directly in a
/// param/return position (enums only ever show up as record fields,
/// resolved through `field_access_enum_name`), so this panics rather
/// than silently emitting something wrong.
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

/// Engine of the handler's bound db instance, straight from the IR
/// node its binding edge points at — `postgres` when unbound.
/// Handler-level (not per-verb-call), so [`RustSyntax`] resolves it
/// once at construction and every db-verb leaf reuses it.
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

fn model_class_name(ir: &NormalizedIr, table: TableId) -> String {
    ir.table(table).name.clone()
}

fn record_class_name(ir: &NormalizedIr, record: ciac_ir::RecordId) -> String {
    ir.record(record).name.clone()
}

/// The `Orientation::Expression` `HostSyntax` implementation for this
/// target: `if`/`match`/every db verb lower as real Rust expressions,
/// so there is no `Sink`/`Dest` concept here at all — the shared
/// dispatcher never calls any `Statement`-oriented leaf on this type.
/// Holds `ir` (every leaf resolves table/record/enum names through
/// it) and `db_engine` (resolved once per handler, v0.13 M1, since
/// every db-verb leaf needs the same placeholder style).
struct RustSyntax<'a> {
    ir: &'a NormalizedIr,
    db_engine: &'static str,
}

impl RustSyntax<'_> {
    /// Builds a ` WHERE ..` clause (empty string if there's no
    /// predicate) and the ordered `.bind(..)` expressions it needs
    /// (v0.14 M2). Written with Postgres-style `$N` placeholders and
    /// rewritten per-engine by `sqlph`, same as every other SQL string
    /// this backend emits. A bare enum comparison binds the variant's
    /// string form directly (`"Ready"`) rather than through
    /// `enum_literal`, which requires a named type this raw-SQL
    /// position has no use for (`term.field` is a raw SQL column name,
    /// not a `FieldAccess`).
    fn where_clause(&self, predicate: Option<&LoweredPredicate>) -> (String, Vec<String>) {
        let Some(predicate) = predicate else {
            return (String::new(), Vec::new());
        };
        let mut conditions = Vec::with_capacity(predicate.terms.len());
        let mut binds = Vec::with_capacity(predicate.terms.len());
        for (i, term) in predicate.terms.iter().enumerate() {
            let idx = i + 1;
            let bind_expr = match &term.value {
                PredValue::EnumVariant(v) => format!("{v:?}"),
                PredValue::BoolLit(b) => if *b { "true" } else { "false" }.to_owned(),
                PredValue::Rendered(s) => {
                    if term.field_ty == HirType::Uuid {
                        format!("{s}.to_string()")
                    } else {
                        s.clone()
                    }
                }
            };
            let field = &term.field;
            let op = match term.op {
                PredOp::Eq => "=",
                PredOp::NotEq => "!=",
                PredOp::Lt => "<",
                PredOp::LtEq => "<=",
                PredOp::Gt => ">",
                PredOp::GtEq => ">=",
                PredOp::Contains => "LIKE",
            };
            conditions.push(format!("{field} {op} ${idx}"));
            if term.op == PredOp::Contains {
                binds.push(format!("format!(\"%{{}}%\", {bind_expr})"));
            } else {
                binds.push(bind_expr);
            }
        }
        let sql = ciac_codegen::template::sqlph(
            &format!(" WHERE {}", conditions.join(" AND ")),
            self.db_engine,
        );
        (sql, binds)
    }

    /// Renders `value` as a `serde_json::Value`-typed argument for
    /// `search.index`/`external_http.request`'s untyped payload
    /// params — the 3-way (Record/Json/else) branch is a real,
    /// pre-existing divergence from `cache.set`/`object_store.put`'s
    /// 2-way branch, not something to unify away.
    fn json_value(&self, value: &str, value_ty: &HirType) -> String {
        match value_ty {
            HirType::Record(_) => format!("serde_json::to_value(&{value})?"),
            HirType::Json => value.to_owned(),
            _ => format!("serde_json::json!({{\"value\": {value}}})"),
        }
    }
}

const SEARCH_INDEX_NAME: &str = "documents";

impl HostSyntax for RustSyntax<'_> {
    const ORIENTATION: Orientation = Orientation::Expression;

    fn int_lit(&self, n: i64) -> String {
        n.to_string()
    }
    fn float_lit(&self, f: f64) -> String {
        fidelity_checked_float(f)
    }
    fn str_lit(&self, s: &str) -> String {
        format!("{s:?}.to_owned()")
    }
    fn bool_lit(&self, b: bool) -> String {
        if b { "true" } else { "false" }.to_owned()
    }
    fn field_access(&self, base: &str, field: &str) -> String {
        format!("{base}.{field}")
    }
    fn index(&self, base: &str, key: IndexKey<'_>) -> String {
        let k = match key {
            IndexKey::StrKey(s) => format!("{s:?}"),
            IndexKey::Expr(e) => e,
        };
        format!("{base}[{k}]")
    }
    fn uuid_new(&self) -> String {
        "uuid::Uuid::new_v4().to_string()".to_owned()
    }
    fn timestamp_now(&self) -> String {
        "chrono::Utc::now()".to_owned()
    }
    fn enum_literal(&self, enum_name: Option<&str>, variant: &str) -> String {
        let name = enum_name.unwrap_or_else(|| {
            unreachable!(
                "bare enum literal `{variant}` must be lowered at its use site \
                 (comparison/record field), not as a standalone expression"
            )
        });
        format!("{name}::{variant}")
    }
    /// Renders an expression used as a record-construction field value
    /// (or `..base`) — the one place, across every verb, that a
    /// still-live handler-input value's *field* can end up borrowed
    /// into a brand-new owned record. `x.field` used this way must be
    /// cloned, not moved: unlike a GC'd/statement-oriented target,
    /// Rust rejects a later whole-value use of `x` (e.g. returning the
    /// handler's input parameter itself after only one of its fields
    /// was pulled into an inserted/updated row) as a use of a
    /// partially moved value — found live via `ciac verify -t rust` on
    /// `sim-vertical-slice.ciac`/`sim-broker-slice.ciac` (v0.17 M11),
    /// whose handlers do exactly that. Always cloning here (rather
    /// than tracking whether the source is actually reused later) is
    /// the simplest correct rule: cloning a `Copy` field is a harmless
    /// no-op, and every non-`Copy` field this compiler generates
    /// (`String`, `serde_json::Value`, ..) is cheap enough that this
    /// is not a meaningful cost next to the query it feeds.
    fn value_for_record_field(&self, rendered: String, original: &HirExpr) -> String {
        if matches!(original, HirExpr::FieldAccess { .. }) {
            format!("{rendered}.clone()")
        } else {
            rendered
        }
    }
    fn record_cons(
        &self,
        record_name: &str,
        fields: &[(String, String)],
        base: Option<&str>,
    ) -> String {
        let field_strs: Vec<String> = fields
            .iter()
            .map(|(name, value)| format!("{name}: {value}"))
            .collect();
        match base {
            None => format!("{record_name} {{ {} }}", field_strs.join(", ")),
            Some(base) => format!("{record_name} {{ {}, ..{base} }}", field_strs.join(", ")),
        }
    }
    fn binary(
        &self,
        op: BinOp,
        lhs: &str,
        rhs: &str,
        lhs_ty: &HirType,
        rhs_ty: &HirType,
    ) -> String {
        if op == BinOp::Add && (*lhs_ty == HirType::Str || *rhs_ty == HirType::Str) {
            // `String + String` doesn't compile and mixed-type numeric
            // stringification is finicky; `format!` handles any
            // `Display`-able pair uniformly.
            return format!("format!(\"{{}}{{}}\", {lhs}, {rhs})");
        }
        let op_s = match op {
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
        format!("({lhs} {op_s} {rhs})")
    }
    fn unary(&self, op: UnOp, operand: &str) -> String {
        match op {
            UnOp::Neg => format!("(-{operand})"),
            UnOp::Not => format!("(!{operand})"),
        }
    }

    fn if_expr(&self, cond: &str, then_block: &str, else_block: &str) -> String {
        format!(
            "if {} {{\n{}\n}} else {{\n{}\n}}",
            strip_outer_parens(cond.to_owned()),
            indent_lines(then_block, "    "),
            indent_lines(else_block, "    "),
        )
    }
    fn match_expr(&self, enum_name: &str, scrutinee: &str, arms: &[MatchArm]) -> String {
        let mut out = format!("match {scrutinee} {{\n");
        for arm in arms {
            let pattern = match &arm.variant {
                Some(v) => format!("{enum_name}::{v}"),
                None => "_".to_owned(),
            };
            let arm_body = indent_lines(&arm.body, "        ");
            out.push_str(&format!("    {pattern} => {{\n{arm_body}\n    }}\n"));
        }
        out.push('}');
        out
    }
    fn db_insert_expr(&self, table: TableId, value: &str) -> String {
        let table_snake = self.ir.table(table).name.to_snake_case();
        let record_ctx = context::build_record(self.ir, self.ir.table(table).record);
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
        // like `v`) may still be referenced later in the handler body
        // (e.g. a `fail` after the insert), and Rust would reject that
        // as a use of a moved value.
        //
        // The `self.world` branch is the v0.17 M11 world-guard: when
        // `AppState::simulation` constructed this handler, `db.insert`
        // writes to the in-memory `SimWorld` (and can be
        // failure-injected) instead of ever touching `self.db`.
        format!(
            "{{\n    let __row = {value}.clone();\n    if let Some(world) = self.world {{\n        world.db_insert_checked(\"{table_snake}\", serde_json::to_value(&__row)?)?;\n    }} else {{\n        sqlx::query(\"INSERT INTO {table_snake} ({cols}) VALUES ({phs})\"){binds}\n            .execute(self.db)\n            .await?;\n    }}\n    __row\n}}",
            cols = record_ctx.select_cols,
            phs = ciac_codegen::template::sqlph(&record_ctx.insert_placeholders, self.db_engine),
        )
    }
    fn db_update_expr(&self, table: TableId, key: &str, value: &str) -> String {
        let table_snake = self.ir.table(table).name.to_snake_case();
        let record_ctx = context::build_record(self.ir, self.ir.table(table).record);
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
            assignments = ciac_codegen::template::sqlph(&record_ctx.update_assignments, self.db_engine),
            where_ph = ciac_codegen::template::sqlph(&record_ctx.update_where, self.db_engine),
        )
    }
    fn db_delete_expr(&self, table: TableId, key: &str) -> String {
        let table_snake = self.ir.table(table).name.to_snake_case();
        format!(
            "{{\n    let __deleted = sqlx::query(\"DELETE FROM {table_snake} WHERE id = {ph}\")\n        .bind({key}.to_string())\n        .execute(self.db)\n        .await?;\n    __deleted.rows_affected() > 0\n}}",
            ph = ciac_codegen::template::sqlph("$1", self.db_engine),
        )
    }
    fn query_expr(&self, verb: Verb, predicate: Option<&LoweredPredicate>) -> String {
        match verb {
            Verb::DbQuery(table) => {
                let table_snake = self.ir.table(table).name.to_snake_case();
                let model = model_class_name(self.ir, table);
                let record_name = record_class_name(self.ir, self.ir.table(table).record);
                let record_ctx = context::build_record(self.ir, self.ir.table(table).record);
                let (where_sql, binds) = self.where_clause(predicate);
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
                let table_snake = self.ir.table(table).name.to_snake_case();
                let (where_sql, binds) = self.where_clause(predicate);
                let bind_lines: String = binds
                    .iter()
                    .map(|b| format!("\n        .bind({b})"))
                    .collect();
                format!(
                    "{{\n    let __count: i64 = sqlx::query_scalar(\"SELECT COUNT(*) FROM {table_snake}{where_sql}\"){bind_lines}\n        .fetch_one(self.db)\n        .await?;\n    __count\n}}"
                )
            }
            Verb::DbDeleteWhere(table) => {
                let table_snake = self.ir.table(table).name.to_snake_case();
                let (where_sql, binds) = self.where_clause(predicate);
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
    fn let_binding(&self, name: &str, value: &str) -> String {
        format!("let {name} = {value};")
    }
    fn wrap_tail(&self, value: &str, wrap: Wrap) -> String {
        match wrap {
            Wrap::None => format!("{value};"),
            Wrap::Plain => value.to_owned(),
            Wrap::Wrapped => format!("Ok({value})"),
        }
    }
    fn unit_literal(&self) -> String {
        "()".to_owned()
    }
    // v0.16 M6 (assessed, deliberately deferred): sema fully validates
    // `transaction` blocks, but making this genuinely atomic needs
    // every `db.*` verb inside to execute against a held
    // `sqlx::Transaction<'_, _>` instead of `self.db` — sqlx's
    // `Transaction` has no `Deref`-to-pool trick that lets
    // already-generated `.execute(self.db)` call sites transparently
    // keep working, so real atomicity means threading an executor
    // choice through the entire recursive lowering call graph (30+
    // sites: every arm that can nest a db verb, not just the db-verb
    // arms themselves) rather than the one `in_tx: bool` flag that
    // suffices for a `Statement`-oriented target's uniform session.
    // That's a materially larger, riskier change to code every
    // existing Rust example depends on, so it's tracked as a
    // follow-up rather than attempted here. Interim: lower the body
    // exactly as if unwrapped — correct per-statement, just not yet
    // atomic across the whole block. See docs/language.md's
    // transactions section.
    fn transaction_expr(&self, inner: &str) -> String {
        let note =
            "// NOTE: this block is not yet atomic on the Rust backend (see docs/language.md)";
        format!("{note}\n{inner}")
    }

    fn return_stmt(&self, value: Option<&str>, _indent: &str) -> String {
        match value {
            Some(v) => format!("return Ok({v});"),
            None => "return Ok(());".to_owned(),
        }
    }
    fn fail(&self, error: RecordId, args: &[String], _indent: &str) -> String {
        let name = record_class_name(self.ir, error);
        let record = self.ir.record(error);
        let field_inits: Vec<String> = record
            .fields
            .iter()
            .zip(args)
            .map(|(f, a)| format!("{}: {a}", f.name))
            .collect();
        format!(
            "return Err({name} {{ {} }}.into());",
            field_inits.join(", ")
        )
    }
    fn publish(&self, subject: &str, value: &str, _value_ty: &HirType, _indent: &str) -> String {
        format!("self.queue.publish({subject:?}, serde_json::to_vec(&{value})?).await?;")
    }
    fn db_get(&self, table: TableId, key: &str) -> String {
        let model = model_class_name(self.ir, table);
        let record_name = record_class_name(self.ir, self.ir.table(table).record);
        let table_snake = self.ir.table(table).name.to_snake_case();
        let record_ctx = context::build_record(self.ir, self.ir.table(table).record);
        format!(
            "{{\n    let row: Option<{model}> = sqlx::query_as(\"SELECT {cols} FROM {table_snake} WHERE id = {ph}\")\n        .bind({key}.to_string())\n        .fetch_optional(self.db)\n        .await?;\n    row.map({record_name}::try_from).transpose()?\n}}",
            cols = record_ctx.select_cols,
            ph = ciac_codegen::template::sqlph("$1", self.db_engine),
        )
    }
    fn cache_get(&self, key: &str) -> String {
        format!(
            "{{\n    let mut conn = self.cache.get_multiplexed_async_connection().await?;\n    let raw: Option<String> = conn.get(&{key}).await?;\n    match raw {{\n        Some(s) => Some(serde_json::from_str(&s)?),\n        None => None,\n    }}\n}}"
        )
    }
    fn cache_set(&self, key: &str, value: &str, _value_ty: &HirType) -> String {
        format!(
            "{{\n    let mut conn = self.cache.get_multiplexed_async_connection().await?;\n    let _: () = conn.set(&{key}, serde_json::to_string(&{value})?).await?;\n}}"
        )
    }
    fn cache_delete(&self, key: &str) -> String {
        format!(
            "{{\n    let mut conn = self.cache.get_multiplexed_async_connection().await?;\n    let _: () = conn.del(&{key}).await?;\n}}"
        )
    }
    fn object_store_put(&self, key: &str, value: &str, value_ty: &HirType) -> String {
        let payload = if matches!(value_ty, HirType::Record(_)) {
            format!("&serde_json::to_vec(&{value})?")
        } else {
            format!("{value}.to_string().as_bytes()")
        };
        format!("self.object_store.put(&{key}, {payload}).await?")
    }
    fn object_store_get(&self, key: &str) -> String {
        format!("serde_json::from_slice(&self.object_store.get(&{key}).await?)?")
    }
    fn object_store_delete(&self, key: &str) -> String {
        format!("self.object_store.delete(&{key}).await?")
    }
    fn object_store_list(&self, prefix: &str) -> String {
        format!("self.object_store.list(&{prefix}).await?")
    }
    fn email_send(&self, to: &str, subject: &str, body: &str) -> String {
        format!("self.email.send(&{to}, &{subject}, &{body}).await?")
    }
    fn search_index(&self, doc_id: &str, document: &str, document_ty: &HirType) -> String {
        let document = self.json_value(document, document_ty);
        format!("self.search.index({SEARCH_INDEX_NAME:?}, &{doc_id}, &{document}).await?")
    }
    fn search_query(&self, query: &str) -> String {
        format!(
            "self.search.search({SEARCH_INDEX_NAME:?}, &serde_json::json!({{\"query\": {{\"query_string\": {{\"query\": {query}}}}}}})).await?"
        )
    }
    fn http_call(&self, url: &str, json_body: &str, body_ty: &HirType) -> String {
        let json_val = self.json_value(json_body, body_ty);
        format!("self.http.post(&{url}).json(&{json_val}).send().await?.json::<serde_json::Value>().await?")
    }
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

    let db_engine = db_engine_of(ir, hir);
    let body = match &hir.body {
        Some(_) => {
            let syntax = RustSyntax { ir, db_engine };
            lower::lower_body_expr(&syntax, ir, hir)
        }
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
        db_engine: db_engine.to_owned(),
        extras,
        schema_imports,
        model_imports,
        body,
    }
}
