//! Direct lowering of the typed HIR (`ciac_ir::hir`) into TypeScript
//! source (`23UpdatePlan.md` M4).
//!
//! The walker (block/tail shaping, precedence, enum-literal use-site
//! recovery, float-literal fidelity, divergence truncation) lives in
//! `ciac_codegen::lower`; [`TsSyntax`] supplies only the leaf
//! constructors genuinely specific to this target. TS runs in
//! `Orientation::Statement` — the same mode Python exercises — because
//! a `{}` block is not an expression in JS/TS the way it is in Rust, so
//! `if`/`match`/the statement-shaped `db.*` verbs decompose into a line
//! sequence applied to a [`Dest`] exactly like Python's own
//! `lower_tail`. Two things this target needs that Python doesn't:
//!
//! - Raw parameterized SQL (not an ORM), reached through Drizzle's
//!   `$client` escape hatch (`state.ts.j2`'s own established
//!   convention — see `db.ts.j2`'s migration runner), following the
//!   Rust backend's bind-order discipline (`ciac_codegen::template::sqlph`)
//!   rather than Python's SQLAlchemy `.where(..)` chain — Pillar 4 is
//!   explicit that this backend "adds zero new placeholder logic".
//! - A hoisted `let` declaration per HIR `Let` binding, computed once
//!   in [`render`] rather than emitted per `assign()` call: unlike
//!   Python (whose `if`/`else` share the enclosing scope), TS's
//!   `if {} else {}` introduces a real block scope, so a `let`/`const`
//!   declared *inside* each branch of a value-producing `if`/`match`
//!   would be invisible after the block. Declaring the name once above
//!   the branch and using a bare `name = value;` at every `Dest::Assign`
//!   site (mirroring Python's own keyword-free `assign()` spelling)
//!   sidesteps that regardless of nesting depth.

use ciac_codegen::lower::{
    self, fidelity_checked_float, strip_outer_parens, Dest, HostSyntax, IndexKey, LoweredPredTerm,
    LoweredPredicate, Orientation, PredValue,
};
use ciac_codegen::model::{self as context, FieldCtx, FieldTypeKind, RecordCtx};
use ciac_codegen::template::sqlph;
use ciac_ir::{
    BinOp, HandlerBody, HirExpr, HirStmt, HirType, NormalizedIr, PredOp, RecordId, TableId, UnOp,
    Verb,
};
use heck::ToSnakeCase;
use serde::Serialize;

/// TypeScript in-memory type annotation for a HIR type (Pillar 2's
/// mapping table) — a handler *signature* concern, not part of the
/// `HostSyntax` body contract.
pub fn ts_type(ir: &NormalizedIr, ty: &HirType) -> String {
    match ty {
        HirType::Str | HirType::Uuid => "string".to_owned(),
        HirType::Int | HirType::Float => "number".to_owned(),
        HirType::Bool => "boolean".to_owned(),
        HirType::Timestamp => "Date".to_owned(),
        HirType::Json => "unknown".to_owned(),
        HirType::Enum { variants } => variants
            .iter()
            .map(|v| format!("{v:?}"))
            .collect::<Vec<_>>()
            .join(" | "),
        HirType::Record(id) => ir.record(*id).name.clone(),
        HirType::Option(inner) => format!("{} | null", ts_type(ir, inner)),
        HirType::List(inner) => format!("{}[]", ts_type(ir, inner)),
        HirType::Unit | HirType::Never => "void".to_owned(),
    }
}

fn record_class_name(ir: &NormalizedIr, record: RecordId) -> String {
    ir.record(record).name.clone()
}

/// `local_name`, duplicated from `ciac_codegen::lower::dispatch` (a
/// private helper there): a [`HirExpr::Local`]'s declared name when it
/// names a parameter, else a synthesized `v<slot>` for a `let` local.
/// Needed here (not just inside the shared walker) to build the
/// hoisted-`let` declaration list — see the module doc.
fn local_name(body: &HandlerBody, slot: u32) -> String {
    let slot = slot as usize;
    if slot < body.params.len() {
        body.params[slot].0.clone()
    } else {
        format!("v{slot}")
    }
}

/// Collects every name a `HirStmt::Let` binds *whose value is an `if`/
/// `match` expression* — the only shape where `Dest::Assign(name)`
/// reaches more than one branch, each in its own block scope. Those
/// names need a `let` hoisted above the branch and a bare `name =
/// value;` at each branch's own assignment site (see the module doc);
/// every other `Let` assigns exactly once and gets a plain `const
/// name = value;` at that one site instead — collecting it here too
/// would make `assign()` hoist it needlessly, tripping `eslint`'s
/// `prefer-const` (live-caught on `query-verbs.ciac`'s `Replace`,
/// whose `let n = Note { .. }` never branches). Recurses into `if`/
/// `match` branch bodies and `transaction {}`'s inner block — every
/// site [`ciac_codegen::lower::lower_block_stmt`] itself recurses into.
fn collect_branching_lets(body: &HandlerBody, stmts: &[HirStmt], out: &mut Vec<String>) {
    for stmt in stmts {
        match stmt {
            HirStmt::Let { slot, value } => {
                if matches!(value, HirExpr::If { .. } | HirExpr::Match { .. }) {
                    out.push(local_name(body, *slot));
                }
                collect_branching_lets_expr(body, value, out);
            }
            HirStmt::Expr(e) => collect_branching_lets_expr(body, e, out),
            HirStmt::Return(Some(e)) => collect_branching_lets_expr(body, e, out),
            HirStmt::Return(None) | HirStmt::Fail { .. } => {}
            HirStmt::Publish { value, .. } => collect_branching_lets_expr(body, value, out),
            HirStmt::Transaction { body: inner } => collect_branching_lets(body, inner, out),
        }
    }
}

fn collect_branching_lets_expr(body: &HandlerBody, expr: &HirExpr, out: &mut Vec<String>) {
    match expr {
        HirExpr::If {
            then_branch,
            else_branch,
            ..
        } => {
            collect_branching_lets(body, then_branch, out);
            collect_branching_lets(body, else_branch, out);
        }
        HirExpr::Match { arms, .. } => {
            for arm in arms {
                collect_branching_lets(body, &arm.body, out);
            }
        }
        _ => {}
    }
}

