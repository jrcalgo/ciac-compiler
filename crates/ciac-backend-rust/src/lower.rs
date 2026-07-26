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
    LoweredPredTerm, LoweredPredicate, MatchArm, Orientation, PredValue, Wrap,
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
    /// 27UpdatePlan.md M4: this handler's own bound instance name for
    /// each peripheral capability, resolved once at construction (the
    /// same "handler-level, not per-verb-call" reasoning as
    /// `db_engine` above) -- `None` when the handler never calls that
    /// capability's verbs, in which case the corresponding leaf is
    /// never actually invoked either. Threaded through so the
    /// world-guard branch can call `SimWorld`'s instance-keyed
    /// methods (`world.cache_get(instance, key)`, ...) with the same
    /// instance name production code's own `get_cache(instance)`-
    /// shaped constructor already resolved.
    cache_instance: Option<String>,
    object_store_instance: Option<String>,
    email_instance: Option<String>,
    search_instance: Option<String>,
    http_instance: Option<String>,
    /// 27UpdatePlan.md M4: `Some(_)` while lowering a `transaction {}`
    /// block's world branch (see `HostSyntax::begin_world_batch`) --
    /// while set, `db_insert_expr`/`db_update_expr`/`db_delete_expr`
    /// push onto the generated code's own `__batch_ops` `Vec` instead
    /// of calling a `*_checked` method immediately, so the whole
    /// transaction commits atomically via one `commit_batch_checked`
    /// call assembled in `end_world_batch`.
    batching: std::cell::Cell<bool>,
}

