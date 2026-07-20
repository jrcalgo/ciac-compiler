//! Direct lowering of the typed HIR (`ciac_ir::hir`) into Java source
//! (`25UpdatePlan.md` M4).
//!
//! The walker (block/tail shaping, precedence, enum-literal use-site
//! recovery, float-literal fidelity, divergence truncation) lives in
//! `ciac_codegen::lower`; [`JavaSyntax`] supplies only the leaf
//! constructors genuinely specific to this target. Java runs in
//! `Orientation::Statement` — the third consumer of the mode Python
//! validated and Go re-validated — since a `{}` block is not an
//! expression in Java either.
//!
//! **No error-idiom amendment override needed:** unchecked exceptions
//! throughout (every generated error record already extends
//! `RuntimeException` — see `record.java.j2`'s M1/M2 own work) mean a
//! fallible verb's result is usable the moment it's computed, with no
//! `if err != nil`-style decomposition — the same shape Python already
//! has. Every "simple verb" leaf below (`db_get`/`cache_*`/
//! `object_store_*`/`email_send`/`search_*`/`http_call`) is therefore a
//! plain scalar expression, letting `HostSyntax`'s own default
//! `..._tail` wrappers apply `Dest` unchanged — zero of Go's
//! `fallible_tail`/two-return-value machinery needed.
//!
//! **`TransactionTemplate`, not `@Transactional`** (Pillar 4): a
//! `transaction {}` block wraps its (sema-guaranteed `return`/
//! `publish`-free) inner statements in
//! `txTemplate.executeWithoutResult(status -> { .. })` — a `fail`'s
//! `throw` propagating out of that lambda both rolls back (Spring's
//! own `TransactionCallback` contract: any `RuntimeException` from the
//! callback triggers rollback, then re-throws) and reaches the caller
//! unchanged, so no `in_tx`-conditional handle-switching is needed at
//! all: every db verb leaf below ignores its own `in_tx` parameter and
//! always issues SQL through the same `JdbcClient` bean, which
//! transparently participates in the ambient transaction via Spring's
//! `DataSourceUtils` connection binding. A real, disclosed
//! simplification against Go's explicit dual-handle (`*sql.Tx` vs.
//! pool) scheme — there is no separate "transaction handle" type in
//! this target's own JDBC story.
//!
//! **What stays out of scope this milestone, matching Go's/TS's own M4
//! precedent exactly:** every leaf below is implemented —
//! `object_store.*`/`email.*`/`search.*`/`http.*` included — so the
//! trait compiles completely with no `unimplemented!()` leaf
//! reachable, but the *component* kinds that request those
//! capabilities stay refused in `JavaBackend::supports` until M6/M7
//! add their client wrappers and gate them for real.
//! `typed-handlers.ciac`/`extras-verbs.ciac`/`typed-video.ciac` (need
//! `object_store`/`cache`/`auth`) stay `CIAC0011`-refused this
//! milestone; `domain-orders.ciac`/`query-verbs.ciac` (db-only) are
//! this milestone's actual proving examples.

pub(crate) use ciac_codegen::lower::scan;
use ciac_codegen::lower::{
    self, apply_dest, fidelity_checked_float, strip_outer_parens, Dest, HostSyntax, IndexKey,
    LoweredPredicate, Orientation, PredValue,
};
use ciac_codegen::model::{self as context, FieldCtx, RecordCtx};
use ciac_ir::{
    BinOp, Component, DbEngine, HandlerBody, HirExpr, HirStmt, HirType, NormalizedIr, PredOp,
    RecordId, TableId, UnOp, Verb,
};
use heck::{ToLowerCamelCase, ToSnakeCase};
use serde::Serialize;
use std::cell::Cell;
use std::collections::HashMap;

use crate::filters::jdbcph;

/// A Java `camelCase` identifier for a `snake_case` field name — the
/// plain-`&str` twin of `filters::java_camel` (that one exists to be a
/// minijinja filter over a template `field`/`String` value; this one
/// exists because `lower.rs`'s own leaf bodies build identifiers
/// outside any template context).
fn java_camel(s: &str) -> String {
    s.to_lower_camel_case()
}

/// Java type annotation for a HIR type — a handler *signature* concern
/// (param/return types, and the hoisted-`var` declarations
/// [`collect_branching_lets`] needs a real type for), not part of the
/// `HostSyntax` body contract — mirrors Go's `go_type`/Python's
/// `py_type`. `Option<T>`/`List<T>` box a primitive `T` (`Long`/
/// `Double`/`Boolean`), since neither `null` nor a generic type
/// parameter can hold a Java primitive — see [`java_boxed`].
pub fn java_type(ir: &NormalizedIr, ty: &HirType) -> String {
    match ty {
        HirType::Str | HirType::Uuid => "String".to_owned(),
        HirType::Int => "long".to_owned(),
        HirType::Float => "double".to_owned(),
        HirType::Bool => "boolean".to_owned(),
        HirType::Timestamp => "java.time.Instant".to_owned(),
        HirType::Json => "com.fasterxml.jackson.databind.JsonNode".to_owned(),
        HirType::Enum { .. } => {
            unreachable!("a bare enum type never appears in a param/return/local position")
        }
        HirType::Record(id) => ir.record(*id).name.clone(),
        HirType::Option(inner) => java_boxed(ir, inner),
        HirType::List(inner) => format!("java.util.List<{}>", java_boxed(ir, inner)),
        HirType::Unit | HirType::Never => "void".to_owned(),
    }
}