/// The `Orientation::Statement` `HostSyntax` implementation for this
/// target: `if`/`match`/the statement-shaped `db.*` verbs decompose
/// into a line sequence applied to a `Dest`, same as Python. Holds
/// `db_engine`/`db_field` (resolved once per handler, since every
/// db-verb leaf needs the same engine dialect and `AppState` field) and
/// `cache_field` similarly.
struct TsSyntax<'a> {
    ir: &'a NormalizedIr,
    db_engine: &'static str,
    db_field: Option<String>,
    cache_field: Option<String>,
    /// `AppState` field of this handler's bound ontology instance, one
    /// per capability kind — resolved the same way as `db_field`/
    /// `cache_field` (via `context::extras_of`, over the same
    /// `hir_bindings` this handler already computes), reused as-is
    /// rather than re-derived: `ExtraDepCtx.rust_state_field` is
    /// already just the raw `AppState` property name (e.g.
    /// `object_store`, `object_store_media`, `http_upstream`),
    /// language-agnostic despite the `rust_` prefix in its own field
    /// name — the same reuse `db_field`/`cache_field` already make of
    /// `Access::rust_db_field`/`rust_cache_field`.
    object_store_field: Option<String>,
    email_field: Option<String>,
    search_field: Option<String>,
    http_field: Option<String>,
    /// 27UpdatePlan.md M6: the capability instance's own *declared*
    /// name (e.g. `"Sessions"` from `cache Sessions`), not the
    /// `AppState` field it resolves to -- `SimWorld`'s cache/object
    /// store/email/search/http maps are keyed by this name, matching
    /// `given.cache`/`given.store`/etc.'s own `instance` field and
    /// Rust's own `cache_instance`/`object_store_instance`/etc.
    cache_instance: Option<String>,
    object_store_instance: Option<String>,
    email_instance: Option<String>,
    search_instance: Option<String>,
    http_instance: Option<String>,
    /// A handler body can call more than one statement-shaped db verb
    /// within the same block scope (e.g. two `db.insert`s inside one
    /// `transaction {}` — live-caught: two `const __row = ..;`
    /// declarations at the same scope is a real `SyntaxError`, not a
    /// style nit). Every temp this module declares directly into the
    /// caller's block (not inside its own IIFE, which already has its
    /// own scope) is suffixed with a number from this counter instead
    /// of reusing a bare name.
    tmp: std::cell::Cell<u32>,
    /// Names `collect_branching_lets` found — see its own doc and
    /// `assign`'s.
    branching_locals: std::collections::HashSet<String>,
    /// 28UpdatePlan.md M7a: this handler's own service, `None` in
    /// single-service mode -- mirrors `ciac-backend-rust`'s `RustSyntax.
    /// service_name` exactly (see that field's own doc comment). Feeds
    /// `world_table_key` below so a multi-service system's world-guard
    /// db verbs compose `"{service}::{table}"` instead of colliding
    /// across two same-named tables in different services.
    service_name: Option<String>,
}

impl TsSyntax<'_> {
    /// Composes the namespaced key every world-guarded db verb below
    /// addresses through -- see `RustSyntax::world_table_key`'s
    /// identical doc comment (`ciac-backend-rust/src/lower.rs`) for the
    /// full rationale; this is a codegen-time (Rust) computation
    /// embedding a literal string into the generated TypeScript, not a
    /// runtime call into `namespacedTableKey` (that runtime function
    /// exists for the system-runner's own scenario-driven seeding/
    /// assertions, which don't know the service name until they parse
    /// the scenario JSON).
    fn world_table_key(&self, table_snake: &str) -> String {
        match &self.service_name {
            Some(service) => format!("{service}::{table_snake}"),
            None => table_snake.to_owned(),
        }
    }

    fn fresh(&self, base: &str) -> String {
        let n = self.tmp.get();
        self.tmp.set(n + 1);
        format!("{base}{n}")
    }

    /// The raw-driver handle a db verb reaches through: the pool's
    /// `$client` escape hatch normally, or the transaction-scoped
    /// checked-out connection (`__tx`) when `in_tx` — see
    /// `transaction_stmt`'s own doc for why SQLite never needs the
    /// `__tx` branch (its `.transaction()` wrapper runs its callback
    /// against the same handle synchronously; only Postgres/MySQL's
    /// pool-of-connections model needs a dedicated checkout for real
    /// cross-statement atomicity). The `!` on `__tx` (v0.23 M9) is safe
    /// even though `transaction_stmt` only assigns `__tx` when
    /// `!this.state.world`: this handle is reached by an unguarded db
    /// verb (`db.update`/`db.get`/...) that `unsupportedSimCapabilities`
    /// already refuses `ciac sim` from ever driving, so the assertion's
    /// one lie (referencing `__tx` while it's `undefined`) is provably
    /// unreachable, matching Rust's own "falls straight through to
    /// real infrastructure" disclosure for the same unguarded verbs.
    fn handle(&self, in_tx: bool) -> String {
        if in_tx && self.db_engine != "sqlite" {
            "__tx!".to_owned()
        } else {
            format!(
                "this.state.{}.$client",
                self.db_field
                    .as_deref()
                    .expect("a db verb requires a bound database instance")
            )
        }
    }

    fn cache_handle(&self) -> String {
        format!(
            "this.state.{}",
            self.cache_field
                .as_deref()
                .expect("a cache verb requires a bound cache instance")
        )
    }

    fn object_store_state_field(&self) -> &str {
        self.object_store_field
            .as_deref()
            .expect("an object_store verb requires a bound object_store instance")
    }

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
    fn email_instance(&self) -> &str {
        self.email_instance
            .as_deref()
            .expect("a handler calling an email verb has a bound instance, per sema")
    }
    fn search_instance(&self) -> &str {
        self.search_instance
            .as_deref()
            .expect("a handler calling a search verb has a bound instance, per sema")
    }
    fn http_instance(&self) -> &str {
        self.http_instance
            .as_deref()
            .expect("a handler calling an external_http verb has a bound instance, per sema")
    }

    /// 27UpdatePlan.md M6: compiles a `LoweredPredicate` into a JS
    /// boolean expression over an in-memory `world.ts` row (a plain
    /// object, not a SQL row) -- `SimWorld.db.findWhere`'s own filter
    /// only ever supports equality, so `db.query`/`db.count`/
    /// `db.delete_where`'s world branches filter the *unconstrained*
    /// row set themselves, mirroring Rust's own `world_predicate_expr`.
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

    /// The `.filter((__r) => ..)` a world-branch query chains onto its
    /// `findWhere(..)`, or the empty string when the predicate is
    /// trivially true.
    ///
    /// An unfiltered verb (`db.count(T)` with no `where`) used to emit
    /// `.filter((__r) => true)`: a no-op whose bound `__r` is never
    /// read, which `@typescript-eslint/no-unused-vars` rejects — so
    /// `ciac verify --target typescript` failed its own lint step on
    /// any program containing one. Dropping the call entirely is both
    /// the fix and the more faithful lowering; the production branch's
    /// SQL has no `WHERE` clause in exactly this case either.
    fn world_filter_suffix(&self, predicate: Option<&LoweredPredicate>, row_var: &str) -> String {
        match predicate {
            Some(predicate) if !predicate.terms.is_empty() => {
                let expr = self.world_predicate_expr(Some(predicate), row_var);
                format!(".filter(({row_var}) => {expr})")
            }
            _ => String::new(),
        }
    }
    fn world_predicate_term_expr(&self, term: &LoweredPredTerm, row_var: &str) -> String {
        let field = &term.field;
        let value_expr = match &term.value {
            PredValue::EnumVariant(v) => format!("{v:?}"),
            PredValue::BoolLit(b) => b.to_string(),
            PredValue::Rendered(s) => s.clone(),
        };
        match term.op {
            PredOp::Eq => format!("{row_var}[{field:?}] === ({value_expr})"),
            PredOp::NotEq => format!("{row_var}[{field:?}] !== ({value_expr})"),
            PredOp::Contains => {
                format!("String({row_var}[{field:?}] ?? \"\").includes(String({value_expr}))")
            }
            PredOp::Lt | PredOp::LtEq | PredOp::Gt | PredOp::GtEq => {
                let op_str = match term.op {
                    PredOp::Lt => "<",
                    PredOp::LtEq => "<=",
                    PredOp::Gt => ">",
                    PredOp::GtEq => ">=",
                    _ => unreachable!(),
                };
                format!("Number({row_var}[{field:?}]) {op_str} Number({value_expr})")
            }
        }
    }

    /// A field's write-side bind expression: SQLite's driver rejects a
    /// bare JS `boolean` bind param outright (live-verified: `SQLite3
    /// can only bind numbers, strings, bigints, buffers, and null`),
    /// has no native JSON type (columns are `TEXT`, per `sql_ddl_type`/
    /// `drizzle_column`'s own established engine mapping), and stores
    /// `Timestamp` as `TEXT` too — Postgres/MySQL's drivers accept a
    /// real `boolean`/object/`Date` directly for their native column
    /// types, so only the SQLite arms need a coercion.
    fn bind_expr(&self, field: &FieldCtx, base: &str) -> String {
        let access = format!("{base}.{}", field.name);
        match (&field.type_kind, self.db_engine) {
            (FieldTypeKind::Bool, "sqlite") => format!("({access} ? 1 : 0)"),
            (FieldTypeKind::Json, "sqlite") => format!("JSON.stringify({access})"),
            (FieldTypeKind::Timestamp, "sqlite") => format!("{access}.toISOString()"),
            _ => access,
        }
    }

    /// The inverse of `bind_expr` for a raw row read back from SQLite
    /// (its columns are typed `unknown` by better-sqlite3 without a
    /// cast — see `row_cast_type` — so every field needs an explicit
    /// coercion back to its in-memory type; Postgres/MySQL's drivers
    /// already hand back a real `boolean`/parsed-JSON value/`Date`, so
    /// only the SQLite arms differ from a bare property read).
    fn map_row_field(&self, field: &FieldCtx, row_var: &str) -> String {
        let access = format!("{row_var}.{}", field.name);
        match (&field.type_kind, self.db_engine) {
            (FieldTypeKind::Bool, "sqlite") => format!("Boolean({access})"),
            (FieldTypeKind::Timestamp, "sqlite") => format!("new Date({access} as string)"),
            (FieldTypeKind::Json, "sqlite") => format!("JSON.parse({access} as string)"),
            _ => access,
        }
    }

    fn map_row_expr(&self, record: &RecordCtx, row_var: &str) -> String {
        let fields: Vec<String> = record
            .fields
            .iter()
            .map(|f| format!("{}: {}", f.name, self.map_row_field(f, row_var)))
            .collect();
        format!("{{ {} }}", fields.join(", "))
    }

    /// better-sqlite3's `.get()`/`.all()` return `unknown`/`unknown[]`
    /// without an explicit cast (confirmed live and matching
    /// `db.ts.j2`'s own established `(row as { name: string }).name`
    /// pattern for the migrations ledger) — this is that cast's type,
    /// spelled in the column's *raw storage* shape (bool/timestamp/json
    /// are all stored as `number`/`string`/`string` on SQLite; see
    /// `bind_expr`), not its in-memory `ts_type`.
    fn row_cast_type(&self, record: &RecordCtx) -> String {
        let fields: Vec<String> = record
            .fields
            .iter()
            .map(|f| {
                let ty = match f.type_kind {
                    FieldTypeKind::Bool | FieldTypeKind::Int | FieldTypeKind::Float => "number",
                    _ => "string",
                };
                format!("{}: {ty}", f.name)
            })
            .collect();
        format!("{{ {} }}", fields.join("; "))
    }

    /// Builds a ` WHERE ..` clause (empty string if there's no
    /// predicate) and the ordered bind expressions it needs, written
    /// Postgres-style (`$N`) and rewritten per-engine by `sqlph`, same
    /// as every other SQL string this backend emits (v0.13 M1's
    /// discipline, unchanged).
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
                PredValue::BoolLit(b) => {
                    if self.db_engine == "sqlite" {
                        if *b { "1" } else { "0" }.to_owned()
                    } else if *b {
                        "true".to_owned()
                    } else {
                        "false".to_owned()
                    }
                }
                PredValue::Rendered(s) => {
                    if term.field_ty == HirType::Bool && self.db_engine == "sqlite" {
                        format!("({s} ? 1 : 0)")
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
                binds.push(format!("`%${{{bind_expr}}}%`"));
            } else {
                binds.push(bind_expr);
            }
        }
        let sql = sqlph(
            &format!(" WHERE {}", conditions.join(" AND ")),
            self.db_engine,
        );
        (sql, binds)
    }

    /// Renders `value` as a JS value (not a JSON string) for
    /// `search.index`'s `document`/`http.call`'s body, mirroring
    /// Python's `json_body`/Rust's `json_value` 3-way (Record/Json/
    /// else) branch exactly — a real, pre-existing divergence from
    /// `cache.set`/`object_store.put`'s 2-way branch this refactor
    /// doesn't unify away. Unexercised this milestone (Email/Search/
    /// ExternalHttp stay `CIAC0011`-refused until M7 — see
    /// `TsBackend::supports`), kept correct for the trait's sake.
    fn json_body(&self, value: &str, value_ty: &HirType) -> String {
        match value_ty {
            HirType::Record(_) => format!("{{ ...{value} }}"),
            HirType::Json => value.to_owned(),
            _ => format!("{{ value: {value} }}"),
        }
    }
}