impl RustSyntax<'_> {
    /// `26UpdatePlan.md` M1: the executor a db-verb leaf binds its
    /// query to — the pool (`self.db`) outside a transaction,
    /// exactly as before this milestone, or a reborrow of the held
    /// `sqlx::Transaction` (`&mut *__tx`, bound by
    /// [`HostSyntax::transaction_expr`]'s `real_branch` wrapper)
    /// inside one. A fresh `&mut *__tx` reborrow per call site (not a
    /// single captured `&mut` held across statements) is required
    /// because `sqlx::Transaction` isn't `Copy` and the borrow
    /// checker won't let one `&mut` outlive the statement that uses
    /// it.
    fn executor(in_tx: bool) -> &'static str {
        if in_tx {
            "&mut *__tx"
        } else {
            "self.db"
        }
    }

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

    /// 27UpdatePlan.md M4: the world-guard branch's own predicate
    /// evaluator for `db.query`/`db.count`/`db.delete_where`'s
    /// `where` clause -- `SimWorld::db.find_where` only matches
    /// equality (a `BTreeMap<String, Value>`), narrower than the
    /// production SQL this mirrors, so every term (not just `Eq`) is
    /// instead compiled into a generated Rust boolean expression
    /// evaluated per-row against the JSON document `find_where`
    /// returns -- `true` (matches everything) for no predicate at
    /// all. Numeric comparisons go through `f64` uniformly (matching
    /// JSON's own single numeric representation), a disclosed
    /// simplification, not a precision guarantee the fake claims.
    fn world_predicate_expr(&self, predicate: Option<&LoweredPredicate>, row_var: &str) -> String {
        let Some(predicate) = predicate else {
            return "true".to_owned();
        };
        predicate
            .terms
            .iter()
            .map(|term| self.world_predicate_term_expr(term, row_var))
            .collect::<Vec<_>>()
            .join(" && ")
    }
    fn world_predicate_term_expr(&self, term: &LoweredPredTerm, row_var: &str) -> String {
        let field = &term.field;
        let value_json = match &term.value {
            PredValue::EnumVariant(v) => format!("serde_json::json!({v:?})"),
            PredValue::BoolLit(b) => format!("serde_json::json!({b})"),
            PredValue::Rendered(s) => {
                if term.field_ty == HirType::Uuid {
                    format!("serde_json::json!({s}.to_string())")
                } else {
                    format!("serde_json::json!({s})")
                }
            }
        };
        match term.op {
            PredOp::Eq => format!("{row_var}.get({field:?}) == Some(&{value_json})"),
            PredOp::NotEq => format!("{row_var}.get({field:?}) != Some(&{value_json})"),
            PredOp::Contains => format!(
                "{row_var}.get({field:?}).and_then(|v| v.as_str()).map(|v| v.contains(({value_json}).as_str().unwrap_or(\"\"))).unwrap_or(false)"
            ),
            PredOp::Lt | PredOp::LtEq | PredOp::Gt | PredOp::GtEq => {
                let op_str = match term.op {
                    PredOp::Lt => "<",
                    PredOp::LtEq => "<=",
                    PredOp::Gt => ">",
                    PredOp::GtEq => ">=",
                    _ => unreachable!(),
                };
                format!(
                    "{row_var}.get({field:?}).and_then(|v| v.as_f64()).zip(({value_json}).as_f64()).map(|(a, b)| a {op_str} b).unwrap_or(false)"
                )
            }
        }
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
    /// 27UpdatePlan.md M4: the instance name a `cache.*` verb call's
    /// world-guard branch resolves against -- `.expect()`, not a
    /// fallback, because sema already refuses a `cache.*` call with no
    /// bound `cache` capability before codegen ever runs; reaching
    /// here with `None` would be a compiler bug, not a user error.
    fn cache_instance(&self) -> &str {
        self.cache_instance
            .as_deref()
            .expect("a handler calling a cache verb has a bound cache instance, per sema")
    }
    fn object_store_instance(&self) -> &str {
        self.object_store_instance
            .as_deref()
            .expect("a handler calling an object_store verb has a bound instance, per sema")
    }
    fn search_instance(&self) -> &str {
        self.search_instance
            .as_deref()
            .expect("a handler calling a search verb has a bound instance, per sema")
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
    fn db_insert_expr(&self, table: TableId, value: &str, in_tx: bool) -> String {
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
        // failure-injected) instead of ever touching `self.db`. Kept
        // here unchanged even when `in_tx` (`26UpdatePlan.md` M1):
        // this whole leaf only ever renders inside
        // `transaction_expr`'s `real_branch`, where `self.world` is
        // already known `None` at generated-code runtime (the
        // enclosing `if !self.world.is_some()` already chose this
        // branch) — so this nested check is provably dead on the
        // `Some` arm and harmlessly redundant, not wrong; leaving it
        // in place (rather than threading a third "world already
        // resolved" state through every leaf) is what keeps this
        // milestone's diff to "swap the executor", not "restructure
        // every verb's world-guard".
        //
        // 27UpdatePlan.md M4: while `self.batching` is set (rendering
        // a `transaction {}` block's world branch, see
        // `begin_world_batch`), this pushes onto the generated code's
        // own `__batch_ops` `Vec` instead of calling
        // `db_insert_checked` immediately, so the whole transaction
        // commits atomically via one `commit_batch_checked` call
        // (`end_world_batch`) rather than each insert applying (and
        // being unable to roll back) on its own.
        if self.batching.get() {
            return format!(
                "{{\n    let __row = {value}.clone();\n    __batch_ops.push(crate::world::BatchOp::Insert {{ table: {table_snake:?}.into(), row: serde_json::to_value(&__row)? }});\n    __row\n}}"
            );
        }
        format!(
            "{{\n    let __row = {value}.clone();\n    if let Some(world) = self.world {{\n        world.db_insert_checked(\"{table_snake}\", serde_json::to_value(&__row)?)?;\n    }} else {{\n        sqlx::query(\"INSERT INTO {table_snake} ({cols}) VALUES ({phs})\"){binds}\n            .execute({executor})\n            .await?;\n    }}\n    __row\n}}",
            cols = record_ctx.select_cols,
            phs = ciac_codegen::template::sqlph(&record_ctx.insert_placeholders, self.db_engine),
            executor = Self::executor(in_tx),
        )
    }
    fn db_update_expr(&self, table: TableId, key: &str, value: &str, in_tx: bool) -> String {
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
        //
        // 27UpdatePlan.md M4: world-guard added -- `db_update_checked`
        // does the same full-record replace `SET` production SQL
        // performs, `None` for a missing row exactly like
        // `rows_affected() == 0`. Batching mode (see `db_insert_expr`
        // above) pushes a `BatchOp::Update` instead.
        if self.batching.get() {
            return format!(
                "{{\n    let __row = {value}.clone();\n    __batch_ops.push(crate::world::BatchOp::Update {{ table: {table_snake:?}.into(), pk: {key}.to_string(), row: serde_json::to_value(&__row)? }});\n    Some(__row)\n}}"
            );
        }
        format!(
            "{{\n    let __row = {value}.clone();\n    if let Some(world) = self.world {{\n        match world.db_update_checked({table_snake:?}, &{key}.to_string(), serde_json::to_value(&__row)?)? {{\n            Some(_) => Some(__row),\n            None => None,\n        }}\n    }} else {{\n        let __updated = sqlx::query(\"UPDATE {table_snake} SET {assignments} WHERE id = {where_ph}\"){binds}\n            .bind({key}.to_string())\n            .execute({executor})\n            .await?;\n        if __updated.rows_affected() == 0 {{ None }} else {{ Some(__row) }}\n    }}\n}}",
            assignments = ciac_codegen::template::sqlph(&record_ctx.update_assignments, self.db_engine),
            where_ph = ciac_codegen::template::sqlph(&record_ctx.update_where, self.db_engine),
            executor = Self::executor(in_tx),
        )
    }
    fn db_delete_expr(&self, table: TableId, key: &str, in_tx: bool) -> String {
        let table_snake = self.ir.table(table).name.to_snake_case();
        // 27UpdatePlan.md M4: world-guard added -- `db_delete_checked`
        // resolves cascade/restrict references the same way
        // production's `ON DELETE` behavior would (enforced at the
        // schema level, not per-statement). Batching mode pushes a
        // `BatchOp::Delete`; the actual "was a row deleted" outcome
        // isn't known until the transaction commits, so this
        // optimistically returns `true` (matching the common case,
        // same simplification `commit_batch_checked`'s own caller in
        // `ciac-sim`'s test suite makes for a same-batch delete).
        if self.batching.get() {
            return format!(
                "{{\n    __batch_ops.push(crate::world::BatchOp::Delete {{ table: {table_snake:?}.into(), pk: {key}.to_string() }});\n    true\n}}"
            );
        }
        format!(
            "if let Some(world) = self.world {{\n    world.db_delete_checked({table_snake:?}, &{key}.to_string())?\n}} else {{\n    let __deleted = sqlx::query(\"DELETE FROM {table_snake} WHERE id = {ph}\")\n        .bind({key}.to_string())\n        .execute({executor})\n        .await?;\n    __deleted.rows_affected() > 0\n}}",
            ph = ciac_codegen::template::sqlph("$1", self.db_engine),
            executor = Self::executor(in_tx),
        )
    }
    fn query_expr(&self, verb: Verb, predicate: Option<&LoweredPredicate>, in_tx: bool) -> String {
        let executor = Self::executor(in_tx);
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
                let world_pred = self.world_predicate_expr(predicate, "__row");
                format!(
                    "if let Some(world) = self.world {{\n    world.db.find_where({table_snake:?}, &Default::default())\n        .into_iter()\n        .filter(|__row| {world_pred})\n        .map(serde_json::from_value::<{record_name}>)\n        .collect::<Result<Vec<_>, _>>()?\n}} else {{\n    let __rows: Vec<{model}> = sqlx::query_as(\"SELECT {cols} FROM {table_snake}{where_sql}\"){bind_lines}\n        .fetch_all({executor})\n        .await?;\n    __rows.into_iter().map({record_name}::try_from).collect::<Result<Vec<_>, _>>()?\n}}",
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
                let world_pred = self.world_predicate_expr(predicate, "__row");
                format!(
                    "if let Some(world) = self.world {{\n    world.db.find_where({table_snake:?}, &Default::default())\n        .into_iter()\n        .filter(|__row| {world_pred})\n        .count() as i64\n}} else {{\n    let __count: i64 = sqlx::query_scalar(\"SELECT COUNT(*) FROM {table_snake}{where_sql}\"){bind_lines}\n        .fetch_one({executor})\n        .await?;\n    __count\n}}"
                )
            }
            Verb::DbDeleteWhere(table) => {
                let table_snake = self.ir.table(table).name.to_snake_case();
                let (where_sql, binds) = self.where_clause(predicate);
                let bind_lines: String = binds
                    .iter()
                    .map(|b| format!("\n        .bind({b})"))
                    .collect();
                let world_pred = self.world_predicate_expr(predicate, "__row");
                // 27UpdatePlan.md M4: `SimWorld` has no bulk
                // delete-by-predicate method (`commit_batch_checked`
                // takes explicit `(table, pk)` pairs, not a filter),
                // so the world branch resolves matching ids first,
                // then deletes each through `db_delete_checked` (real
                // cascade/restrict handling, real failure-injection)
                // -- a disclosed divergence from production's single
                // bulk `DELETE`: a failure rule keyed on this table's
                // `db.commit` can fire more than once for one
                // `delete_where` call under simulation, where
                // production's single statement would only ever fail
                // (or succeed) once. No checked-in example combines
                // `delete_where` with failure injection today.
                format!(
                    "if let Some(world) = self.world {{\n    let __matching: Vec<String> = world.db.find_where({table_snake:?}, &Default::default())\n        .into_iter()\n        .filter(|__row| {world_pred})\n        .filter_map(|__row| __row.get(\"id\").and_then(|v| v.as_str()).map(str::to_owned))\n        .collect();\n    let mut __n = 0i64;\n    for __pk in &__matching {{\n        if world.db_delete_checked({table_snake:?}, __pk)? {{\n            __n += 1;\n        }}\n    }}\n    __n\n}} else {{\n    let __deleted = sqlx::query(\"DELETE FROM {table_snake}{where_sql}\"){bind_lines}\n        .execute({executor})\n        .await?;\n    __deleted.rows_affected() as i64\n}}"
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
    // `26UpdatePlan.md` M1: real atomicity, closing the gap v0.16 M6
    // assessed and deliberately deferred. The blocker then was real:
    // sqlx's `Transaction` has no `Deref`-to-pool trick, so a
    // `.execute(self.db)` call site can't transparently start
    // executing against a transaction instead. What made it
    // tractable now, on inspection, is that the "30+ sites" the v0.16
    // assessment worried about turned out not to exist: every `db.*`
    // verb call is lowered directly from `lower_expr_any`'s own match
    // arms (never nested inside another verb's arguments — the HIR
    // only ever nests *scalars* inside a verb call, and scalars
    // cannot themselves be db verbs), so threading `in_tx: bool`
    // through exactly the three functions that already recurse across
    // statement/block boundaries (`lower_expr_any`, `lower_block_expr`,
    // `lower_stmt_expr` — the Expression-orientation analogs of
    // `Statement`-orientation's existing `lower_tail`/`lower_block_stmt`/
    // `lower_stmt`, which have threaded `in_tx` since the factory
    // arcs) reaches every leaf that needs it. See
    // `HostSyntax::transaction_expr`'s doc for why the body is
    // rendered twice.
    fn transaction_expr(&self, world_branch: &str, real_branch: &str) -> String {
        // 27UpdatePlan.md M4: `world` is now bound (`if let Some(world)`,
        // not just `.is_some()`) so `end_world_batch`'s appended
        // `world.commit_batch_checked(..)` call (already folded into
        // `world_branch` by the time this runs) has something to call
        // on.
        format!(
            "if let Some(world) = self.world {{\n{}\n}} else {{\n    let mut __tx = self.db.begin().await?;\n{}\n    __tx.commit().await?;\n}}",
            indent_lines(world_branch, "    "),
            indent_lines(real_branch, "    "),
        )
    }
    fn begin_world_batch(&self) {
        self.batching.set(true);
    }
    /// 27UpdatePlan.md M4: wraps the just-rendered world branch (whose
    /// own `db.insert`/`update`/`delete` calls pushed onto
    /// `__batch_ops` instead of writing immediately, per
    /// `begin_world_batch`) with the accumulator's declaration and one
    /// final atomic `world.commit_batch_checked(..)` call -- the
    /// transaction's real atomicity under simulation, retiring the
    /// "degraded per-verb shape" every prior milestone's own
    /// disclosure named. `world` here resolves to the binding
    /// `transaction_expr`'s `if let Some(world) = self.world` wrapper
    /// introduces around this whole block.
    fn end_world_batch(&self, world_branch: &str) -> String {
        self.batching.set(false);
        format!(
            "let mut __batch_ops: Vec<crate::world::BatchOp> = Vec::new();\n{world_branch}\nworld.commit_batch_checked(__batch_ops)?;"
        )
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
            "if let Some(world) = self.world {{\n    world.db.get({table_snake:?}, &{key}.to_string()).map(serde_json::from_value::<{record_name}>).transpose()?\n}} else {{\n    let row: Option<{model}> = sqlx::query_as(\"SELECT {cols} FROM {table_snake} WHERE id = {ph}\")\n        .bind({key}.to_string())\n        .fetch_optional(self.db)\n        .await?;\n    row.map({record_name}::try_from).transpose()?\n}}",
            cols = record_ctx.select_cols,
            ph = ciac_codegen::template::sqlph("$1", self.db_engine),
        )
    }
    fn cache_get(&self, key: &str) -> String {
        let instance = self.cache_instance();
        format!(
            "if let Some(world) = self.world {{\n    match world.cache_get({instance:?}, &{key}) {{\n        Some(s) => Some(serde_json::from_str(&s)?),\n        None => None,\n    }}\n}} else {{\n    let mut conn = self.cache.get_multiplexed_async_connection().await?;\n    let raw: Option<String> = conn.get(&{key}).await?;\n    match raw {{\n        Some(s) => Some(serde_json::from_str(&s)?),\n        None => None,\n    }}\n}}"
        )
    }
    fn cache_set(&self, key: &str, value: &str, _value_ty: &HirType) -> String {
        let instance = self.cache_instance();
        format!(
            "if let Some(world) = self.world {{\n    world.cache_set({instance:?}, &{key}, serde_json::to_string(&{value})?, None);\n}} else {{\n    let mut conn = self.cache.get_multiplexed_async_connection().await?;\n    let _: () = conn.set(&{key}, serde_json::to_string(&{value})?).await?;\n}}"
        )
    }
    fn cache_delete(&self, key: &str) -> String {
        let instance = self.cache_instance();
        format!(
            "if let Some(world) = self.world {{\n    world.cache_delete({instance:?}, &{key});\n}} else {{\n    let mut conn = self.cache.get_multiplexed_async_connection().await?;\n    let _: () = conn.del(&{key}).await?;\n}}"
        )
    }
    fn object_store_put(&self, key: &str, value: &str, value_ty: &HirType) -> String {
        let instance = self.object_store_instance();
        let (real_payload, world_payload) = if matches!(value_ty, HirType::Record(_)) {
            (
                format!("&serde_json::to_vec(&{value})?"),
                format!("serde_json::to_vec(&{value})?"),
            )
        } else {
            (
                format!("{value}.to_string().as_bytes()"),
                format!("{value}.to_string().into_bytes()"),
            )
        };
        format!(
            "if let Some(world) = self.world {{\n    world.object_put({instance:?}, &{key}, {world_payload});\n}} else {{\n    self.object_store.put(&{key}, {real_payload}).await?\n}}"
        )
    }
    fn object_store_get(&self, key: &str) -> String {
        let instance = self.object_store_instance();
        format!(
            "if let Some(world) = self.world {{\n    serde_json::from_slice(&world.object_get({instance:?}, &{key})?)?\n}} else {{\n    serde_json::from_slice(&self.object_store.get(&{key}).await?)?\n}}"
        )
    }
    fn object_store_delete(&self, key: &str) -> String {
        let instance = self.object_store_instance();
        format!(
            "if let Some(world) = self.world {{\n    world.object_delete({instance:?}, &{key});\n}} else {{\n    self.object_store.delete(&{key}).await?\n}}"
        )
    }
    fn object_store_list(&self, prefix: &str) -> String {
        let instance = self.object_store_instance();
        format!(
            "if let Some(world) = self.world {{\n    world.object_list({instance:?}, &{prefix})\n}} else {{\n    self.object_store.list(&{prefix}).await?\n}}"
        )
    }
    fn email_send(&self, to: &str, subject: &str, body: &str) -> String {
        let instance = self
            .email_instance
            .as_deref()
            .expect("a handler calling an email verb has a bound instance, per sema");
        format!(
            "if let Some(world) = self.world {{\n    world.email_send({instance:?}, &{to}, &{subject}, &{body});\n}} else {{\n    self.email.send(&{to}, &{subject}, &{body}).await?\n}}"
        )
    }
    fn search_index(&self, doc_id: &str, document: &str, document_ty: &HirType) -> String {
        let instance = self.search_instance();
        let document = self.json_value(document, document_ty);
        format!(
            "if let Some(world) = self.world {{\n    world.search_index({instance:?}, &{doc_id}, {document});\n}} else {{\n    self.search.index({SEARCH_INDEX_NAME:?}, &{doc_id}, &{document}).await?\n}}"
        )
    }
    fn search_query(&self, query: &str) -> String {
        let instance = self.search_instance();
        format!(
            "if let Some(world) = self.world {{\n    world.search_query({instance:?}, &({query}).to_string())\n}} else {{\n    self.search.search({SEARCH_INDEX_NAME:?}, &serde_json::json!({{\"query\": {{\"query_string\": {{\"query\": {query}}}}}}})).await?\n}}"
        )
    }
    fn http_call(&self, url: &str, json_body: &str, body_ty: &HirType) -> String {
        let instance = self
            .http_instance
            .as_deref()
            .expect("a handler calling an external_http verb has a bound instance, per sema");
        let json_val = self.json_value(json_body, body_ty);
        format!(
            "if let Some(world) = self.world {{\n    world.http_post({instance:?}, &{url}, {json_val})?\n}} else {{\n    self.http.post(&{url}).json(&{json_val}).send().await?.json::<serde_json::Value>().await?\n}}"
        )
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
    let instance_of = |kind: &str| {
        bindings
            .iter()
            .find(|b| b.kind == kind)
            .map(|b| b.name.clone())
    };
    let body = match &hir.body {
        Some(_) => {
            let syntax = RustSyntax {
                ir,
                db_engine,
                cache_instance: instance_of("cache"),
                object_store_instance: instance_of("object_store"),
                email_instance: instance_of("email"),
                search_instance: instance_of("search"),
                http_instance: instance_of("external_http"),
                batching: std::cell::Cell::new(false),
            };
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