/// The reference-type spelling of a HIR type for a generic type
/// argument or a nullable position — boxes the three primitives
/// [`java_type`] would otherwise return bare (`long`/`double`/
/// `boolean`), since neither `java.util.List<T>`'s type argument nor a
/// `null`-accepting `Option<T>` slot can hold a Java primitive.
fn java_boxed(ir: &NormalizedIr, ty: &HirType) -> String {
    match ty {
        HirType::Int => "Long".to_owned(),
        HirType::Float => "Double".to_owned(),
        HirType::Bool => "Boolean".to_owned(),
        other => java_type(ir, other),
    }
}

fn table_sql_name(ir: &NormalizedIr, table: TableId) -> String {
    ir.table(table).name.to_snake_case()
}

/// The engine backing a `table` declaration — falls back to Postgres
/// for a program with no explicit `db_instance` recorded (the
/// single-service, single-`db`-instance shape every M4 example so far
/// still resolves through the same default the shared `RecordCtx`
/// builder itself falls back to). Needed here (rather than reusing a
/// shared per-service default) because a Postgres `jsonb` column
/// rejects a bound `String` outright ("column .. is of type jsonb but
/// expression is of type character varying") unless the placeholder
/// itself carries an explicit `::jsonb` cast — MySQL's/SQLite's own
/// JSON-as-text columns need no such cast, so the cast must be
/// engine-conditional, not unconditional.
fn table_db_engine(ir: &NormalizedIr, table: TableId) -> DbEngine {
    ir.table(table)
        .db_instance
        .and_then(|nid| match &ir.node(nid).component {
            Component::Database { engine, .. } => Some(*engine),
            _ => None,
        })
        .unwrap_or(DbEngine::Postgres)
}

/// A field's SQL placeholder text for a write (`INSERT`/`UPDATE`)
/// bind — plain `?` except a `Json` field on Postgres, which needs an
/// explicit `?::jsonb` cast (see [`table_db_engine`]'s own doc).
fn field_placeholder(f: &FieldCtx, engine: DbEngine) -> &'static str {
    if f.is_json && engine == DbEngine::Postgres {
        "?::jsonb"
    } else {
        "?"
    }
}