const SEARCH_INDEX_NAME: &str = "documents";

impl HostSyntax for TsSyntax<'_> {
    const ORIENTATION: Orientation = Orientation::Statement;

    fn int_lit(&self, n: i64) -> String {
        n.to_string()
    }
    fn float_lit(&self, f: f64) -> String {
        fidelity_checked_float(f)
    }
    fn str_lit(&self, s: &str) -> String {
        format!("{s:?}")
    }
    fn bool_lit(&self, b: bool) -> String {
        if b { "true" } else { "false" }.to_owned()
    }
    fn field_access(&self, base: &str, field: &str) -> String {
        format!("{base}.{field}")
    }
    /// `Json` indexing (Pillar 2): optional-chained access with a
    /// runtime presence check that throws a `KeyError`/`IndexError`-
    /// shaped error rather than silently propagating `undefined` —
    /// decided over silent propagation, which would diverge from
    /// Python's `KeyError` behavior. Unexercised this milestone (no
    /// open example indexes a `Json` field), kept correct for the
    /// trait's sake.
    fn index(&self, base: &str, key: IndexKey<'_>) -> String {
        match key {
            IndexKey::StrKey(s) => {
                let quoted = format!("{s:?}");
                format!(
                    "(() => {{ const __v = ({base})?.[{quoted}]; if (__v === undefined) throw new Error(\"KeyError: '{s}'\"); return __v; }})()"
                )
            }
            IndexKey::Expr(e) => format!(
                "(() => {{ const __k = {e}; const __v = ({base})?.[__k]; if (__v === undefined) throw new Error(`IndexError: ${{__k}}`); return __v; }})()"
            ),
        }
    }
    fn uuid_new(&self) -> String {
        "crypto.randomUUID()".to_owned()
    }
    fn timestamp_now(&self) -> String {
        "new Date()".to_owned()
    }
    fn enum_literal(&self, _enum_name: Option<&str>, variant: &str) -> String {
        format!("{variant:?}")
    }
    /// `satisfies {record_name}` (not a bare object literal, and not
    /// `as {record_name}` — `satisfies` validates the shape without
    /// widening the expression's inferred type away from its literal
    /// fields) rather than a plain object literal: TS's structural
    /// typing means an *unannotated* literal never actually spells the
    /// record's name anywhere, which left the type import genuinely
    /// unused — live-caught by `eslint` (`@typescript-eslint/
    /// no-unused-vars` on `OrderAudit` in `domain-orders.ciac`'s
    /// `PlaceOrder`) after `tsc` itself stayed clean (structural typing
    /// means the mismatch was never a *type* error, only a lint one).
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
            None => format!("({{ {} }} satisfies {record_name})", field_strs.join(", ")),
            Some(base) => format!(
                "({{ ...{base}, {} }} satisfies {record_name})",
                field_strs.join(", ")
            ),
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
            return format!("`${{{lhs}}}${{{rhs}}}`");
        }
        // Integer division: JS `/` is float division; `Int / Int`
        // lowers to `Math.trunc(a / b)` for i64-truncation parity with
        // Rust (Pillar 2's decision — Python's `/` stays true division,
        // a real, documented cross-target divergence the equivalence
        // suite asserts-as-documented, not one this backend silently
        // fixes for Python).
        if op == BinOp::Div && *lhs_ty == HirType::Int && *rhs_ty == HirType::Int {
            return format!("Math.trunc({lhs} / {rhs})");
        }
        let op_s = match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Eq => "===",
            BinOp::NotEq => "!==",
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

    fn if_tail(
        &self,
        cond: &str,
        then_lines: Vec<String>,
        else_lines: Vec<String>,
        indent: &str,
    ) -> Vec<String> {
        let mut out = vec![format!(
            "{indent}if ({}) {{",
            strip_outer_parens(cond.to_owned())
        )];
        out.extend(then_lines);
        if else_lines.is_empty() {
            out.push(format!("{indent}}}"));
        } else {
            out.push(format!("{indent}}} else {{"));
            out.extend(else_lines);
            out.push(format!("{indent}}}"));
        }
        out
    }
    /// A real `switch` statement (Pillar 2's decision, over Python's
    /// if/elif-chain transcription) — a `break` is only appended when
    /// the arm's own last line doesn't already diverge (`return`/
    /// `throw`), avoiding an `no-unreachable`-shaped dead `break`.
    fn match_tail(
        &self,
        scrutinee: &str,
        arms: &[(Option<String>, Vec<String>)],
        indent: &str,
    ) -> Vec<String> {
        let mut out = vec![format!("{indent}switch ({scrutinee}) {{")];
        for (variant, lines) in arms {
            match variant {
                Some(v) => out.push(format!("{indent}  case {v:?}:")),
                None => out.push(format!("{indent}  default:")),
            }
            let diverges = lines.last().is_some_and(|l| {
                let t = l.trim_start();
                t.starts_with("return") || t.starts_with("throw")
            });
            out.extend(lines.iter().cloned());
            if !diverges {
                out.push(format!("{indent}    break;"));
            }
        }
        out.push(format!("{indent}}}"));
        out
    }
    fn db_insert_tail(
        &self,
        table: TableId,
        value: &str,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let row = self.fresh("__row");
        let table_snake = self.ir.table(table).name.to_snake_case();
        let record = context::build_record(self.ir, self.ir.table(table).record);
        let handle = self.handle(in_tx);
        let binds: Vec<String> = record
            .fields
            .iter()
            .map(|f| self.bind_expr(f, &row))
            .collect();
        let sql = sqlph(
            &format!(
                "INSERT INTO {table_snake} ({}) VALUES ({})",
                record.select_cols, record.insert_placeholders
            ),
            self.db_engine,
        );
        let world_key = self.world_table_key(&table_snake);
        let mut out = vec![format!("{indent}const {row} = {value};")];
        // v0.23 M9's world guard: the one HostSyntax leaf `ciac sim`
        // needs to fake, mirroring Rust's `SimWorld::db_insert_checked`
        // exactly (including the fact the failure effect it checks is
        // named `"db.commit"`, not `"db.insert"` — the FailureEngine
        // rule vocabulary both backends share). Reached regardless of
        // `in_tx`: a standalone `db.insert` and one inside
        // `transaction { .. }` are both simulated the same way.
        out.push(format!("{indent}if (this.state.world) {{"));
        out.push(format!(
            "{indent}  this.state.world.dbInsertChecked({world_key:?}, {row});"
        ));
        out.push(format!("{indent}}} else {{"));
        if self.db_engine == "sqlite" {
            out.push(format!(
                "{indent}  {handle}.prepare({sql:?}).run({});",
                binds.join(", ")
            ));
        } else {
            out.push(format!(
                "{indent}  await {handle}.query({sql:?}, [{}]);",
                binds.join(", ")
            ));
        }
        out.push(format!("{indent}}}"));
        lower::apply_dest(self, dest, &row, indent, &mut out);
        out
    }
    fn db_update_tail(
        &self,
        table: TableId,
        key: &str,
        value: &str,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let row = self.fresh("__row");
        let result = self.fresh("__result");
        let out_var = self.fresh("__out");
        let table_snake = self.ir.table(table).name.to_snake_case();
        let record = context::build_record(self.ir, self.ir.table(table).record);
        let record_name = record_class_name(self.ir, self.ir.table(table).record);
        let handle = self.handle(in_tx);
        let mut binds: Vec<String> = record
            .fields
            .iter()
            .filter(|f| f.name != "id")
            .map(|f| self.bind_expr(f, &row))
            .collect();
        binds.push(key.to_owned());
        let sql = sqlph(
            &format!(
                "UPDATE {table_snake} SET {} WHERE id = {}",
                record.update_assignments, record.update_where
            ),
            self.db_engine,
        );
        // 27UpdatePlan.md M6: `{out_var}` is declared once, above the
        // world/production split, and only ever *assigned* (never
        // `const`-declared again) inside each branch -- TS's `if {}
        // else {}` introduces a real block scope (unlike Rust's/
        // Python's own branch-sharing orientations), so a `const`
        // declared inside one arm and read after the block closes is a
        // real `ReferenceError`/compile error, not a style nit; found
        // live via `tsc` against `domain-orders.ciac`.
        let world_key = self.world_table_key(&table_snake);
        let mut out = vec![
            format!("{indent}const {row} = {value};"),
            format!("{indent}let {out_var}: {record_name} | null;"),
        ];
        // World guard -- `dbUpdateChecked` does the same full-record
        // replace `SET` production SQL performs, `null` for a missing
        // row exactly like `rowCount === 0`.
        out.push(format!("{indent}if (this.state.world) {{"));
        out.push(format!(
            "{indent}  {out_var} = this.state.world.dbUpdateChecked({world_key:?}, {key}, {row}) as unknown as {record_name} | null;"
        ));
        out.push(format!("{indent}}} else {{"));
        match self.db_engine {
            "sqlite" => {
                out.push(format!(
                    "{indent}  const {result} = {handle}.prepare({sql:?}).run({});",
                    binds.join(", ")
                ));
                out.push(format!(
                    "{indent}  {out_var} = {result}.changes === 0 ? null : {row};"
                ));
            }
            "mysql" => {
                out.push(format!(
                    "{indent}  const [{result}] = await {handle}.query({sql:?}, [{}]);",
                    binds.join(", ")
                ));
                out.push(format!(
                    "{indent}  {out_var} = ({result} as {{ affectedRows: number }}).affectedRows === 0 ? null : {row};"
                ));
            }
            _ => {
                out.push(format!(
                    "{indent}  const {result} = await {handle}.query({sql:?}, [{}]);",
                    binds.join(", ")
                ));
                out.push(format!(
                    "{indent}  {out_var} = {result}.rowCount === 0 ? null : {row};"
                ));
            }
        }
        out.push(format!("{indent}}}"));
        lower::apply_dest(self, dest, &out_var, indent, &mut out);
        out
    }
    fn db_delete_tail(
        &self,
        table: TableId,
        key: &str,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let result = self.fresh("__result");
        let out_var = self.fresh("__out");
        let table_snake = self.ir.table(table).name.to_snake_case();
        let handle = self.handle(in_tx);
        let sql = sqlph(
            &format!("DELETE FROM {table_snake} WHERE id = $1"),
            self.db_engine,
        );
        // 27UpdatePlan.md M6: `{out_var}` is declared once, above the
        // world/production split, and only ever *assigned* inside each
        // branch -- see `db_update_tail`'s own doc for why (TS's `if {}
        // else {}` block scope; found live via `tsc`).
        let world_key = self.world_table_key(&table_snake);
        let mut out = vec![format!("{indent}let {out_var}: boolean;")];
        // World guard -- `dbDeleteChecked` resolves cascade/restrict
        // references the same way production's `ON DELETE` behavior
        // would (enforced at the schema level, not per-statement).
        out.push(format!("{indent}if (this.state.world) {{"));
        out.push(format!(
            "{indent}  {out_var} = this.state.world.dbDeleteChecked({world_key:?}, {key});"
        ));
        out.push(format!("{indent}}} else {{"));
        match self.db_engine {
            "sqlite" => {
                out.push(format!(
                    "{indent}  const {result} = {handle}.prepare({sql:?}).run({key});"
                ));
                out.push(format!("{indent}  {out_var} = {result}.changes > 0;"));
            }
            "mysql" => {
                out.push(format!(
                    "{indent}  const [{result}] = await {handle}.query({sql:?}, [{key}]);"
                ));
                out.push(format!(
                    "{indent}  {out_var} = ({result} as {{ affectedRows: number }}).affectedRows > 0;"
                ));
            }
            _ => {
                out.push(format!(
                    "{indent}  const {result} = await {handle}.query({sql:?}, [{key}]);"
                ));
                out.push(format!(
                    "{indent}  {out_var} = ({result}.rowCount ?? 0) > 0;"
                ));
            }
        }
        out.push(format!("{indent}}}"));
        lower::apply_dest(self, dest, &out_var, indent, &mut out);
        out
    }
    fn query_tail(
        &self,
        verb: Verb,
        predicate: Option<&LoweredPredicate>,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        match verb {
            Verb::DbQuery(table) => {
                let rows = self.fresh("__rows");
                let out_var = self.fresh("__out");
                let table_snake = self.ir.table(table).name.to_snake_case();
                let record = context::build_record(self.ir, self.ir.table(table).record);
                let handle = self.handle(in_tx);
                let (where_sql, binds) = self.where_clause(predicate);
                let sql = sqlph(
                    &format!(
                        "SELECT {} FROM {table_snake}{where_sql}",
                        record.select_cols
                    ),
                    self.db_engine,
                );
                let world_filter = self.world_filter_suffix(predicate, "__r");
                let cast_ty = self.row_cast_type(&record);
                let record_name = record_class_name(self.ir, self.ir.table(table).record);
                let world_key = self.world_table_key(&table_snake);
                // 27UpdatePlan.md M6: `{out_var}` is declared once,
                // above the world/production split, and only ever
                // *assigned* inside each branch -- see
                // `db_update_tail`'s own doc for why (TS's `if {} else
                // {}` block scope; found live via `tsc`).
                let mut out = vec![format!("{indent}let {out_var}: {record_name}[];")];
                out.push(format!("{indent}if (this.state.world) {{"));
                out.push(format!(
                    "{indent}  const {rows} = this.state.world.db.findWhere({world_key:?}, {{}}){world_filter};"
                ));
                // World rows are already in the record's own in-memory
                // shape (the same object `dbInsertChecked`'s own `row`
                // param received, never through `bindExpr`'s SQLite
                // storage-shape coercion) -- `mapRowExpr`'s SQLite/MySQL/
                // Postgres storage-decoding only applies to a *real* SQL
                // row, so the world branch casts through `unknown`
                // rather than remapping.
                out.push(format!(
                    "{indent}  {out_var} = {rows} as unknown as {record_name}[];"
                ));
                out.push(format!("{indent}}} else {{"));
                match self.db_engine {
                    "sqlite" => {
                        out.push(format!(
                            "{indent}  const {rows} = {handle}.prepare({sql:?}).all({}) as {cast_ty}[];",
                            binds.join(", ")
                        ));
                    }
                    "mysql" => out.push(format!(
                        "{indent}  const [{rows}] = await {handle}.query({sql:?}, [{}]);",
                        binds.join(", ")
                    )),
                    _ => out.push(format!(
                        "{indent}  const {rows} = (await {handle}.query({sql:?}, [{}])).rows;",
                        binds.join(", ")
                    )),
                }
                let map_expr = self.map_row_expr(&record, "__r");
                out.push(format!(
                    "{indent}  {out_var} = {rows}.map((__r) => ({map_expr}));"
                ));
                out.push(format!("{indent}}}"));
                lower::apply_dest(self, dest, &out_var, indent, &mut out);
                out
            }
            Verb::DbCount(table) => {
                let out_var = self.fresh("__out");
                let table_snake = self.ir.table(table).name.to_snake_case();
                let handle = self.handle(in_tx);
                let (where_sql, binds) = self.where_clause(predicate);
                let sql = sqlph(
                    &format!("SELECT COUNT(*) as count FROM {table_snake}{where_sql}"),
                    self.db_engine,
                );
                let world_filter = self.world_filter_suffix(predicate, "__r");
                let world_key = self.world_table_key(&table_snake);
                let mut out = vec![format!("{indent}let {out_var}: number;")];
                out.push(format!("{indent}if (this.state.world) {{"));
                out.push(format!(
                    "{indent}  {out_var} = this.state.world.db.findWhere({world_key:?}, {{}}){world_filter}.length;"
                ));
                out.push(format!("{indent}}} else {{"));
                match self.db_engine {
                    "sqlite" => {
                        let row = self.fresh("__row");
                        out.push(format!(
                            "{indent}  const {row} = {handle}.prepare({sql:?}).get({}) as {{ count: number }};",
                            binds.join(", ")
                        ));
                        out.push(format!("{indent}  {out_var} = {row}.count;"));
                    }
                    "mysql" => {
                        let rows = self.fresh("__rows");
                        out.push(format!(
                            "{indent}  const [{rows}] = await {handle}.query({sql:?}, [{}]);",
                            binds.join(", ")
                        ));
                        out.push(format!(
                            "{indent}  {out_var} = Number(({rows} as {{ count: number }}[])[0].count);"
                        ));
                    }
                    _ => {
                        let result = self.fresh("__result");
                        out.push(format!(
                            "{indent}  const {result} = await {handle}.query({sql:?}, [{}]);",
                            binds.join(", ")
                        ));
                        // Postgres returns COUNT(*) as a bigint, decoded
                        // as a string by `pg` to avoid precision loss —
                        // `Number(..)` matches this backend's `Int`
                        // decision (Pillar 2: `number`, 2^53 boundary
                        // disclosed) rather than adding a bigint mode.
                        out.push(format!(
                            "{indent}  {out_var} = Number({result}.rows[0].count);"
                        ));
                    }
                }
                out.push(format!("{indent}}}"));
                lower::apply_dest(self, dest, &out_var, indent, &mut out);
                out
            }
            Verb::DbDeleteWhere(table) => {
                let result = self.fresh("__result");
                let out_var = self.fresh("__out");
                let table_snake = self.ir.table(table).name.to_snake_case();
                let handle = self.handle(in_tx);
                let (where_sql, binds) = self.where_clause(predicate);
                let sql = sqlph(
                    &format!("DELETE FROM {table_snake}{where_sql}"),
                    self.db_engine,
                );
                let world_filter = self.world_filter_suffix(predicate, "__r");
                let world_key = self.world_table_key(&table_snake);
                let matching = self.fresh("__matching");
                let pk = self.fresh("__pk");
                // 27UpdatePlan.md M6: `SimWorld` has no bulk
                // delete-by-predicate method (`commitBatchChecked`
                // takes explicit `(table, pk)` pairs, not a filter), so
                // the world branch resolves matching ids first, then
                // deletes each through `dbDeleteChecked` (real cascade/
                // restrict handling, real failure-injection) -- a
                // disclosed divergence from production's single bulk
                // `DELETE`, mirroring Rust's own `query_expr` world
                // branch exactly.
                let world = self.fresh("__world");
                let mut out = vec![format!("{indent}let {out_var}: number;")];
                out.push(format!("{indent}if (this.state.world) {{"));
                // Captured into a local (rather than repeated
                // `this.state.world.*` accesses) so TypeScript's
                // control-flow narrowing of the guard above survives
                // across the loop below -- narrowing a dotted member
                // expression like `this.state.world` is not guaranteed
                // to persist across an intervening method call.
                out.push(format!("{indent}  const {world} = this.state.world;"));
                out.push(format!(
                    "{indent}  const {matching} = {world}.db.findWhere({world_key:?}, {{}}){world_filter}.map((__r) => __r.id as string);"
                ));
                out.push(format!("{indent}  {out_var} = 0;"));
                out.push(format!("{indent}  for (const {pk} of {matching}) {{"));
                out.push(format!(
                    "{indent}    if ({world}.dbDeleteChecked({world_key:?}, {pk})) {{"
                ));
                out.push(format!("{indent}      {out_var} += 1;"));
                out.push(format!("{indent}    }}"));
                out.push(format!("{indent}  }}"));
                out.push(format!("{indent}}} else {{"));
                match self.db_engine {
                    "sqlite" => {
                        out.push(format!(
                            "{indent}  const {result} = {handle}.prepare({sql:?}).run({});",
                            binds.join(", ")
                        ));
                        out.push(format!("{indent}  {out_var} = {result}.changes;"));
                    }
                    "mysql" => {
                        out.push(format!(
                            "{indent}  const [{result}] = await {handle}.query({sql:?}, [{}]);",
                            binds.join(", ")
                        ));
                        out.push(format!(
                            "{indent}  {out_var} = ({result} as {{ affectedRows: number }}).affectedRows;"
                        ));
                    }
                    _ => {
                        out.push(format!(
                            "{indent}  const {result} = await {handle}.query({sql:?}, [{}]);",
                            binds.join(", ")
                        ));
                        out.push(format!("{indent}  {out_var} = {result}.rowCount ?? 0;"));
                    }
                }
                out.push(format!("{indent}}}"));
                lower::apply_dest(self, dest, &out_var, indent, &mut out);
                out
            }
            _ => unreachable!("HirExpr::Query only ever carries a db query verb, found {verb:?}"),
        }
    }
    fn assign(&self, name: &str, value: &str, indent: &str) -> String {
        if self.branching_locals.contains(name) {
            format!("{indent}{name} = {value};")
        } else {
            format!("{indent}const {name} = {value};")
        }
    }
    fn discard_stmt(&self, value: &str, indent: &str) -> String {
        // `void (..)` rather than a bare `{value};`: a discarded
        // record-construction value renders as a leading `{ .. }`,
        // which JS parses as a block statement (not an object literal)
        // at statement-start position — `void (..)` forces expression
        // context unconditionally, so this is a real correctness fix,
        // not just an `@typescript-eslint/no-unused-expressions`
        // placation.
        format!("{indent}void ({value});")
    }
    fn empty_block_stmt(&self, _indent: &str) -> Vec<String> {
        // An empty `{}` block is valid JS on its own; Python needs a
        // synthesized `pass`, TS needs nothing.
        Vec::new()
    }
    /// REAL atomicity (Pillar 4), matching the Rust backend's own
    /// (`26UpdatePlan.md` M1): Postgres/MySQL check out a dedicated
    /// connection and run `BEGIN`/`COMMIT`/`ROLLBACK` by hand (a
    /// pool's `.query()` alone is not transactional — different calls
    /// can land on different pooled connections — live-verified
    /// against a real local Postgres server: a `throw` between two
    /// inserts left zero rows, a clean two-insert transaction left
    /// both). SQLite uses better-sqlite3's native `.transaction()`
    /// wrapper instead (live-verified identically): it's synchronous
    /// and already runs its callback against the single open
    /// connection, so no separate handle is needed there — see
    /// `handle`'s own doc.
    ///
    /// 27UpdatePlan.md M6: under simulation (`this.state.world` set),
    /// the real checkout/`BEGIN`/`COMMIT`/`ROLLBACK`/release calls are
    /// skipped, and `this.state.world`'s ambient batch mode
    /// (`beginWorldBatch`/`commitWorldBatch`/`rollbackWorldBatch` --
    /// see `world.ts`'s own doc) wraps `inner_lines` instead: every
    /// `db.insert`/`update`/`delete` inside `inner_lines` already
    /// calls `dbXChecked` (unchanged from outside a transaction), and
    /// while a batch is open those calls queue instead of applying, so
    /// the whole block commits atomically at `commitWorldBatch()`,
    /// retiring the "degraded per-verb shape" a prior milestone's own
    /// disclosure named -- matching Rust's own M4 atomicity fix.
    /// Production TypeScript's real atomicity (SQLite's `.transaction()`
    /// wrapper; Postgres/MySQL's dedicated-connection `BEGIN`/`COMMIT`/
    /// `ROLLBACK`) is unchanged.
    fn transaction_stmt(&self, inner_lines: Vec<String>, indent: &str) -> Vec<String> {
        let field = self
            .db_field
            .as_deref()
            .expect("a transaction block requires a bound database instance");
        match self.db_engine {
            "sqlite" => {
                let mut out = vec![
                    format!("{indent}this.state.world?.beginWorldBatch();"),
                    format!("{indent}try {{"),
                    format!("{indent}  this.state.{field}.$client.transaction(() => {{"),
                ];
                out.extend(inner_lines);
                out.push(format!("{indent}  }})();"));
                out.push(format!("{indent}  this.state.world?.commitWorldBatch();"));
                out.push(format!("{indent}}} catch (__e) {{"));
                out.push(format!("{indent}  this.state.world?.rollbackWorldBatch();"));
                out.push(format!("{indent}  throw __e;"));
                out.push(format!("{indent}}}"));
                out
            }
            _ => {
                let mut out = vec![
                    // `$client.connect` is an *overloaded* signature
                    // (a zero-arg promise form and a callback form) --
                    // `ReturnType<typeof this.state.{field}.$client.connect>`
                    // has no call-site argument count to resolve
                    // against, so `tsc` picks the callback overload's
                    // `void` return, not the promise one this code
                    // actually calls. Routing through a zero-arg
                    // wrapper closure gives `tsc` a real call
                    // expression to resolve overloads against, so
                    // `Awaited<ReturnType<typeof __connect>>` lands on
                    // the correct `PoolClient`/`PoolConnection` type.
                    format!(
                        "{indent}const __connect = () => this.state.{field}.$client.connect();"
                    ),
                    format!("{indent}let __tx: Awaited<ReturnType<typeof __connect>> | undefined;"),
                    format!("{indent}if (!this.state.world) {{"),
                    format!("{indent}    __tx = await __connect();"),
                    format!("{indent}    await __tx.query(\"BEGIN\");"),
                    format!("{indent}}}"),
                    format!("{indent}this.state.world?.beginWorldBatch();"),
                    format!("{indent}try {{"),
                ];
                out.extend(inner_lines);
                out.push(format!("{indent}    if (!this.state.world) {{"));
                out.push(format!("{indent}        await __tx!.query(\"COMMIT\");"));
                out.push(format!("{indent}    }}"));
                out.push(format!("{indent}    this.state.world?.commitWorldBatch();"));
                out.push(format!("{indent}}} catch (__e) {{"));
                out.push(format!("{indent}    if (!this.state.world) {{"));
                out.push(format!("{indent}        await __tx!.query(\"ROLLBACK\");"));
                out.push(format!("{indent}    }}"));
                out.push(format!(
                    "{indent}    this.state.world?.rollbackWorldBatch();"
                ));
                out.push(format!("{indent}    throw __e;"));
                out.push(format!("{indent}}} finally {{"));
                out.push(format!("{indent}    if (!this.state.world) {{"));
                out.push(format!("{indent}        __tx!.release();"));
                out.push(format!("{indent}    }}"));
                out.push(format!("{indent}}}"));
                out
            }
        }
    }

    fn return_stmt(&self, value: Option<&str>, indent: &str) -> String {
        match value {
            Some(v) => format!("{indent}return {v};"),
            None => format!("{indent}return;"),
        }
    }
    fn fail(&self, error: RecordId, args: &[String], indent: &str) -> String {
        let exc = record_class_name(self.ir, error);
        format!("{indent}throw new {exc}({});", args.join(", "))
    }
    fn publish(&self, subject: &str, value: &str, _value_ty: &HirType, indent: &str) -> String {
        // The shared `publish(state, subject, payload)` free function
        // (`queue.ts.j2`) every generated call site goes through — not
        // an `AppState` method (there isn't one) — taking a `Buffer`,
        // not a raw string; live-caught by `tsc` on `order-system.ciac`'s
        // `std/webhook.ciac`-expanded `RecordEvent` handler, the first
        // typed-handler body (as opposed to a pipeline-level `publish`
        // step, already wired correctly in `_steps.ts.j2`) this arc
        // exercised with a `publish` statement.
        format!(
            "{indent}await publish(this.state, {subject:?}, Buffer.from(JSON.stringify({value})));"
        )
    }
    fn db_get(&self, table: TableId, key: &str) -> String {
        let table_snake = self.ir.table(table).name.to_snake_case();
        let record = context::build_record(self.ir, self.ir.table(table).record);
        let record_name = record_class_name(self.ir, self.ir.table(table).record);
        let handle = format!(
            "this.state.{}.$client",
            self.db_field
                .as_deref()
                .expect("db.get requires a bound database instance")
        );
        let sql = sqlph(
            &format!(
                "SELECT {} FROM {table_snake} WHERE id = $1",
                record.select_cols
            ),
            self.db_engine,
        );
        let map_expr = self.map_row_expr(&record, "__row");
        let world_key = self.world_table_key(&table_snake);
        // 27UpdatePlan.md M6: the world guard is prepended as an early
        // return inside the same IIFE -- the original per-engine body
        // below is otherwise untouched.
        let world_guard = format!(
            "if (this.state.world) {{ return this.state.world.db.get({world_key:?}, {key}) as unknown as {record_name} | null; }}"
        );
        match self.db_engine {
            "sqlite" => {
                let cast_ty = self.row_cast_type(&record);
                format!(
                    "(() => {{ {world_guard} const __row = {handle}.prepare({sql:?}).get({key}) as {cast_ty} | undefined; return __row === undefined ? null : {map_expr}; }})()"
                )
            }
            "mysql" => format!(
                "await (async () => {{ {world_guard} const [__rows] = await {handle}.query({sql:?}, [{key}]); const __row = __rows[0]; return __row === undefined ? null : {map_expr}; }})()"
            ),
            _ => format!(
                "await (async () => {{ {world_guard} const __row = (await {handle}.query({sql:?}, [{key}])).rows[0]; return __row === undefined ? null : {map_expr}; }})()"
            ),
        }
    }
    fn cache_get(&self, key: &str) -> String {
        let cache = self.cache_handle();
        let instance = self.cache_instance();
        format!(
            "await (async () => {{ if (this.state.world) {{ const __cv = this.state.world.cacheGet({instance:?}, {key}); return __cv === null ? null : JSON.parse(__cv); }} const __cv = await {cache}.get({key}); return __cv === null ? null : JSON.parse(__cv); }})()"
        )
    }
    fn cache_set(&self, key: &str, value: &str, _value_ty: &HirType) -> String {
        let cache = self.cache_handle();
        let instance = self.cache_instance();
        format!(
            "(await (this.state.world ? this.state.world.cacheSet({instance:?}, {key}, JSON.stringify({value}), null) : {cache}.set({key}, JSON.stringify({value}))))"
        )
    }
    fn cache_delete(&self, key: &str) -> String {
        let cache = self.cache_handle();
        let instance = self.cache_instance();
        format!(
            "(await (this.state.world ? this.state.world.cacheDelete({instance:?}, {key}) : {cache}.del({key})))"
        )
    }
    // --- object store / email / search / http (v0.23 M7, world-guarded
    // 27UpdatePlan.md M6): each leaf reaches the handler's bound
    // instance through `this.state.<state_field>` in production and
    // `this.state.world.<method>({instance_name}, ..)` under
    // simulation, resolved once in `render()` exactly like
    // `db_field`/`cache_field` — see `object_store_field`'s own doc.
    fn object_store_put(&self, key: &str, value: &str, value_ty: &HirType) -> String {
        let field = self.object_store_state_field();
        let instance = self.object_store_instance();
        let payload = if matches!(value_ty, HirType::Record(_)) {
            format!("JSON.stringify({value})")
        } else {
            format!("String({value})")
        };
        format!(
            "(await (this.state.world ? this.state.world.objectPut({instance:?}, {key}, Buffer.from({payload})) : this.state.{field}.put({key}, {payload})))"
        )
    }
    fn object_store_get(&self, key: &str) -> String {
        let field = self.object_store_state_field();
        let instance = self.object_store_instance();
        format!(
            "JSON.parse(await (this.state.world ? Promise.resolve(this.state.world.objectGet({instance:?}, {key}).toString(\"utf8\")) : this.state.{field}.get({key})))"
        )
    }
    fn object_store_delete(&self, key: &str) -> String {
        let field = self.object_store_state_field();
        let instance = self.object_store_instance();
        format!(
            "(await (this.state.world ? this.state.world.objectDelete({instance:?}, {key}) : this.state.{field}.delete({key})))"
        )
    }
    fn object_store_list(&self, prefix: &str) -> String {
        let field = self.object_store_state_field();
        let instance = self.object_store_instance();
        format!(
            "(await (this.state.world ? Promise.resolve(this.state.world.objectList({instance:?}, {prefix})) : this.state.{field}.list({prefix})))"
        )
    }
    fn email_send(&self, to: &str, subject: &str, body: &str) -> String {
        let field = self
            .email_field
            .as_deref()
            .expect("an email verb requires a bound email instance");
        let instance = self.email_instance();
        format!(
            "(await (this.state.world ? this.state.world.emailSend({instance:?}, {to}, {subject}, {body}) : this.state.{field}.send({to}, {subject}, {body})))"
        )
    }
    fn search_index(&self, doc_id: &str, document: &str, document_ty: &HirType) -> String {
        let field = self
            .search_field
            .as_deref()
            .expect("a search verb requires a bound search instance");
        let instance = self.search_instance();
        let document = self.json_body(document, document_ty);
        format!(
            "(await (this.state.world ? this.state.world.searchIndex({instance:?}, {doc_id}, {document}) : this.state.{field}.index({SEARCH_INDEX_NAME:?}, {doc_id}, {document})))"
        )
    }
    fn search_query(&self, query: &str) -> String {
        let field = self
            .search_field
            .as_deref()
            .expect("a search verb requires a bound search instance");
        let instance = self.search_instance();
        format!(
            "(await (this.state.world ? Promise.resolve(this.state.world.searchQuery({instance:?}, {query})) : this.state.{field}.search({SEARCH_INDEX_NAME:?}, {{ query: {{ query_string: {{ query: {query} }} }} }})))"
        )
    }
    fn http_call(&self, url: &str, json_body: &str, body_ty: &HirType) -> String {
        let field = self
            .http_field
            .as_deref()
            .expect("an external_http verb requires a bound external_http instance");
        let instance = self.http_instance();
        let json_arg = self.json_body(json_body, body_ty);
        format!(
            "(await (this.state.world ? Promise.resolve(this.state.world.httpPost({instance:?}, {url}, {json_arg})) : this.state.{field}.post({url}, {json_arg})))"
        )
    }
}

#[derive(Debug, Serialize)]
pub struct ParamCtx {
    pub name: String,
    pub ts_type: String,
}

/// One `schemas.ts` import `logic.ts.j2` needs. `is_error` records
/// (`error X { .. }`) are constructed at runtime (`throw new X(..)` —
/// a real value use, not just a type position), so they need a plain
/// `import { X }`; every other record is only ever used in a type
/// position here (a handler's own param/return type, or another
/// record's field type) and gets `import type { X }` — a bare `import
/// type` would be erased entirely at compile time and leave `new X(..)`
/// referencing nothing, a real bug live-caught generating
/// `domain-orders.ciac`'s `PlaceOrder` (`throw new InvalidOrder(..)`).
#[derive(Debug, Serialize)]
pub struct SchemaImportCtx {
    pub name: String,
    pub is_error: bool,
}

/// Everything `logic.ts.j2` needs to render one typed handler's file —
/// inline (compiler-owned, `src/logic/<module>.ts`) or `extern` (seeded,
/// `src/services/<module>.ts`, `body` is just a stand-in `throw`).
#[derive(Debug, Serialize)]
pub struct LogicFileCtx {
    pub class_name: String,
    pub module: String,
    pub is_extern: bool,
    pub params: Vec<ParamCtx>,
    pub return_type: String,
    pub schema_imports: Vec<SchemaImportCtx>,
    /// Whether this handler's body has a `publish` statement — gates
    /// the `import { publish } from "../queue.js"` line.
    pub needs_queue: bool,
    /// Every HIR `Let`'s local name, hoisted into one `let` declaration
    /// at the top of `handle()` — see the module doc for why.
    pub locals: Vec<String>,
    pub body: Vec<String>,
}