/// Collects every `(name, HirType)` a `HirStmt::Let` binds *whose value
/// is an `if`/`match` expression* — duplicated near-verbatim from
/// `ciac-backend-go`'s own `collect_branching_lets` (that crate's own
/// doc explains why: Go's `if {} else {}`/`switch {}` introduce a real
/// block scope a `:=`-declared name inside one branch can't escape;
/// Java's `if {}`/`switch {}` have the identical block-scoping rule for
/// a `var`-declared local, so the same hoist-above-the-branch fix
/// applies unchanged). Recurses into `if`/`match` branch bodies and
/// `transaction {}`'s inner block — every site
/// [`ciac_codegen::lower::lower_block_stmt`] itself recurses into.
fn collect_branching_lets(body: &HandlerBody, stmts: &[HirStmt], out: &mut Vec<(String, HirType)>) {
    for stmt in stmts {
        match stmt {
            HirStmt::Let { slot, value } => {
                if matches!(value, HirExpr::If { .. } | HirExpr::Match { .. }) {
                    out.push((local_name(body, *slot), value.ty()));
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

fn collect_branching_lets_expr(
    body: &HandlerBody,
    expr: &HirExpr,
    out: &mut Vec<(String, HirType)>,
) {
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

/// `local_name`, duplicated from `ciac_codegen::lower::dispatch` (a
/// private helper there, same duplication every backend's own
/// `lower.rs` already carries): a [`HirExpr::Local`]'s declared name
/// when it names a parameter, else a synthesized `v<slot>` for a `let`
/// local.
fn local_name(body: &HandlerBody, slot: u32) -> String {
    let slot = slot as usize;
    if slot < body.params.len() {
        body.params[slot].0.clone()
    } else {
        format!("v{slot}")
    }
}

/// The `Orientation::Statement` `HostSyntax` implementation for this
/// target. Holds the bound `db`/`cache`/... `AppState` field names
/// (resolved once per handler) and a name->id lookup for
/// [`HostSyntax::record_cons`]'s functional-update case, which needs a
/// record's *full* declared field list — something the shared
/// dispatcher's own `record_name: &str` alone can't give it back.
struct JavaSyntax<'a> {
    ir: &'a NormalizedIr,
    tx_field: Option<String>,
    cache_field: Option<String>,
    object_store_field: Option<String>,
    email_field: Option<String>,
    search_field: Option<String>,
    http_field: Option<String>,
    record_by_name: HashMap<String, RecordId>,
    tmp: Cell<u32>,
    branching_locals: HashMap<String, HirType>,
}

impl JavaSyntax<'_> {
    fn fresh(&self, base: &str) -> String {
        let n = self.tmp.get();
        self.tmp.set(n + 1);
        format!("{base}{n}")
    }

    fn jdbc(&self) -> &str {
        "jdbc"
    }

    /// A field's write-side bind expression: the Java record accessor
    /// call, e.g. `__row.title()`. A `Json` field's accessor returns
    /// `JsonNode`, which JDBC's `JdbcClient` can't bind directly --
    /// `JsonNode` (`ObjectNode`/`ArrayNode`) implements `Iterable`, so
    /// `.param(..)` mistakes it for a positional-expansion candidate
    /// ("Parameter expansion is only supported with named parameters")
    /// rather than a single scalar value. Serializing through
    /// `Schemas.toJson` first binds a plain `String`, matching the
    /// column's own `TEXT`/`JSONB`-as-text storage.
    fn bind_expr(&self, record: &RecordCtx, base: &str) -> Vec<String> {
        record
            .fields
            .iter()
            .map(|f| Self::field_bind(f, base))
            .collect()
    }

    fn field_bind(f: &FieldCtx, base: &str) -> String {
        let accessor = format!("{base}.{}()", java_camel(&f.name));
        if f.is_json {
            format!("Schemas.toJson({accessor})")
        } else {
            accessor
        }
    }

    /// Builds a ` WHERE ..` clause (empty string with no predicate) and
    /// the ordered bind expressions it needs — JDBC's `?` is purely
    /// positional (Pillar 5: "the simplest placeholder story of any
    /// backend"), so, unlike Go's own `$N`-numbered scheme, term order
    /// alone is enough; no per-term index bookkeeping is needed at all.
    fn where_clause(&self, predicate: Option<&LoweredPredicate>) -> (String, Vec<String>) {
        let Some(predicate) = predicate else {
            return (String::new(), Vec::new());
        };
        let mut conditions = Vec::with_capacity(predicate.terms.len());
        let mut binds = Vec::with_capacity(predicate.terms.len());
        for term in &predicate.terms {
            let bind_expr = match &term.value {
                PredValue::EnumVariant(v) => format!("{v:?}"),
                PredValue::BoolLit(b) => b.to_string(),
                PredValue::Rendered(s) => s.clone(),
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
            conditions.push(format!("{field} {op} ?"));
            if term.op == PredOp::Contains {
                binds.push(format!("(\"%\" + {bind_expr} + \"%\")"));
            } else {
                binds.push(bind_expr);
            }
        }
        (format!(" WHERE {}", conditions.join(" AND ")), binds)
    }

    fn params_chain(binds: &[String]) -> String {
        binds
            .iter()
            .map(|b| format!(".param({b})"))
            .collect::<Vec<_>>()
            .join("")
    }
}

impl HostSyntax for JavaSyntax<'_> {
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
    /// Java records expose fields through accessor *methods*, not bare
    /// public fields (unlike Go's `.Field`/Python's `.field`) — the one
    /// structural divergence every leaf that spells a field access has
    /// to account for.
    fn field_access(&self, base: &str, field: &str) -> String {
        format!("{base}.{}()", java_camel(field))
    }
    /// `Json` indexing (Pillar 2): `JsonNode.path(..)` is null-safe by
    /// design (never throws, returns a `MissingNode` on a miss), so the
    /// KeyError/IndexError parity every other target's decoder enforces
    /// needs an explicit check — `Schemas.indexOrThrow` (added this
    /// milestone) does exactly that, uniformly for both a literal
    /// string key and a dynamic one: `JsonNode.path(String)` accepts
    /// any runtime `String` value, not just a compile-time literal, so
    /// (unlike Rust's `serde_json::Value` index operator, which needs
    /// the raw `&str` text for a literal key) there is no separate
    /// code shape for the two `IndexKey` variants here, only a
    /// different Java expression for the key argument itself.
    fn index(&self, base: &str, key: IndexKey<'_>) -> String {
        let key_s = match key {
            IndexKey::StrKey(s) => format!("{s:?}"),
            IndexKey::Expr(e) => e,
        };
        format!("Schemas.indexOrThrow({base}, {key_s})")
    }
    fn uuid_new(&self) -> String {
        "java.util.UUID.randomUUID().toString()".to_owned()
    }
    fn timestamp_now(&self) -> String {
        "java.time.Instant.now()".to_owned()
    }
    /// A real Java enum constant, `{EnumName}.{Variant}` — `Some`-only,
    /// mirroring Go's/Rust's own choice: Java enums are a real named
    /// type too (`RecordEnum.java.j2`, v0.25 M3), so a bare variant with
    /// no enclosing context to name it panics rather than guessing.
    fn enum_literal(&self, enum_name: Option<&str>, variant: &str) -> String {
        let name = enum_name.expect("Java enum literals need a named enclosing type");
        format!("{name}.{variant}")
    }
    /// Java records have no struct-spread/update syntax and no
    /// in-place field mutation (unlike Go's `v := base; v.Field =
    /// value`) — every construction, fresh or functional-update alike,
    /// is a full positional constructor call in the record's own
    /// declared field order. Sema already guarantees a *fresh*
    /// construction's `fields` lists every declared field (
    /// `check_field_inits`'s `require_all: true`), so both cases
    /// resolve identically here: for each declared field, prefer the
    /// caller-supplied value if the (surface-named) field appears in
    /// `fields`, else fall back to `base`'s own accessor — this also
    /// transparently gets a `Reference<T>` field's storage rename right
    /// (`customer` on the wire/handler side, `customerId` as the
    /// generated accessor — see `build_record`'s own doc), since the
    /// fallback path reads the *Java-facing* field name computed there,
    /// not the raw HIR field name used to key `fields`.
    fn record_cons(
        &self,
        record_name: &str,
        fields: &[(String, String)],
        base: Option<&str>,
    ) -> String {
        let rid = *self
            .record_by_name
            .get(record_name)
            .unwrap_or_else(|| panic!("record_cons: unknown record `{record_name}`"));
        let raw_record = self.ir.record(rid);
        let java_record = context::build_record(self.ir, rid);
        let field_map: HashMap<&str, &str> = fields
            .iter()
            .map(|(n, v)| (n.as_str(), v.as_str()))
            .collect();
        let args: Vec<String> = raw_record
            .fields
            .iter()
            .zip(java_record.fields.iter())
            .map(|(raw, java_field)| {
                if let Some(v) = field_map.get(raw.name.as_str()) {
                    (*v).to_owned()
                } else {
                    let base = base
                        .expect("record_cons: field omitted without a base value to fall back to");
                    format!("{base}.{}()", java_camel(&java_field.name))
                }
            })
            .collect();
        format!("new {record_name}({})", args.join(", "))
    }
    /// Java's `+` auto-stringifies a non-`String` operand the moment
    /// either side is a `String` (`Object.toString()`, called
    /// implicitly) — simpler than Go's `fmt.Sprintf`/Python's `str()`
    /// wrapping, needing no explicit conversion call at all.
    fn binary(
        &self,
        op: BinOp,
        lhs: &str,
        rhs: &str,
        lhs_ty: &HirType,
        rhs_ty: &HirType,
    ) -> String {
        if op == BinOp::Add && (*lhs_ty == HirType::Str || *rhs_ty == HirType::Str) {
            return format!("({lhs} + {rhs})");
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
    /// Java's `/` on `long` already truncates toward zero (JLS
    /// 15.17.2), matching Rust `i64` division exactly — no
    /// `Math.trunc`-style special case needed, the same free
    /// simplification Go's own native `int64` division gets.
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
    /// A real `switch` statement over the enum value itself, one
    /// arrow-case per variant (Java 21) — case labels are bare,
    /// unqualified enum constant names (Java's own switch-on-enum
    /// special case), not quoted strings, so this needs no
    /// string-conversion of the scrutinee at all, simpler than Go's own
    /// string-keyed `switch`. No `break` needed (arrow-case bodies
    /// don't fall through), mirroring Go's own "no explicit break"
    /// simplicity over TS/C-family colon-case switches.
    fn match_tail(
        &self,
        scrutinee: &str,
        arms: &[(Option<String>, Vec<String>)],
        indent: &str,
    ) -> Vec<String> {
        let mut out = vec![format!("{indent}switch ({scrutinee}) {{")];
        for (variant, lines) in arms {
            match variant {
                Some(v) => out.push(format!("{indent}case {v} -> {{")),
                None => out.push(format!("{indent}default -> {{")),
            }
            out.extend(lines.iter().cloned());
            out.push(format!("{indent}}}"));
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
        _in_tx: bool,
    ) -> Vec<String> {
        let row = self.fresh("__row");
        let table_sql = table_sql_name(self.ir, table);
        let record = context::build_record(self.ir, self.ir.table(table).record);
        let engine = table_db_engine(self.ir, table);
        let placeholders: Vec<&str> = record
            .fields
            .iter()
            .map(|f| field_placeholder(f, engine))
            .collect();
        let sql = format!(
            "INSERT INTO {table_sql} ({}) VALUES ({})",
            record.select_cols,
            placeholders.join(", ")
        );
        let binds = self.bind_expr(&record, &row);
        let mut out = vec![format!("{indent}var {row} = {value};")];
        out.push(format!(
            "{indent}{}.sql({sql:?}){}.update();",
            self.jdbc(),
            Self::params_chain(&binds)
        ));
        // `Dest::Discard` here means the caller only wanted the SQL
        // side effect, already emitted above -- unlike every other
        // leaf's own `discard_stmt` (always a call-chain expression,
        // safe as a bare statement), the value here is a plain local
        // variable name, which is not a valid Java `ExpressionStatement`
        // on its own; Java has no "unused local" hard error, so simply
        // emitting nothing is safe.
        if !matches!(dest, Dest::Discard) {
            apply_dest(self, dest, &row, indent, &mut out);
        }
        out
    }
    /// `db.update`'s own HIR type is a nullable record (typeck.rs: not
    /// found -> `null`) — a plain ternary on the affected-row count
    /// covers it in one line, no closure/two-return-value machinery
    /// needed the way Go's own version does (Java's block-scoping
    /// concern here is the *outer* `let`/`if`-branch hoisting problem
    /// [`collect_branching_lets`] already solves, not this verb's own
    /// internal found/not-found branch, which never spans more than one
    /// statement).
    fn db_update_tail(
        &self,
        table: TableId,
        key: &str,
        value: &str,
        dest: &Dest,
        indent: &str,
        _in_tx: bool,
    ) -> Vec<String> {
        let row = self.fresh("__row");
        let n = self.fresh("__n");
        let table_sql = table_sql_name(self.ir, table);
        let record = context::build_record(self.ir, self.ir.table(table).record);
        let engine = table_db_engine(self.ir, table);
        let non_id: Vec<&FieldCtx> = record.fields.iter().filter(|f| f.name != "id").collect();
        let assignments: Vec<String> = non_id
            .iter()
            .map(|f| format!("{} = {}", f.name, field_placeholder(f, engine)))
            .collect();
        let mut binds: Vec<String> = non_id.iter().map(|f| Self::field_bind(f, &row)).collect();
        binds.push(key.to_owned());
        let sql = format!(
            "UPDATE {table_sql} SET {} WHERE id = ?",
            assignments.join(", ")
        );
        let mut out = vec![format!("{indent}var {row} = {value};")];
        out.push(format!(
            "{indent}int {n} = {}.sql({sql:?}){}.update();",
            self.jdbc(),
            Self::params_chain(&binds)
        ));
        let result = self.fresh("__result");
        out.push(format!("{indent}var {result} = ({n} == 0) ? null : {row};"));
        if !matches!(dest, Dest::Discard) {
            apply_dest(self, dest, &result, indent, &mut out);
        }
        out
    }
    fn db_delete_tail(
        &self,
        table: TableId,
        key: &str,
        dest: &Dest,
        indent: &str,
        _in_tx: bool,
    ) -> Vec<String> {
        let n = self.fresh("__n");
        let table_sql = table_sql_name(self.ir, table);
        let sql = jdbcph(&format!("DELETE FROM {table_sql} WHERE id = ?"));
        let mut out = vec![format!(
            "{indent}int {n} = {}.sql({sql:?}).param({key}).update();",
            self.jdbc()
        )];
        if !matches!(dest, Dest::Discard) {
            apply_dest(self, dest, &format!("({n} > 0)"), indent, &mut out);
        }
        out
    }
    fn query_tail(
        &self,
        verb: Verb,
        predicate: Option<&LoweredPredicate>,
        dest: &Dest,
        indent: &str,
        _in_tx: bool,
    ) -> Vec<String> {
        match verb {
            Verb::DbQuery(table) => {
                let table_sql = table_sql_name(self.ir, table);
                let record = context::build_record(self.ir, self.ir.table(table).record);
                let record_name = self.ir.record(self.ir.table(table).record).name.clone();
                let (where_sql, binds) = self.where_clause(predicate);
                let sql = jdbcph(&format!(
                    "SELECT {} FROM {table_sql}{where_sql}",
                    record.select_cols
                ));
                let out_var = self.fresh("__rows");
                let mut out = vec![format!(
                    "{indent}java.util.List<{record_name}> {out_var} = {}.sql({sql:?}){}.query(RowMappers.{}).list();",
                    self.jdbc(),
                    Self::params_chain(&binds),
                    record_name.to_shouty_snake_case_ish()
                )];
                if !matches!(dest, Dest::Discard) {
                    apply_dest(self, dest, &out_var, indent, &mut out);
                }
                out
            }
            Verb::DbCount(table) => {
                let table_sql = table_sql_name(self.ir, table);
                let (where_sql, binds) = self.where_clause(predicate);
                let sql = jdbcph(&format!("SELECT COUNT(*) FROM {table_sql}{where_sql}"));
                let count = self.fresh("__count");
                let mut out = vec![format!(
                    "{indent}long {count} = {}.sql({sql:?}){}.query((__rs, __rowNum) -> __rs.getLong(1)).single();",
                    self.jdbc(),
                    Self::params_chain(&binds)
                )];
                if !matches!(dest, Dest::Discard) {
                    apply_dest(self, dest, &count, indent, &mut out);
                }
                out
            }
            Verb::DbDeleteWhere(table) => {
                let table_sql = table_sql_name(self.ir, table);
                let (where_sql, binds) = self.where_clause(predicate);
                let sql = jdbcph(&format!("DELETE FROM {table_sql}{where_sql}"));
                let n = self.fresh("__n");
                let mut out = vec![format!(
                    "{indent}int {n} = {}.sql({sql:?}){}.update();",
                    self.jdbc(),
                    Self::params_chain(&binds)
                )];
                if !matches!(dest, Dest::Discard) {
                    apply_dest(self, dest, &n, indent, &mut out);
                }
                out
            }
            _ => unreachable!("HirExpr::Query only ever carries a db query verb, found {verb:?}"),
        }
    }
    /// A hoisted branching local ([`collect_branching_lets`]) is
    /// declared once, above the branch, by the template's own
    /// `hoisted_locals` loop — every assignment to it here is a plain
    /// `name = value;` reassignment. Every other `Let` introduces its
    /// name fresh via `var name = value;` (Java 10+ local-variable type
    /// inference), exactly once.
    fn assign(&self, name: &str, value: &str, indent: &str) -> String {
        if self.branching_locals.contains_key(name) {
            format!("{indent}{name} = {value};")
        } else {
            format!("{indent}var {name} = {value};")
        }
    }
    fn discard_stmt(&self, value: &str, indent: &str) -> String {
        format!("{indent}{value};")
    }
    fn empty_block_stmt(&self, _indent: &str) -> Vec<String> {
        // An empty `{}` block is valid Java on its own.
        Vec::new()
    }
    /// Programmatic `TransactionTemplate.executeWithoutResult`, not
    /// `@Transactional` (Pillar 4 — the proxy self-invocation trap a
    /// same-class annotated call would carry silently). `fail`'s own
    /// `throw` propagating out of this lambda both rolls back (Spring's
    /// `TransactionCallback` contract) and re-throws unchanged to the
    /// caller — sema already forbids `return`/`publish`/non-db verbs
    /// inside a `transaction {}` block, so this lambda body is
    /// guaranteed to need no `return`-from-enclosing-method escape
    /// (which a Java lambda cannot express anyway).
    fn transaction_stmt(&self, inner_lines: Vec<String>, indent: &str) -> Vec<String> {
        let tx_field = self
            .tx_field
            .as_deref()
            .expect("a transaction block requires a bound database instance");
        let mut out = vec![format!(
            "{indent}{tx_field}.executeWithoutResult(__status -> {{"
        )];
        out.extend(inner_lines);
        out.push(format!("{indent}}});"));
        out
    }

    fn return_stmt(&self, value: Option<&str>, indent: &str) -> String {
        match value {
            Some(v) => format!("{indent}return {v};"),
            None => format!("{indent}return;"),
        }
    }
    fn fail(&self, error: RecordId, args: &[String], indent: &str) -> String {
        let name = self.ir.record(error).name.clone();
        format!("{indent}throw new {name}({});", args.join(", "))
    }
    fn publish(&self, subject: &str, value: &str, value_ty: &HirType, indent: &str) -> String {
        let payload = json_body(value, value_ty);
        format!("{indent}queue.publishJson({subject:?}, {payload});")
    }
    fn db_get(&self, table: TableId, key: &str) -> String {
        let table_sql = table_sql_name(self.ir, table);
        let record = context::build_record(self.ir, self.ir.table(table).record);
        let record_name = self.ir.record(self.ir.table(table).record).name.clone();
        let sql = jdbcph(&format!(
            "SELECT {} FROM {table_sql} WHERE id = ?",
            record.select_cols
        ));
        format!(
            "{}.sql({sql:?}).param({key}).query(RowMappers.{}).optional().orElse(null)",
            self.jdbc(),
            record_name.to_shouty_snake_case_ish()
        )
    }
    fn cache_get(&self, key: &str) -> String {
        let field = java_camel(
            self.cache_field
                .as_deref()
                .expect("cache.get requires a bound cache instance"),
        );
        format!("Schemas.fromJsonOrNull({field}.opsForValue().get({key}))")
    }
    fn cache_set(&self, key: &str, value: &str, value_ty: &HirType) -> String {
        let field = java_camel(
            self.cache_field
                .as_deref()
                .expect("cache.set requires a bound cache instance"),
        );
        let payload = json_body(value, value_ty);
        format!("{field}.opsForValue().set({key}, {payload})")
    }
    fn cache_delete(&self, key: &str) -> String {
        let field = java_camel(
            self.cache_field
                .as_deref()
                .expect("cache.delete requires a bound cache instance"),
        );
        format!("{field}.delete({key})")
    }
    fn object_store_put(&self, key: &str, value: &str, value_ty: &HirType) -> String {
        let field = java_camel(
            self.object_store_field
                .as_deref()
                .expect("object_store.put requires a bound object_store instance"),
        );
        let payload = json_body(value, value_ty);
        format!("{field}.put({key}, {payload}.getBytes(java.nio.charset.StandardCharsets.UTF_8))")
    }
    fn object_store_get(&self, key: &str) -> String {
        let field = java_camel(
            self.object_store_field
                .as_deref()
                .expect("object_store.get requires a bound object_store instance"),
        );
        format!(
            "Schemas.fromJsonOrNull({field}.get({key}) == null ? null : new String({field}.get({key}), java.nio.charset.StandardCharsets.UTF_8))"
        )
    }
    fn object_store_delete(&self, key: &str) -> String {
        let field = java_camel(
            self.object_store_field
                .as_deref()
                .expect("object_store.delete requires a bound object_store instance"),
        );
        format!("{field}.delete({key})")
    }
    fn object_store_list(&self, prefix: &str) -> String {
        let field = java_camel(
            self.object_store_field
                .as_deref()
                .expect("object_store.list requires a bound object_store instance"),
        );
        format!("{field}.list({prefix})")
    }
    fn email_send(&self, to: &str, subject: &str, body: &str) -> String {
        let field = java_camel(
            self.email_field
                .as_deref()
                .expect("email.send requires a bound email instance"),
        );
        format!("{field}.send({to}, {subject}, {body})")
    }
    fn search_index(&self, doc_id: &str, document: &str, document_ty: &HirType) -> String {
        let field = java_camel(
            self.search_field
                .as_deref()
                .expect("search.index requires a bound search instance"),
        );
        let payload = json_body(document, document_ty);
        format!("{field}.index({doc_id}, {payload})")
    }
    fn search_query(&self, query: &str) -> String {
        let field = java_camel(
            self.search_field
                .as_deref()
                .expect("search.query requires a bound search instance"),
        );
        format!("{field}.search({query})")
    }
    fn http_call(&self, url: &str, json_body_expr: &str, body_ty: &HirType) -> String {
        let field = java_camel(
            self.http_field
                .as_deref()
                .expect("external_http.request requires a bound external_http instance"),
        );
        let payload = json_body(json_body_expr, body_ty);
        format!("{field}.post({url}, {payload})")
    }
}

/// Renders `value` as JSON text for `cache.set`/`object_store.put`/
/// `search.index`/`http.call`'s payload — unlike Go/Python/TS, this
/// needs no Record/Json/scalar 3-way branch at all: `Schemas.toJson`
/// (Jackson) serializes a record, a `JsonNode`, or a boxed scalar
/// alike, so one call covers every payload type this closed verb
/// signature ever accepts. A real, disclosed simplification the other
/// three targets' own ORM/serializer choice doesn't get for free.
fn json_body(value: &str, _value_ty: &HirType) -> String {
    format!("Schemas.toJson({value})")
}

/// A minimal `SHOUTY_SNAKE_CASE`-ish transform for a `RowMappers`
/// constant name — `heck`'s own `ToShoutySnakeCase` is already
/// registered as the `shouty_snake_case` minijinja filter
/// (`ciac_codegen::template::environment`), reused here as a plain
/// trait extension so `lower.rs`'s own leaf bodies (outside any
/// template context) can spell the identical constant name.
trait ShoutySnake {
    fn to_shouty_snake_case_ish(&self) -> String;
}
impl ShoutySnake for str {
    fn to_shouty_snake_case_ish(&self) -> String {
        use heck::ToShoutySnakeCase;
        self.to_shouty_snake_case()
    }
}

#[derive(Debug, Serialize)]
pub struct ParamCtx {
    pub name: String,
    pub java_type: String,
}

/// One hoisted `JavaType name;` [`collect_branching_lets`] found.
#[derive(Debug, Serialize)]
pub struct HoistedLocalCtx {
    pub name: String,
    pub java_type: String,
}

/// Everything `logic.java.j2` needs to render one typed handler's
/// file — inline (compiler-owned, `logic/<Name>.java`) or `extern`
/// (seeded, `services/<Name>.java`, `handle` just throws
/// `UnsupportedOperationException`).
#[derive(Debug, Serialize)]
pub struct LogicFileCtx {
    pub class_name: String,
    pub is_extern: bool,
    pub params: Vec<ParamCtx>,
    pub return_type: String,
    pub needs_db: bool,
    pub needs_tx: bool,
    pub needs_cache: bool,
    pub needs_queue: bool,
    pub rust_db_field: Option<String>,
    pub rust_cache_field: Option<String>,
    /// The constructor-injected `TransactionTemplate` field/
    /// `@Qualifier` name (`{dbField}Tx`, matching the `AppState` bean
    /// this milestone adds alongside each db instance's own
    /// `{dbField}Jdbc` `JdbcClient` bean) — `Some` exactly when
    /// `needs_tx` is, since a `transaction {}` block always requires a
    /// bound database instance.
    pub java_tx_field: Option<String>,
    pub extras: Vec<context::ExtraDepCtx>,
    /// Every `(name, JavaType)` this handler's body needs hoisted above
    /// a branching `if`/`switch` (see [`collect_branching_lets`]).
    pub hoisted_locals: Vec<HoistedLocalCtx>,
    pub body: Vec<String>,
    /// Every record/enum type referenced (params, return, hoisted
    /// locals, or the body) that needs a `schemas` import.
    pub schema_imports: Vec<String>,
}

/// Builds the render context for one typed handler node. `name` is the
/// handler's declared name (`node.component.name()`).
pub fn render(ir: &NormalizedIr, name: &str, hir: &HandlerBody) -> LogicFileCtx {
    let needs = scan(ir, hir);
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

    let params: Vec<ParamCtx> = hir
        .params
        .iter()
        .map(|(n, ty)| ParamCtx {
            name: n.clone(),
            java_type: java_type(ir, ty),
        })
        .collect();

    let record_by_name: HashMap<String, RecordId> =
        ir.records().map(|(id, r)| (r.name.clone(), id)).collect();

    let mut branching = Vec::new();
    let java_tx_field = access
        .rust_db_field
        .as_ref()
        .map(|f| format!("{}Tx", java_camel(f)));
    let body = match &hir.body {
        Some(stmts) => {
            collect_branching_lets(hir, stmts, &mut branching);
            let syntax = JavaSyntax {
                ir,
                tx_field: java_tx_field.clone(),
                cache_field: access.rust_cache_field.clone(),
                object_store_field,
                email_field,
                search_field,
                http_field,
                record_by_name,
                tmp: Cell::new(0),
                branching_locals: branching.iter().cloned().collect(),
            };
            lower::lower_body_stmt(&syntax, ir, hir, "        ")
        }
        None => {
            vec!["        throw new UnsupportedOperationException(\"not implemented\");".to_owned()]
        }
    };

    let needs_tx = body_has_transaction(hir.body.as_deref().unwrap_or(&[]));

    // `needs.records`/`needs.enums` cover every `schemas`-package type
    // this handler's signature or body actually spells by name — a
    // record class name or a named enum type (`VideoStatus`), each
    // living in its own file (`record.java.j2`/`RecordEnum.java.j2`)
    // under the same package, so both lists need their own import
    // line.
    let mut schema_imports: Vec<String> = needs
        .records
        .iter()
        .map(|id| ir.record(*id).name.clone())
        .collect();
    schema_imports.extend(needs.enums.iter().cloned());
    schema_imports.sort();
    schema_imports.dedup();

    LogicFileCtx {
        class_name: name.to_owned(),
        is_extern: hir.body.is_none(),
        params,
        return_type: java_type(ir, &hir.return_ty),
        needs_db: access.db.is_some(),
        needs_tx,
        needs_cache: access.cache_expr.is_some(),
        needs_queue: needs.queue,
        rust_db_field: access.rust_db_field,
        rust_cache_field: access.rust_cache_field,
        java_tx_field,
        extras,
        hoisted_locals: branching
            .into_iter()
            .map(|(name, ty)| HoistedLocalCtx {
                name,
                java_type: java_type(ir, &ty),
            })
            .collect(),
        schema_imports,
        body,
    }
}

/// Whether `stmts` contains a `transaction {}` block anywhere (directly
/// or nested inside `if`/`match`) — the shared `Needs` scanner has no
/// dedicated flag for this (see Go's own `needs_sql_pkg`/`needs_fmt`
/// textual-fallback precedent for the identical situation), so this
/// backend walks the statement list itself rather than re-deriving it
/// from the rendered body's own text.
fn body_has_transaction(stmts: &[HirStmt]) -> bool {
    stmts.iter().any(|stmt| match stmt {
        HirStmt::Transaction { .. } => true,
        HirStmt::Let { value, .. } | HirStmt::Expr(value) => expr_has_transaction(value),
        HirStmt::Return(Some(value)) => expr_has_transaction(value),
        HirStmt::Return(None) | HirStmt::Fail { .. } | HirStmt::Publish { .. } => false,
    })
}

fn expr_has_transaction(expr: &HirExpr) -> bool {
    match expr {
        HirExpr::If {
            then_branch,
            else_branch,
            ..
        } => body_has_transaction(then_branch) || body_has_transaction(else_branch),
        HirExpr::Match { arms, .. } => arms.iter().any(|arm| body_has_transaction(&arm.body)),
        _ => false,
    }
}