/// Builds the render context for one typed handler node. `name` is the
/// handler's declared name (`node.component.name()`).
pub fn render(
    ir: &NormalizedIr,
    name: &str,
    hir: &HandlerBody,
    service_name: Option<&str>,
) -> LogicFileCtx {
    let needs = lower::scan(ir, hir);
    let bindings = context::hir_bindings(ir, hir);
    let access = context::access_of(&bindings);
    let extras = context::extras_of(&bindings);
    let extra_field = |kind: &str| {
        extras
            .iter()
            .find(|e| e.kind == kind)
            .map(|e| e.rust_state_field.clone())
    };
    let object_store_field = extra_field("object_store");
    let email_field = extra_field("email");
    let search_field = extra_field("search");
    let http_field = extra_field("external_http");
    let instance_of = |kind: &str| {
        bindings
            .iter()
            .find(|b| b.kind == kind)
            .map(|b| b.name.clone())
    };
    let cache_instance = instance_of("cache");
    let object_store_instance = instance_of("object_store");
    let email_instance = instance_of("email");
    let search_instance = instance_of("search");
    let http_instance = instance_of("external_http");

    let mut schema_imports: Vec<SchemaImportCtx> = needs
        .records
        .iter()
        .map(|id| SchemaImportCtx {
            name: record_class_name(ir, *id),
            is_error: ir.record(*id).kind == ciac_ir::RecordKind::Error,
        })
        .collect();
    schema_imports.sort_by(|a, b| a.name.cmp(&b.name));

    let params = hir
        .params
        .iter()
        .map(|(n, ty)| ParamCtx {
            name: n.clone(),
            ts_type: ts_type(ir, ty),
        })
        .collect();

    let db_engine = db_engine_of(ir, hir);
    let mut locals = Vec::new();
    let body = match &hir.body {
        Some(stmts) => {
            collect_branching_lets(hir, stmts, &mut locals);
            let syntax = TsSyntax {
                ir,
                db_engine,
                db_field: access.rust_db_field.clone(),
                cache_field: access.rust_cache_field.clone(),
                object_store_field: object_store_field.clone(),
                email_field: email_field.clone(),
                search_field: search_field.clone(),
                http_field: http_field.clone(),
                cache_instance: cache_instance.clone(),
                object_store_instance: object_store_instance.clone(),
                email_instance: email_instance.clone(),
                search_instance: search_instance.clone(),
                http_instance: http_instance.clone(),
                tmp: std::cell::Cell::new(0),
                branching_locals: locals.iter().cloned().collect(),
                service_name: service_name.map(str::to_owned),
            };
            lower::lower_body_stmt(&syntax, ir, hir, "    ")
        }
        None => vec!["    throw new Error(\"not implemented\");".to_owned()],
    };

    LogicFileCtx {
        class_name: name.to_owned(),
        module: name.to_snake_case(),
        is_extern: hir.body.is_none(),
        params,
        return_type: ts_type(ir, &hir.return_ty),
        schema_imports,
        needs_queue: needs.queue,
        locals,
        body,
    }
}

/// Engine of the handler's bound db instance, straight from the IR
/// node its binding edge points at — `postgres` when unbound, matching
/// the Rust backend's own `db_engine_of` exactly (v0.13 M1's
/// discipline, reused here rather than re-derived differently).
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
