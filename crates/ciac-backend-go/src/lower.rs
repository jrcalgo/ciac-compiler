//! Direct lowering of the typed HIR (`ciac_ir::hir`) into Go source
//! (`24UpdatePlan.md` M4).
//!
//! The walker (block/tail shaping, precedence, enum-literal use-site
//! recovery, float-literal fidelity, divergence truncation) lives in
//! `ciac_codegen::lower`; [`GoSyntax`] supplies only the leaf
//! constructors genuinely specific to this target. Go runs in
//! `Orientation::Statement` — the same mode Python/TS already
//! exercise — since a `{}` block is not an expression in Go the way
//! it is in Rust.
//!
//! **The error-idiom amendment, consumed for real here:** every
//! fallible operation (every `db.*`/`cache.*`/`object_store.*`/
//! `email.*`/`search.*`/`http.*` verb) needs its own `if err != nil {
//! ... }` before its result is usable at all — there is no
//! expression-position error-propagation operator the way Rust's `?`
//! lets a single expression string carry a fallible call even in
//! `Expression` orientation. [`GoSyntax::fallible_tail`] is the one
//! seam every fallible leaf below routes through: declare `(value,
//! err)` via `:=`, check `err`, `return <zero>, err` on failure,
//! apply `dest` to the value on success — see its own doc for exactly
//! why a *shared* mutable `err`/temp pair across sibling statements
//! doesn't work and per-call-site fresh names are needed instead.
//!
//! **Real `sql.Tx` transaction atomicity** (Pillar 4), matching the
//! Rust backend's own atomicity (`26UpdatePlan.md` M1; see
//! `RustSyntax::transaction_expr`'s doc): [`transaction_stmt`]
//! (../HostSyntax) opens a real `*sql.Tx` via `BeginTx`, `defer`s a
//! `Rollback` (a safe no-op once `Commit` has already run — ordinary
//! Go idiom, not a hand-rolled one), and every db verb inside routes
//! through it instead of the pool — `handle(in_tx)`'s own job,
//! mirroring the `in_tx: bool` flag every other `Statement`-oriented
//! target already threads through the shared dispatcher.
//!
//! **What stays out of scope this milestone, matching the TS arc's
//! own M4 precedent exactly (`ciac-backend-ts`'s `TsBackend::supports`
//! at v0.23 M4, read directly rather than assumed):** every leaf below
//! is implemented — `object_store.*`/`email.*`/`search.*`/`http.*`
//! included — so the trait compiles completely with no
//! `unimplemented!()` leaf reachable, but the *component* kinds that
//! request those capabilities (`Component::ObjectStore`/`Email`/
//! `Search`/`ExternalHttp`, plus `Cache`/`Auth`) stay refused in
//! `GoBackend::supports` until M6/M7 add their client wrappers and
//! gate them for real. `typed-handlers.ciac` (needs `object_store`)
//! and `extras-verbs.ciac`/`typed-video.ciac` (need the M6/M7
//! ontology) stay `CIAC0011`-refused this milestone; `domain-
//! orders.ciac`/`query-verbs.ciac` (db-only) are this milestone's
//! actual proving examples, exactly as they were for TS.

use ciac_codegen::lower::{
    self, apply_dest, fidelity_checked_float, strip_outer_parens, Dest, HostSyntax, IndexKey,
    LoweredPredTerm, LoweredPredicate, Orientation, PredValue,
};
use ciac_codegen::model::{self as context, FieldCtx, RecordCtx};
use ciac_codegen::template::sqlph;
use ciac_ir::{
    BinOp, HandlerBody, HirExpr, HirStmt, HirType, NormalizedIr, PredOp, RecordId, TableId, UnOp,
    Verb,
};
use heck::ToSnakeCase;
use serde::Serialize;

use crate::filters::go_pascal;

/// Go type annotation for a HIR type — a handler *signature* concern
/// (param/return types, and the hoisted-`let` declarations
/// [`collect_branching_lets`] needs a real type for), not part of the
/// `HostSyntax` body contract. `Option<T>` becomes `*T` (idiomatic Go
/// optionality — `nil` reads as absent everywhere a `db.get`/similar
/// result flows). A bare (field-less) `Enum` type never appears here,
/// mirroring Rust's own `rust_type`: the language surface only ever
/// puts an enum in a *record field* position, resolved through
/// `field_access_enum_name`, never directly in a param/return/local
/// position.
pub fn go_type(ir: &NormalizedIr, ty: &HirType) -> String {
    match ty {
        HirType::Str | HirType::Uuid => "string".to_owned(),
        HirType::Int => "int64".to_owned(),
        HirType::Float => "float64".to_owned(),
        HirType::Bool => "bool".to_owned(),
        HirType::Timestamp => "time.Time".to_owned(),
        // `json.RawMessage`, matching `FieldTypeKind::Json`'s own
        // mapping (`filters::go_type_of`) exactly -- every other
        // target keeps one representation for both the HIR type and
        // the record-field type (Rust: `serde_json::Value` both
        // places; Python: `dict[str, Any]`; TS: `unknown`); an earlier
        // version of this match arm used `any` instead, a real,
        // structural inconsistency with the field-level mapping caught
        // live (`go vet`: "is not an interface" on a `Json` field
        // indexed through [`HostSyntax::index`], which assumed an
        // `any`-typed base).
        HirType::Json => "json.RawMessage".to_owned(),
        HirType::Enum { .. } => {
            unreachable!("a bare enum type never appears in a param/return/local position")
        }
        HirType::Record(id) => qualified_record_name(ir, *id),
        HirType::Option(inner) => format!("*{}", go_type(ir, inner)),
        HirType::List(inner) => format!("[]{}", go_type(ir, inner)),
        HirType::Unit | HirType::Never => "struct{}".to_owned(),
    }
}

/// Every record and enum type this backend lowers into typed-handler
/// Go source lives in the separate `internal/schemas` *package*
/// (`schemas.go.j2`) — unlike Python/TS/Rust, where a handler's own
/// module can reference a sibling record type bare (Python: same
/// interpreter namespace via import; Rust/TS: an explicit `use`/
/// `import` line brings the bare name into scope). Go's package system
/// has no bare-name imports: every cross-package reference needs its
/// package qualifier at the *use* site, not just at the top of the
/// file — so every leaf below that spells a record or enum type name
/// does it through this one qualifier, never the bare
/// `ir.record(id).name`/`ir.table(id).name` directly.
fn qualified_record_name(ir: &NormalizedIr, record: RecordId) -> String {
    format!("schemas.{}", ir.record(record).name)
}

/// The zero value for [`go_type`]'s own mapping — used for every
/// fallible leaf's `return <zero>, err` error path (`fallible_tail`)
/// and for `return_stmt(None, ..)`.
fn go_zero(ir: &NormalizedIr, ty: &HirType) -> String {
    match ty {
        HirType::Str | HirType::Uuid => "\"\"".to_owned(),
        HirType::Int => "0".to_owned(),
        HirType::Float => "0".to_owned(),
        HirType::Bool => "false".to_owned(),
        HirType::Timestamp => "time.Time{}".to_owned(),
        HirType::Json => "nil".to_owned(),
        HirType::Enum { .. } => unreachable!("a bare enum type never appears in return position"),
        HirType::Record(id) => format!("{}{{}}", qualified_record_name(ir, *id)),
        HirType::Option(_) => "nil".to_owned(),
        HirType::List(_) => "nil".to_owned(),
        HirType::Unit | HirType::Never => "struct{}{}".to_owned(),
    }
}

fn record_class_name(ir: &NormalizedIr, record: RecordId) -> String {
    qualified_record_name(ir, record)
}

/// `local_name`, duplicated from `ciac_codegen::lower::dispatch` (a
/// private helper there): a [`HirExpr::Local`]'s declared name when it
/// names a parameter, else a synthesized `v<slot>` for a `let` local.
/// Needed here (not just inside the shared walker) to build the
/// hoisted-`var` declaration list — see [`collect_branching_lets`].
fn local_name(body: &HandlerBody, slot: u32) -> String {
    let slot = slot as usize;
    if slot < body.params.len() {
        body.params[slot].0.clone()
    } else {
        format!("v{slot}")
    }
}

/// Collects every `(name, HirType)` a `HirStmt::Let` binds *whose
/// value is an `if`/`match` expression* — the only shape where
/// `Dest::Assign(name)` reaches more than one branch, each in its own
/// Go block scope. Those names need a `var name T` hoisted above the
/// branch (Go's `if {} else {}` introduces a real block scope, so a
/// `:=`-declared name inside one branch is invisible after the
/// statement ends — the same problem TS's own `collect_branching_lets`
/// solves for `let`/`const`) and a bare `name = value` at each
/// branch's own assignment site instead of a fresh `:=`; every other
/// `Let` assigns exactly once and gets a plain `name := value` at that
/// one site instead. Recurses into `if`/`match` branch bodies and
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

/// The fixed name every db verb's transaction handle is bound to
/// inside a `transaction {}` block. A plain constant (not a fresh
/// counter name, unlike `fresh`'s own temps): `transaction {}` blocks
/// cannot nest (sema-enforced, matching every other target's own
/// single-transaction-at-a-time assumption — see TS's identical
/// `__tx` choice), so one fixed name is safe for the whole handler.
const TX_HANDLE: &str = "__tx";

/// The `Orientation::Statement` `HostSyntax` implementation for this
/// target: `if`/`match`/every fallible verb decompose into a line
/// sequence applied to a [`Dest`], same as Python/TS. Holds
/// `db_field`/`cache_field`/... (resolved once per handler, since
/// every verb leaf of a given kind needs the same `AppState` field)
/// and `zero_return` (this handler's own `go_zero(return_ty)`, needed
/// by every `fallible_tail` call regardless of which verb).
struct GoSyntax<'a> {
    ir: &'a NormalizedIr,
    db_engine: &'static str,
    db_field: Option<String>,
    cache_field: Option<String>,
    object_store_field: Option<String>,
    email_field: Option<String>,
    search_field: Option<String>,
    http_field: Option<String>,
    /// 27UpdatePlan.md M7: the *declared* capability instance name
    /// (e.g. `"Sessions"` from `cache Sessions`), distinct from the
    /// `*_field` names above (the Go `AppState` field, e.g.
    /// `CacheSessions`) -- `World`'s own cache/object-store/email/
    /// search/http maps are keyed by this declared name, matching
    /// `given.cache`/`given.store`/etc.'s own `instance` field in
    /// scenario JSON, mirroring Rust's/TypeScript's own
    /// `cache_instance`/... accessors exactly.
    cache_instance: Option<String>,
    object_store_instance: Option<String>,
    email_instance: Option<String>,
    search_instance: Option<String>,
    http_instance: Option<String>,
    zero_return: String,
    /// A handler body can call more than one fallible verb within the
    /// same block scope (e.g. two `db.insert`s inside one
    /// `transaction {}`) — every temp `fallible_tail`/a db-verb tail
    /// leaf declares directly into the caller's block (not inside its
    /// own closure, which would need its own `defer`-free scope
    /// anyway) is suffixed with a number from this counter instead of
    /// reusing a bare name, so two sequential fallible calls never
    /// collide on `:=` redeclaration rules.
    tmp: std::cell::Cell<u32>,
    /// `(name, HirType)` pairs `collect_branching_lets` found — see
    /// its own doc and `assign`'s.
    branching_locals: std::collections::HashMap<String, HirType>,
}

impl GoSyntax<'_> {
    fn fresh(&self, base: &str) -> String {
        let n = self.tmp.get();
        self.tmp.set(n + 1);
        format!("{base}{n}")
    }

    /// The `*sql.DB`-or-`*sql.Tx` handle a db verb reaches through:
    /// the bound pool normally, or the transaction-scoped `*sql.Tx`
    /// (see [`TX_HANDLE`]) when `in_tx`.
    fn handle(&self, in_tx: bool) -> String {
        if in_tx {
            TX_HANDLE.to_owned()
        } else {
            format!(
                "st.{}",
                go_pascal(
                    self.db_field
                        .clone()
                        .expect("a db verb requires a bound database instance")
                )
            )
        }
    }

    fn cache_handle(&self) -> String {
        format!(
            "st.{}",
            go_pascal(
                self.cache_field
                    .clone()
                    .expect("a cache verb requires a bound cache instance")
            )
        )
    }

    fn object_store_state_field(&self) -> String {
        go_pascal(
            self.object_store_field
                .clone()
                .expect("an object_store verb requires a bound object_store instance"),
        )
    }

    /// 27UpdatePlan.md M7: the declared instance name a `cache.*` verb
    /// call's world-guard branch resolves against -- `.expect()`, not
    /// a fallback, because sema already refuses a `cache.*` call with
    /// no bound `cache` capability before codegen ever runs.
    fn cache_instance(&self) -> String {
        self.cache_instance
            .clone()
            .expect("a cache verb requires a bound cache instance")
    }
    fn object_store_instance(&self) -> String {
        self.object_store_instance
            .clone()
            .expect("an object_store verb requires a bound object_store instance")
    }
    fn email_instance(&self) -> String {
        self.email_instance
            .clone()
            .expect("an email verb requires a bound email instance")
    }
    fn search_instance(&self) -> String {
        self.search_instance
            .clone()
            .expect("a search verb requires a bound search instance")
    }
    fn http_instance(&self) -> String {
        self.http_instance
            .clone()
            .expect("an external_http verb requires a bound external_http instance")
    }

    /// 27UpdatePlan.md M7: `db.query`/`count`/`delete_where`'s `where`
    /// clause compiled into a Go boolean expression evaluated per-row
    /// against `world.Row` (a `map[string]any` JSON-decoded document)
    /// -- `World`'s own `queryWhere`'s only filter is a caller-supplied
    /// predicate closure, so every term (not just `Eq`) needs to be
    /// compiled here, the same reason Rust's/TypeScript's own
    /// `world_predicate_expr` exists. `"true"` (matches everything)
    /// for no predicate at all.
    fn world_predicate_expr(&self, predicate: Option<&LoweredPredicate>) -> String {
        let Some(predicate) = predicate else {
            return "true".to_owned();
        };
        if predicate.terms.is_empty() {
            return "true".to_owned();
        }
        predicate
            .terms
            .iter()
            .map(|term| self.world_predicate_term_expr(term))
            .collect::<Vec<_>>()
            .join(" && ")
    }

    fn world_pred_value_expr(&self, term: &LoweredPredTerm) -> String {
        match &term.value {
            PredValue::EnumVariant(v) => format!("{v:?}"),
            PredValue::BoolLit(b) => b.to_string(),
            PredValue::Rendered(s) => s.clone(),
        }
    }

    /// `world.JSONEq`/`world.Contains`/`world.Lt`/... take `any`, so
    /// `Eq`/`NotEq` need no per-type conversion at all (JSON-marshal
    /// comparison already treats an `int64` production value and the
    /// `float64` a JSON-decoded row carries as equal); `Lt`/`LtEq`/
    /// `Gt`/`GtEq` do, since Go's `map[string]any` values decode to
    /// exactly the two ordered types `world.compare` accepts
    /// (`float64` for every numeric field, `string` for
    /// Str/Uuid/Timestamp -- an RFC3339 timestamp orders correctly as
    /// a plain string, the same trick Rust's own `f64`-uniform
    /// simplification takes for numerics).
    fn world_predicate_term_expr(&self, term: &LoweredPredTerm) -> String {
        let field = &term.field;
        let value = self.world_pred_value_expr(term);
        match term.op {
            PredOp::Eq => format!("world.JSONEq(row[{field:?}], {value})"),
            PredOp::NotEq => format!("!world.JSONEq(row[{field:?}], {value})"),
            PredOp::Contains => format!("world.Contains(row[{field:?}], {value})"),
            PredOp::Lt | PredOp::LtEq | PredOp::Gt | PredOp::GtEq => {
                let conv = match term.field_ty {
                    HirType::Int => format!("float64({value})"),
                    HirType::Timestamp => format!("({value}).Format(time.RFC3339Nano)"),
                    _ => value,
                };
                let func = match term.op {
                    PredOp::Lt => "world.Lt",
                    PredOp::LtEq => "world.LtEq",
                    PredOp::Gt => "world.Gt",
                    PredOp::GtEq => "world.GtEq",
                    _ => unreachable!("guarded by the outer match"),
                };
                format!("{func}(row[{field:?}], {conv})")
            }
        }
    }

    /// A field's write-side bind expression: `{base}.{PascalField}`.
    fn bind_expr(&self, field: &FieldCtx, base: &str) -> String {
        format!("{base}.{}", go_pascal(field.name.clone()))
    }

    /// The `Scan`-target list for one row of `record`, e.g. `&__row.ID,
    /// &__row.Title` — `database/sql`'s `Scan` assigns a `TEXT` column
    /// straight into a named-string-type field (an enum's own storage
    /// shape) via reflection, so no per-field coercion is needed the
    /// way TS's SQLite branch needs (see `bind_expr`'s own simplicity
    /// by contrast) — a real, disclosed simplification `database/sql`
    /// gives Go over `better-sqlite3`'s untyped `unknown` rows.
    fn scan_targets(&self, record: &RecordCtx, row_var: &str) -> String {
        record
            .fields
            .iter()
            .map(|f| format!("&{row_var}.{}", go_pascal(f.name.clone())))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Builds a ` WHERE ..` clause (empty string if there's no
    /// predicate) and the ordered bind expressions it needs, written
    /// Postgres-style (`$N`) and rewritten per-engine by `sqlph`, the
    /// same discipline every other SQL string this backend emits uses
    /// (v0.13 M1, unchanged).
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
            conditions.push(format!("{field} {op} ${idx}"));
            if term.op == PredOp::Contains {
                binds.push(format!("\"%\" + {bind_expr} + \"%\""));
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

    /// The error-idiom amendment's one shared implementation seam
    /// (see the module doc): declares `(value, err) := call_expr`
    /// fresh (never reusing a name across sibling fallible calls —
    /// see `tmp`'s own doc for why that matters), checks `err`, and
    /// applies `dest` to `value` on success via the shared
    /// `apply_dest` (which itself calls `assign`/`return_stmt`/
    /// `discard_stmt` below — so `Dest::Assign`'s branching-vs-fresh
    /// `:=`/`=` distinction and `Dest::Return`'s `, nil` suffix both
    /// apply here automatically, with no special-casing needed in
    /// this one shared helper).
    fn fallible_tail(&self, call_expr: &str, dest: &Dest, indent: &str) -> Vec<String> {
        let v = self.fresh("__v");
        let err = self.fresh("__err");
        let mut out = vec![format!("{indent}{v}, {err} := {call_expr}")];
        out.push(format!("{indent}if {err} != nil {{"));
        out.push(format!("{indent}\treturn {}, {err}", self.zero_return));
        out.push(format!("{indent}}}"));
        apply_dest(self, dest, &v, indent, &mut out);
        out
    }

    /// Renders `value` as a Go `any` for `search.index`'s `document`/
    /// `http.call`'s body — mirrors Python's `json_body`/Rust's
    /// `json_value`/TS's `json_body` 3-way (Record/Json/else) branch.
    /// Unexercised this milestone (`Search`/`ExternalHttp` stay
    /// `CIAC0011`-refused until M7 — see the module doc), kept correct
    /// for the trait's sake, matching every other target's own choice
    /// to implement these leaves ahead of their component gate.
    fn json_body(&self, value: &str, value_ty: &HirType) -> String {
        match value_ty {
            HirType::Record(_) | HirType::Json => value.to_owned(),
            _ => format!("map[string]any{{\"value\": {value}}}"),
        }
    }
}

const SEARCH_INDEX_NAME: &str = "documents";

impl HostSyntax for GoSyntax<'_> {
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
        format!("{base}.{}", go_pascal(field.to_owned()))
    }
    /// `Json` indexing (Pillar 2): a closure returning `(any, bool)`-
    /// checked map access, panicking (Go's nearest analog to Python's
    /// `KeyError`/TS's `throw` at this position — recovered, if at
    /// all, by whatever top-level `recover()` middleware the caller
    /// installs) rather than silently propagating a zero value.
    /// Unexercised this milestone (no open M4 example indexes a `Json`
    /// field — see the module doc), kept correct for the trait's sake.
    /// `base` is always `Json`-typed (typeck's own indexing rule), so
    /// at the Go-syntax level it's a `json.RawMessage` ([]byte) — a
    /// concrete slice type, not an interface, so it needs
    /// `json.Unmarshal`ing into a real `map[string]any` before a key
    /// lookup is possible at all (a bare `.(map[string]any)` type
    /// assertion, this leaf's first version, is a `go vet` error:
    /// "is not an interface" — found live generating an indexing
    /// example, alongside the `go_type` mapping fix that caused it).
    fn index(&self, base: &str, key: IndexKey<'_>) -> String {
        match key {
            // The panic message is a plain, unquoted `{s}` interpolation
            // (not `{s:?}`) inside its own already-delimited Go string
            // literal -- using `{s:?}` here too (as the map-key
            // position correctly does, needing a real quoted Go string
            // literal) would nest a second pair of double quotes inside
            // the first, producing invalid Go source
            // (`panic("KeyError: "label"")`). Found live: generating
            // this leaf against a `Json`-indexing example and reading
            // `gofmt`'s own rejection.
            IndexKey::StrKey(s) => format!(
                "func() any {{ var __m map[string]any; if __err := json.Unmarshal({base}, &__m); __err != nil {{ panic(__err) }}; __v, __ok := __m[{s:?}]; if !__ok {{ panic(\"KeyError: '{s}'\") }}; return __v }}()"
            ),
            IndexKey::Expr(e) => format!(
                "func() any {{ __k := {e}; var __m map[string]any; if __err := json.Unmarshal({base}, &__m); __err != nil {{ panic(__err) }}; __v, __ok := __m[__k]; if !__ok {{ panic(fmt.Sprintf(\"IndexError: %v\", __k)) }}; return __v }}()"
            ),
        }
    }
    fn uuid_new(&self) -> String {
        "uuid.NewString()".to_owned()
    }
    fn timestamp_now(&self) -> String {
        "time.Now().UTC()".to_owned()
    }
    /// A Go named-string-type constant, `{EnumName}{Variant}` — see
    /// `schemas.go.j2`'s own enum type generation (v0.24 M3). `Some`-
    /// only, mirroring Rust's own choice: Go's enums are a real named
    /// type too, so a bare variant with no enclosing context to name
    /// it panics rather than guessing.
    fn enum_literal(&self, enum_name: Option<&str>, variant: &str) -> String {
        let name = enum_name.expect("Go enum literals need a named enclosing type");
        format!("schemas.{name}{variant}")
    }
    /// Go has no struct-spread/update syntax: a fresh construction is
    /// a plain composite literal (`{RecordName}{Field: value, ...}`);
    /// a functional update (`..base`) needs a copy-then-override
    /// closure (`func() T { v := base; v.Field = value; ...; return v
    /// }()`) — struct assignment in Go already copies by value, so `v
    /// := base` is a real, independent copy, matching CIaC's own
    /// by-value record semantics exactly.
    fn record_cons(
        &self,
        record_name: &str,
        fields: &[(String, String)],
        base: Option<&str>,
    ) -> String {
        // `record_name` arrives bare from the shared dispatcher (see
        // `dispatch.rs::lower_record_cons`) — qualified here, once, for
        // the same reason `qualified_record_name` exists at all.
        match base {
            None => {
                let field_strs: Vec<String> = fields
                    .iter()
                    .map(|(name, value)| format!("{}: {value}", go_pascal(name.clone())))
                    .collect();
                format!("schemas.{record_name}{{{}}}", field_strs.join(", "))
            }
            Some(base) => {
                let mut out = format!("func() schemas.{record_name} {{ __v := {base}; ");
                for (name, value) in fields {
                    out.push_str(&format!("__v.{} = {value}; ", go_pascal(name.clone())));
                }
                out.push_str("return __v }()");
                out
            }
        }
    }
    /// Go's own `/` on `int64` already truncates toward zero (unlike
    /// JS, which needs `Math.trunc` — see TS's own comment on this
    /// exact leaf), so no special-case is needed for `Int / Int`: Go's
    /// native integer division is already Rust-`i64`-division-
    /// compatible, a real simplification this backend gets for free.
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
            "{indent}if {} {{",
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
    /// A real `switch` statement over the enum's string value, one
    /// `case` per variant — no explicit `break` needed (Go's `switch`
    /// doesn't fall through by default, unlike TS/C-family switches,
    /// so this is actually simpler than TS's own `match_tail`, which
    /// has to compute whether a `break` is needed at all).
    fn match_tail(
        &self,
        scrutinee: &str,
        arms: &[(Option<String>, Vec<String>)],
        indent: &str,
    ) -> Vec<String> {
        let mut out = vec![format!("{indent}switch {scrutinee} {{")];
        for (variant, lines) in arms {
            match variant {
                Some(v) => out.push(format!("{indent}case {v:?}:")),
                None => out.push(format!("{indent}default:")),
            }
            out.extend(lines.iter().cloned());
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
        let err = self.fresh("__err");
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
        let mut out = vec![format!("{indent}{row} := {value}")];
        // v0.24 M9's world guard: the one HostSyntax leaf `ciac sim`
        // needs to fake, mirroring `SimWorld::db_insert_checked`/
        // `dbInsertChecked` exactly (including the fact the failure
        // effect it checks is named `"db.commit"`, not `"db.insert"` --
        // the FailureEngine rule vocabulary every backend shares).
        // Reached regardless of `in_tx`: a standalone `db.insert` and
        // one inside `transaction { .. }` are both simulated the same
        // way -- `st` (the outer AppState) stays in scope through a
        // transaction block the same way `self.handle(in_tx)`'s own
        // `st.{}.BeginTx` call site already relies on.
        out.push(format!("{indent}if st.World != nil {{"));
        out.push(format!(
            "{indent}\tif {err} := st.World.DBInsertChecked({table_snake:?}, {row}); {err} != nil {{"
        ));
        out.push(format!("{indent}\t\treturn {}, {err}", self.zero_return));
        out.push(format!("{indent}\t}}"));
        out.push(format!("{indent}}} else {{"));
        out.push(format!(
            "{indent}\tif _, {err} := {handle}.ExecContext(ctx, {sql:?}, {}); {err} != nil {{",
            binds.join(", ")
        ));
        out.push(format!("{indent}\t\treturn {}, {err}", self.zero_return));
        out.push(format!("{indent}\t}}"));
        out.push(format!("{indent}}}"));
        apply_dest(self, dest, &row, indent, &mut out);
        out
    }
    /// `db.update`'s own HIR type is `Option<Record>` (typeck.rs: not
    /// found -> `None`), so its own value has a two-way branch
    /// (found/not-found) *inside* one verb call, distinct from the
    /// source-level `if`/`match` branches [`collect_branching_lets`]
    /// hoists for. Building the whole thing as one `func() (*T, error)
    /// {{ .. }}()` closure (exactly [`Self::db_get`]'s own shape) and
    /// routing it through [`Self::fallible_tail`] like every other
    /// fallible leaf sidesteps two real bugs an earlier version of
    /// this method had: (1) `nil` is untyped, so passing it straight
    /// to `apply_dest`'s `Dest::Discard`/`Dest::Assign` arms produced
    /// `_ = nil`/`name := nil`, both compile errors ("use of untyped
    /// nil") -- the closure's own `(*T, error)` return type gives it a
    /// type before `apply_dest` ever sees it; (2) applying `dest`
    /// separately inside each branch of an *internal* if/else (one
    /// `db.update` verb call is not itself a source-level `if`/`match`,
    /// so `collect_branching_lets` never hoists a `var` for it) would
    /// `:=`-declare a `Dest::Assign` name block-scoped to whichever
    /// branch ran, undefined immediately after the if/else ends -- one
    /// closure means exactly one `apply_dest` call site, so this can't
    /// happen. Found live: generating `query-verbs.ciac`'s `Replace`
    /// handler and reading the output caught both.
    fn db_update_tail(
        &self,
        table: TableId,
        key: &str,
        value: &str,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let table_snake = self.ir.table(table).name.to_snake_case();
        let record = context::build_record(self.ir, self.ir.table(table).record);
        let record_name = record_class_name(self.ir, self.ir.table(table).record);
        let handle = self.handle(in_tx);
        let mut binds: Vec<String> = record
            .fields
            .iter()
            .filter(|f| f.name != "id")
            .map(|f| format!("__row.{}", go_pascal(f.name.clone())))
            .collect();
        binds.push(key.to_owned());
        let sql = sqlph(
            &format!(
                "UPDATE {table_snake} SET {} WHERE id = {}",
                record.update_assignments, record.update_where
            ),
            self.db_engine,
        );
        // 27UpdatePlan.md M7: a `st.World != nil` early-return branch
        // inside the same closure, ahead of the real
        // `ExecContext`/`RowsAffected` path — `World.DBUpdateChecked`
        // is batch-aware internally (queues instead of applying when
        // `transaction_stmt`'s own `BeginWorldBatch` is active), so no
        // `in_tx`-specific branching is needed here at all, mirroring
        // TypeScript's own `dbUpdateChecked` design exactly.
        let closure = format!(
            "func() (*{record_name}, error) {{ __row := {value}; if st.World != nil {{ var __wout {record_name}; __ok, __werr := st.World.DBUpdateChecked({table_snake:?}, {key}, __row, &__wout); if __werr != nil {{ return nil, __werr }}; if !__ok {{ return nil, nil }}; return &__wout, nil }}; __result, __err := {handle}.ExecContext(ctx, {sql:?}, {}); if __err != nil {{ return nil, __err }}; __n, __err2 := __result.RowsAffected(); if __err2 != nil {{ return nil, __err2 }}; if __n == 0 {{ return nil, nil }}; return &__row, nil }}()",
            binds.join(", ")
        );
        self.fallible_tail(&closure, dest, indent)
    }
    fn db_delete_tail(
        &self,
        table: TableId,
        key: &str,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let table_snake = self.ir.table(table).name.to_snake_case();
        let handle = self.handle(in_tx);
        let sql = sqlph(
            &format!("DELETE FROM {table_snake} WHERE id = $1"),
            self.db_engine,
        );
        // 27UpdatePlan.md M7: `{out_var}` is declared once, above the
        // world/production split, and only ever *assigned* inside
        // each branch -- Go's `if {} else {}` opens a real block
        // scope (unlike Rust's `if let`/Python's `if/else`, which
        // share the enclosing scope), so a `:=`-declared name inside
        // one branch would be invisible after the block closes,
        // mirroring TypeScript's own M6 fix for the identical bug
        // class (found live there via `tsc`; applied here from the
        // start rather than re-discovering it).
        let out_var = self.fresh("__out");
        let mut out = vec![format!("{indent}var {out_var} bool")];
        out.push(format!("{indent}if st.World != nil {{"));
        let v = self.fresh("__v");
        let werr = self.fresh("__err");
        out.push(format!(
            "{indent}\t{v}, {werr} := st.World.DBDeleteChecked({table_snake:?}, {key})"
        ));
        out.push(format!("{indent}\tif {werr} != nil {{"));
        out.push(format!("{indent}\t\treturn {}, {werr}", self.zero_return));
        out.push(format!("{indent}\t}}"));
        out.push(format!("{indent}\t{out_var} = {v}"));
        out.push(format!("{indent}}} else {{"));
        let result = self.fresh("__result");
        let err = self.fresh("__err");
        out.push(format!(
            "{indent}\t{result}, {err} := {handle}.ExecContext(ctx, {sql:?}, {key})"
        ));
        out.push(format!("{indent}\tif {err} != nil {{"));
        out.push(format!("{indent}\t\treturn {}, {err}", self.zero_return));
        out.push(format!("{indent}\t}}"));
        let n = self.fresh("__n");
        let nerr = self.fresh("__err");
        out.push(format!("{indent}\t{n}, {nerr} := {result}.RowsAffected()"));
        out.push(format!("{indent}\tif {nerr} != nil {{"));
        out.push(format!("{indent}\t\treturn {}, {nerr}", self.zero_return));
        out.push(format!("{indent}\t}}"));
        out.push(format!("{indent}\t{out_var} = {n} > 0"));
        out.push(format!("{indent}}}"));
        apply_dest(self, dest, &out_var, indent, &mut out);
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
                let table_snake = self.ir.table(table).name.to_snake_case();
                let record = context::build_record(self.ir, self.ir.table(table).record);
                let record_name = record_class_name(self.ir, self.ir.table(table).record);
                let handle = self.handle(in_tx);
                let (where_sql, binds) = self.where_clause(predicate);
                let sql = sqlph(
                    &format!(
                        "SELECT {} FROM {table_snake}{where_sql}",
                        record.select_cols
                    ),
                    self.db_engine,
                );
                let world_pred = self.world_predicate_expr(predicate);
                // 27UpdatePlan.md M7: `{out_var}` hoisted above the
                // world/production split (see `db_delete_tail`'s own
                // doc on why) — assigned once via `World.DBQuery`'s
                // own JSON round-trip in the world branch, via the
                // existing `Scan` loop in the production branch.
                let out_var = self.fresh("__out");
                let mut out = vec![format!("{indent}var {out_var} []{record_name}")];
                out.push(format!("{indent}if st.World != nil {{"));
                let pred_var = self.fresh("__pred");
                out.push(format!(
                    "{indent}\t{pred_var} := func(row world.Row) bool {{ return {world_pred} }}"
                ));
                let werr = self.fresh("__err");
                out.push(format!(
                    "{indent}\tif {werr} := st.World.DBQuery({table_snake:?}, {pred_var}, &{out_var}); {werr} != nil {{"
                ));
                out.push(format!("{indent}\t\treturn {}, {werr}", self.zero_return));
                out.push(format!("{indent}\t}}"));
                out.push(format!("{indent}}} else {{"));
                let rows = self.fresh("__rows");
                let err = self.fresh("__err");
                out.push(format!(
                    "{indent}\t{rows}, {err} := {handle}.QueryContext(ctx, {sql:?}, {})",
                    binds.join(", ")
                ));
                out.push(format!("{indent}\tif {err} != nil {{"));
                out.push(format!("{indent}\t\treturn {}, {err}", self.zero_return));
                out.push(format!("{indent}\t}}"));
                out.push(format!("{indent}\tdefer {rows}.Close()"));
                out.push(format!("{indent}\t{out_var} = []{record_name}{{}}"));
                out.push(format!("{indent}\tfor {rows}.Next() {{"));
                let elem = self.fresh("__elem");
                out.push(format!("{indent}\t\tvar {elem} {record_name}"));
                let scan_err = self.fresh("__err");
                out.push(format!(
                    "{indent}\t\tif {scan_err} := {rows}.Scan({}); {scan_err} != nil {{",
                    self.scan_targets(&record, &elem)
                ));
                out.push(format!(
                    "{indent}\t\t\treturn {}, {scan_err}",
                    self.zero_return
                ));
                out.push(format!("{indent}\t\t}}"));
                out.push(format!("{indent}\t\t{out_var} = append({out_var}, {elem})"));
                out.push(format!("{indent}\t}}"));
                let rows_err = self.fresh("__err");
                out.push(format!(
                    "{indent}\tif {rows_err} := {rows}.Err(); {rows_err} != nil {{"
                ));
                out.push(format!(
                    "{indent}\t\treturn {}, {rows_err}",
                    self.zero_return
                ));
                out.push(format!("{indent}\t}}"));
                out.push(format!("{indent}}}"));
                apply_dest(self, dest, &out_var, indent, &mut out);
                out
            }
            Verb::DbCount(table) => {
                let table_snake = self.ir.table(table).name.to_snake_case();
                let handle = self.handle(in_tx);
                let (where_sql, binds) = self.where_clause(predicate);
                let sql = sqlph(
                    &format!("SELECT COUNT(*) FROM {table_snake}{where_sql}"),
                    self.db_engine,
                );
                let world_pred = self.world_predicate_expr(predicate);
                let count = self.fresh("__count");
                let mut out = vec![format!("{indent}var {count} int64")];
                out.push(format!("{indent}if st.World != nil {{"));
                let pred_var = self.fresh("__pred");
                out.push(format!(
                    "{indent}\t{pred_var} := func(row world.Row) bool {{ return {world_pred} }}"
                ));
                out.push(format!(
                    "{indent}\t{count} = st.World.DBCount({table_snake:?}, {pred_var})"
                ));
                out.push(format!("{indent}}} else {{"));
                let row = self.fresh("__row");
                let err = self.fresh("__err");
                out.push(format!(
                    "{indent}\t{row} := {handle}.QueryRowContext(ctx, {sql:?}, {})",
                    binds.join(", ")
                ));
                out.push(format!(
                    "{indent}\tif {err} := {row}.Scan(&{count}); {err} != nil {{"
                ));
                out.push(format!("{indent}\t\treturn {}, {err}", self.zero_return));
                out.push(format!("{indent}\t}}"));
                out.push(format!("{indent}}}"));
                apply_dest(self, dest, &count, indent, &mut out);
                out
            }
            Verb::DbDeleteWhere(table) => {
                let table_snake = self.ir.table(table).name.to_snake_case();
                let handle = self.handle(in_tx);
                let (where_sql, binds) = self.where_clause(predicate);
                let sql = sqlph(
                    &format!("DELETE FROM {table_snake}{where_sql}"),
                    self.db_engine,
                );
                let world_pred = self.world_predicate_expr(predicate);
                // `World` has no bulk delete-by-predicate method
                // (`CommitBatchChecked` takes explicit `(table, pk)`
                // pairs, not a filter), so the world branch resolves
                // matching ids first, then deletes each through
                // `DBDeleteChecked` (real cascade/restrict handling,
                // real failure-injection) -- a disclosed divergence
                // from production's single bulk `DELETE`, mirroring
                // Rust's/TypeScript's own `query_tail` world branch
                // exactly.
                let n = self.fresh("__n");
                let mut out = vec![format!("{indent}var {n} int64")];
                out.push(format!("{indent}if st.World != nil {{"));
                let pred_var = self.fresh("__pred");
                out.push(format!(
                    "{indent}\t{pred_var} := func(row world.Row) bool {{ return {world_pred} }}"
                ));
                let ids = self.fresh("__ids");
                out.push(format!(
                    "{indent}\t{ids} := st.World.DBMatchingIDs({table_snake:?}, {pred_var})"
                ));
                let id = self.fresh("__id");
                out.push(format!("{indent}\tfor _, {id} := range {ids} {{"));
                let ok = self.fresh("__ok");
                let werr = self.fresh("__err");
                out.push(format!(
                    "{indent}\t\t{ok}, {werr} := st.World.DBDeleteChecked({table_snake:?}, {id})"
                ));
                out.push(format!("{indent}\t\tif {werr} != nil {{"));
                out.push(format!("{indent}\t\t\treturn {}, {werr}", self.zero_return));
                out.push(format!("{indent}\t\t}}"));
                out.push(format!("{indent}\t\tif {ok} {{"));
                out.push(format!("{indent}\t\t\t{n}++"));
                out.push(format!("{indent}\t\t}}"));
                out.push(format!("{indent}\t}}"));
                out.push(format!("{indent}}} else {{"));
                let result = self.fresh("__result");
                let err = self.fresh("__err");
                out.push(format!(
                    "{indent}\t{result}, {err} := {handle}.ExecContext(ctx, {sql:?}, {})",
                    binds.join(", ")
                ));
                out.push(format!("{indent}\tif {err} != nil {{"));
                out.push(format!("{indent}\t\treturn {}, {err}", self.zero_return));
                out.push(format!("{indent}\t}}"));
                let nerr = self.fresh("__err");
                let nval = self.fresh("__nval");
                out.push(format!(
                    "{indent}\t{nval}, {nerr} := {result}.RowsAffected()"
                ));
                out.push(format!("{indent}\tif {nerr} != nil {{"));
                out.push(format!("{indent}\t\treturn {}, {nerr}", self.zero_return));
                out.push(format!("{indent}\t}}"));
                out.push(format!("{indent}\t{n} = {nval}"));
                out.push(format!("{indent}}}"));
                apply_dest(self, dest, &n, indent, &mut out);
                out
            }
            _ => unreachable!("HirExpr::Query only ever carries a db query verb, found {verb:?}"),
        }
    }
    /// `Dest::Assign` at a *branching* local (see
    /// [`collect_branching_lets`]) reassigns the hoisted `var` via
    /// plain `=`; every other assignment introduces the name fresh via
    /// `:=`.
    fn assign(&self, name: &str, value: &str, indent: &str) -> String {
        if self.branching_locals.contains_key(name) {
            format!("{indent}{name} = {value}")
        } else {
            format!("{indent}{name} := {value}")
        }
    }
    /// `_ = value`: Go has no bare-expression-statement form for a
    /// call already bound to `(v, err)` two lines up (see
    /// `fallible_tail`) — the value is already a plain identifier by
    /// the time `discard_stmt` sees it, so blank-assigning it is both
    /// correct and marks it used (avoiding "declared and not used").
    fn discard_stmt(&self, value: &str, indent: &str) -> String {
        format!("{indent}_ = {value}")
    }
    fn empty_block_stmt(&self, _indent: &str) -> Vec<String> {
        // An empty `{}` block is valid Go on its own.
        Vec::new()
    }
    /// REAL atomicity (Pillar 4), matching the Rust backend's own
    /// (`26UpdatePlan.md` M1): `database/sql`'s `*sql.Tx` (from
    /// `BeginTx`) is a real, single checked-out connection running
    /// `BEGIN`/`COMMIT`/`ROLLBACK` under the hood — no manual SQL text
    /// needed, unlike TS's own Postgres/MySQL branch (Go's stdlib
    /// already gives every engine, SQLite included, the same
    /// `*sql.Tx` shape, so there's no per-engine branch here at all,
    /// a real simplification over TS's three-way split). `defer
    /// {TX_HANDLE}.Rollback()` immediately after a successful `Commit`
    /// is a safe, idiomatic no-op (`sql.ErrTxDone`, silently ignored)
    /// — this is the ordinary Go transaction pattern, not a hand-
    /// rolled rollback-on-panic scheme.
    ///
    /// 27UpdatePlan.md M7's world guard: under simulation (`st.World`
    /// set), `World`'s own ambient batch mode (`BeginWorldBatch`/
    /// `CommitWorldBatch`/`RollbackWorldBatch`) stands in for the real
    /// `BeginTx`/`Commit`/`Rollback` sequence — every world-guarded db
    /// verb inside `inner_lines` (`db.insert`/`update`/`delete`, now
    /// all of them, not just `db.insert` as the v0.24 M9 narrow world
    /// allowed) queues into `World`'s own pending batch instead of
    /// applying immediately once `BeginWorldBatch` is active, so the
    /// same real, atomic all-or-nothing guarantee production's `*sql.
    /// Tx` gives applies under simulation too. `defer
    /// st.World.RollbackWorldBatch()` unconditionally after
    /// `BeginWorldBatch`, mirroring the real-`*sql.Tx` branch's own
    /// `defer .. Rollback()` idiom exactly: `RollbackWorldBatch` after
    /// a successful `CommitWorldBatch` is a safe no-op (the pending
    /// batch is already `nil`), the identical "defer rollback, commit
    /// clears it" pattern `database/sql`'s own `sql.ErrTxDone` gives
    /// the production path for free. `{TX_HANDLE}` is still declared
    /// unconditionally (typed `nil`, never dereferenced under
    /// simulation) — Go requires the identifier to exist even on a
    /// path that never runs.
    fn transaction_stmt(&self, inner_lines: Vec<String>, indent: &str) -> Vec<String> {
        let field = self
            .db_field
            .as_deref()
            .expect("a transaction block requires a bound database instance");
        let err = self.fresh("__err");
        let mut out = vec![format!("{indent}var {TX_HANDLE} *sql.Tx")];
        out.push(format!("{indent}if st.World == nil {{"));
        out.push(format!("{indent}\tvar {err} error"));
        out.push(format!(
            "{indent}\t{TX_HANDLE}, {err} = st.{}.BeginTx(ctx, nil)",
            go_pascal(field.to_owned())
        ));
        out.push(format!("{indent}\tif {err} != nil {{"));
        out.push(format!("{indent}\t\treturn {}, {err}", self.zero_return));
        out.push(format!("{indent}\t}}"));
        out.push(format!(
            "{indent}\tdefer func() {{ _ = {TX_HANDLE}.Rollback() }}()"
        ));
        out.push(format!("{indent}}} else {{"));
        out.push(format!("{indent}\tst.World.BeginWorldBatch()"));
        out.push(format!(
            "{indent}\tdefer func() {{ st.World.RollbackWorldBatch() }}()"
        ));
        out.push(format!("{indent}}}"));
        out.extend(inner_lines);
        let cerr = self.fresh("__err");
        out.push(format!("{indent}if st.World == nil {{"));
        out.push(format!(
            "{indent}\tif {cerr} := {TX_HANDLE}.Commit(); {cerr} != nil {{"
        ));
        out.push(format!("{indent}\t\treturn {}, {cerr}", self.zero_return));
        out.push(format!("{indent}\t}}"));
        out.push(format!("{indent}}} else {{"));
        out.push(format!(
            "{indent}\tif {cerr} := st.World.CommitWorldBatch(); {cerr} != nil {{"
        ));
        out.push(format!("{indent}\t\treturn {}, {cerr}", self.zero_return));
        out.push(format!("{indent}\t}}"));
        out.push(format!("{indent}}}"));
        out
    }

    fn return_stmt(&self, value: Option<&str>, indent: &str) -> String {
        match value {
            Some(v) => format!("{indent}return {v}, nil"),
            None => format!("{indent}return {}, nil", self.zero_return),
        }
    }
    fn fail(&self, error: RecordId, args: &[String], indent: &str) -> String {
        let name = record_class_name(self.ir, error);
        let record = self.ir.record(error);
        let field_inits: Vec<String> = record
            .fields
            .iter()
            .zip(args)
            .map(|(f, a)| format!("{}: {a}", go_pascal(f.name.clone())))
            .collect();
        format!(
            "{indent}return {}, &{name}{{{}}}",
            self.zero_return,
            field_inits.join(", ")
        )
    }
    fn publish(&self, subject: &str, value: &str, _value_ty: &HirType, indent: &str) -> String {
        let err = self.fresh("__err");
        format!(
            "{indent}if {err} := queue.PublishJSON(ctx, st.Queue, st.World, {subject:?}, {value}); {err} != nil {{ return {}, {err} }}",
            self.zero_return
        )
    }
    fn db_get(&self, table: TableId, key: &str) -> String {
        // Unreachable via `lower_scalar` (the error-idiom amendment
        // routes every `db.get` through `db_get_tail` instead — see
        // that method and the module doc); kept correct in case a
        // future `HostSyntax` consumer calls it directly the way
        // `db_get_tail`'s own default implementation would.
        //
        // 27UpdatePlan.md M7: an early-return world guard inside the
        // same closure, ahead of the real `QueryRowContext` path —
        // this preserves every other engine's own async/sync wrapper
        // shape unchanged (the same pattern Rust's own `db_get`
        // reached at M4).
        let table_snake = self.ir.table(table).name.to_snake_case();
        let record = context::build_record(self.ir, self.ir.table(table).record);
        let record_name = record_class_name(self.ir, self.ir.table(table).record);
        let handle = format!(
            "st.{}",
            go_pascal(
                self.db_field
                    .clone()
                    .expect("db.get requires a bound database instance")
            )
        );
        let sql = sqlph(
            &format!(
                "SELECT {} FROM {table_snake} WHERE id = $1",
                record.select_cols
            ),
            self.db_engine,
        );
        format!(
            "func() (*{record_name}, error) {{ if st.World != nil {{ var __wrow {record_name}; __ok, __werr := st.World.DBGet({table_snake:?}, {key}, &__wrow); if __werr != nil {{ return nil, __werr }}; if !__ok {{ return nil, nil }}; return &__wrow, nil }}; var __row {record_name}; if err := {handle}.QueryRowContext(ctx, {sql:?}, {key}).Scan({}); err != nil {{ if err == sql.ErrNoRows {{ return nil, nil }}; return nil, err }}; return &__row, nil }}()",
            self.scan_targets(&record, "__row")
        )
    }
    /// Overridden (the error-idiom amendment's whole point): a fresh
    /// `func() (*T, error) {{ .. }}()` closure so `db_get_tail`'s
    /// `fallible_tail` call sees a genuine `(value, error)` pair to
    /// destructure, rather than trying to retrofit error handling onto
    /// a bare value expression.
    fn db_get_tail(&self, table: TableId, key: &str, dest: &Dest, indent: &str) -> Vec<String> {
        self.fallible_tail(&self.db_get(table, key), dest, indent)
    }
    fn cache_get(&self, key: &str) -> String {
        let cache = self.cache_handle();
        let instance = self.cache_instance();
        format!(
            "func() (any, error) {{ if st.World != nil {{ __wv, __wok := st.World.CacheGet({instance:?}, {key}); if !__wok {{ return nil, nil }}; var __wout any; if err := json.Unmarshal([]byte(__wv), &__wout); err != nil {{ return nil, err }}; return __wout, nil }}; __cv, err := {cache}.Get(ctx, {key}).Result(); if err == redis.Nil {{ return nil, nil }}; if err != nil {{ return nil, err }}; var __out any; if err := json.Unmarshal([]byte(__cv), &__out); err != nil {{ return nil, err }}; return __out, nil }}()"
        )
    }
    fn cache_get_tail(&self, key: &str, dest: &Dest, indent: &str) -> Vec<String> {
        self.fallible_tail(&self.cache_get(key), dest, indent)
    }
    /// `cache.set` carries no TTL argument at the CIaC language level
    /// today (production always passes `0`, "never expire", to
    /// go-redis's own `Set`) — the world branch matches that by
    /// always passing a `nil` TTL to `World.CacheSet`, not a gap this
    /// milestone introduces.
    fn cache_set(&self, key: &str, value: &str, _value_ty: &HirType) -> String {
        let cache = self.cache_handle();
        let instance = self.cache_instance();
        format!(
            "func() (any, error) {{ __b, err := json.Marshal({value}); if err != nil {{ return nil, err }}; if st.World != nil {{ st.World.CacheSet({instance:?}, {key}, string(__b), nil); return nil, nil }}; return {cache}.Set(ctx, {key}, __b, 0).Err(), nil }}()"
        )
    }
    fn cache_set_tail(
        &self,
        key: &str,
        value: &str,
        value_ty: &HirType,
        dest: &Dest,
        indent: &str,
    ) -> Vec<String> {
        self.fallible_tail(&self.cache_set(key, value, value_ty), dest, indent)
    }
    fn cache_delete(&self, key: &str) -> String {
        let cache = self.cache_handle();
        let instance = self.cache_instance();
        format!(
            "func() (any, error) {{ if st.World != nil {{ st.World.CacheDelete({instance:?}, {key}); return nil, nil }}; return nil, {cache}.Del(ctx, {key}).Err() }}()"
        )
    }
    fn cache_delete_tail(&self, key: &str, dest: &Dest, indent: &str) -> Vec<String> {
        self.fallible_tail(&self.cache_delete(key), dest, indent)
    }
    // --- object store / email / search / http: each leaf reaches the
    // handler's bound instance through `st.<state_field>` in
    // production (resolved once in `render()` exactly like
    // `db_field`/`cache_field`), or `World`'s own per-instance map,
    // keyed by the declared instance name, under simulation.
    fn object_store_put(&self, key: &str, value: &str, value_ty: &HirType) -> String {
        let field = self.object_store_state_field();
        let instance = self.object_store_instance();
        // Reuses `json_body` (the same Record/Json/scalar 3-way
        // `search.index`/`http.call` already use) rather than its own
        // bespoke match: an earlier version of this match built a
        // `func() ([]byte, error) { return json.Marshal(value) }()`
        // sub-expression for the `Record` arm and passed it as `Put`'s
        // *third argument* -- a genuine `go vet`/`go build` failure
        // ("multiple-value ... in single-value context"), since a
        // multi-return call may only be used as a bare argument when
        // it is the *sole* argument, not one of three. `Put` now takes
        // a single `any` payload and marshals it itself, so every
        // branch here is single-valued.
        let payload = self.json_body(value, value_ty);
        format!(
            "func() (struct{{}}, error) {{ if st.World != nil {{ __wb, __werr := json.Marshal({payload}); if __werr != nil {{ return struct{{}}{{}}, __werr }}; st.World.ObjectPut({instance:?}, {key}, __wb); return struct{{}}{{}}, nil }}; return st.{field}.Put(ctx, {key}, {payload}) }}()"
        )
    }
    fn object_store_put_tail(
        &self,
        key: &str,
        value: &str,
        value_ty: &HirType,
        dest: &Dest,
        indent: &str,
    ) -> Vec<String> {
        self.fallible_tail(&self.object_store_put(key, value, value_ty), dest, indent)
    }
    fn object_store_get(&self, key: &str) -> String {
        let field = self.object_store_state_field();
        let instance = self.object_store_instance();
        format!(
            "func() (any, error) {{ var __b []byte; var __err error; if st.World != nil {{ __b, __err = st.World.ObjectGet({instance:?}, {key}) }} else {{ __b, __err = st.{field}.Get(ctx, {key}) }}; if __err != nil {{ return nil, __err }}; var __out any; if err := json.Unmarshal(__b, &__out); err != nil {{ return nil, err }}; return __out, nil }}()"
        )
    }
    fn object_store_get_tail(&self, key: &str, dest: &Dest, indent: &str) -> Vec<String> {
        self.fallible_tail(&self.object_store_get(key), dest, indent)
    }
    fn object_store_delete(&self, key: &str) -> String {
        let field = self.object_store_state_field();
        let instance = self.object_store_instance();
        format!(
            "func() (struct{{}}, error) {{ if st.World != nil {{ st.World.ObjectDelete({instance:?}, {key}); return struct{{}}{{}}, nil }}; return st.{field}.Delete(ctx, {key}) }}()"
        )
    }
    fn object_store_delete_tail(&self, key: &str, dest: &Dest, indent: &str) -> Vec<String> {
        self.fallible_tail(&self.object_store_delete(key), dest, indent)
    }
    fn object_store_list(&self, prefix: &str) -> String {
        let field = self.object_store_state_field();
        let instance = self.object_store_instance();
        format!(
            "func() ([]string, error) {{ if st.World != nil {{ return st.World.ObjectList({instance:?}, {prefix}), nil }}; return st.{field}.List(ctx, {prefix}) }}()"
        )
    }
    fn object_store_list_tail(&self, prefix: &str, dest: &Dest, indent: &str) -> Vec<String> {
        self.fallible_tail(&self.object_store_list(prefix), dest, indent)
    }
    fn email_send(&self, to: &str, subject: &str, body: &str) -> String {
        let field = go_pascal(
            self.email_field
                .clone()
                .expect("an email verb requires a bound email instance"),
        );
        let instance = self.email_instance();
        format!(
            "func() (struct{{}}, error) {{ if st.World != nil {{ st.World.EmailSend({instance:?}, {to}, {subject}, {body}); return struct{{}}{{}}, nil }}; return st.{field}.Send(ctx, {to}, {subject}, {body}) }}()"
        )
    }
    fn email_send_tail(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        dest: &Dest,
        indent: &str,
    ) -> Vec<String> {
        self.fallible_tail(&self.email_send(to, subject, body), dest, indent)
    }
    fn search_index(&self, doc_id: &str, document: &str, document_ty: &HirType) -> String {
        let field = go_pascal(
            self.search_field
                .clone()
                .expect("a search verb requires a bound search instance"),
        );
        let instance = self.search_instance();
        let document = self.json_body(document, document_ty);
        format!(
            "func() (struct{{}}, error) {{ if st.World != nil {{ st.World.SearchIndex({instance:?}, {doc_id}, {document}); return struct{{}}{{}}, nil }}; return st.{field}.Index(ctx, {SEARCH_INDEX_NAME:?}, {doc_id}, {document}) }}()"
        )
    }
    fn search_index_tail(
        &self,
        doc_id: &str,
        document: &str,
        document_ty: &HirType,
        dest: &Dest,
        indent: &str,
    ) -> Vec<String> {
        self.fallible_tail(
            &self.search_index(doc_id, document, document_ty),
            dest,
            indent,
        )
    }
    fn search_query(&self, query: &str) -> String {
        let field = go_pascal(
            self.search_field
                .clone()
                .expect("a search verb requires a bound search instance"),
        );
        let instance = self.search_instance();
        format!(
            "func() ([]json.RawMessage, error) {{ if st.World != nil {{ __hits := st.World.SearchQuery({instance:?}, {query}); __out := make([]json.RawMessage, 0, len(__hits)); for _, __h := range __hits {{ __b, __err := json.Marshal(__h); if __err != nil {{ return nil, __err }}; __out = append(__out, __b) }}; return __out, nil }}; return st.{field}.Search(ctx, {SEARCH_INDEX_NAME:?}, {query}) }}()"
        )
    }
    fn search_query_tail(&self, query: &str, dest: &Dest, indent: &str) -> Vec<String> {
        self.fallible_tail(&self.search_query(query), dest, indent)
    }
    fn http_call(&self, url: &str, json_body: &str, body_ty: &HirType) -> String {
        let field = go_pascal(
            self.http_field
                .clone()
                .expect("an external_http verb requires a bound external_http instance"),
        );
        let instance = self.http_instance();
        let json_val = self.json_body(json_body, body_ty);
        format!(
            "func() (json.RawMessage, error) {{ if st.World != nil {{ __wv, __werr := st.World.HTTPPost({instance:?}, {url}, {json_val}); if __werr != nil {{ return nil, __werr }}; return json.Marshal(__wv) }}; return st.{field}.Post(ctx, {url}, {json_val}) }}()"
        )
    }
    fn http_call_tail(
        &self,
        url: &str,
        json_body: &str,
        body_ty: &HirType,
        dest: &Dest,
        indent: &str,
    ) -> Vec<String> {
        self.fallible_tail(&self.http_call(url, json_body, body_ty), dest, indent)
    }
}

#[derive(Debug, Serialize)]
pub struct ParamCtx {
    pub name: String,
    pub go_type: String,
}

/// One hoisted `var name GoType` [`collect_branching_lets`] found.
#[derive(Debug, Serialize)]
pub struct HoistedLocalCtx {
    pub name: String,
    pub go_type: String,
}

/// Everything `logic.go.j2` needs to render one typed handler's file —
/// inline (compiler-owned, `internal/logic/<module>.go`) or `extern`
/// (seeded, `internal/services/<module>.go`; `body` is just a
/// stand-in `panic`). The `needs_*` package flags are computed
/// structurally from `ciac_codegen::lower::scan`'s own `Needs` where a
/// precise flag already exists there (`schemas`/`time`/`uuid`/`json`/
/// `database/sql`, each tied to the exact leaf that spells the package
/// name — see `render`'s own comment at each field), rather than
/// re-deriving them by re-walking the HIR a second time: Go's
/// `gofmt` reformats but never drops an unused import the way
/// `goimports` would, so an imprecise flag here is a real "imported
/// and not used" compile failure, not just untidy output.
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
    /// Engine of the bound db instance (v0.13 M1's discipline):
    /// selects placeholder style in generated SQL. `postgres` when no
    /// db is bound.
    pub db_engine: String,
    pub extras: Vec<context::ExtraDepCtx>,
    /// Every `(name, GoType)` this handler's body needs hoisted above
    /// a branching `if`/`match` (see [`collect_branching_lets`]).
    pub hoisted_locals: Vec<HoistedLocalCtx>,
    pub body: Vec<String>,
    /// `internal/schemas` — any record or enum type is referenced
    /// (params, return, hoisted locals, or the body: `RecordCons`/
    /// `Fail`/every field-typed value `needs.records`/`needs.enums`
    /// already tracks).
    pub needs_schemas: bool,
    /// `time` — a `Timestamp`-typed value appears anywhere in the
    /// signature/locals (`time.Time`), or `Timestamp.now()` is called
    /// (`needs.datetime`; the two are independent, unlike Rust where
    /// the type import and the call are the same `chrono` symbol).
    pub needs_time: bool,
    /// `github.com/google/uuid` — `Uuid.new()` is called.
    pub needs_uuid_pkg: bool,
    /// `encoding/json` — `cache.get`/`cache.set`/`object_store.get`
    /// (`needs.json`) or `object_store.put` (unexercised this
    /// milestone, not covered by `needs.json` itself — see
    /// `render`'s own comment).
    pub needs_json: bool,
    /// `database/sql` — only `db.get`'s own closure spells
    /// `sql.ErrNoRows` by name (`needs.db_get`); every other db verb
    /// reaches its handle through a plain `st.Field`/`__tx` value with
    /// no package-qualified type spelled out.
    pub needs_sql_pkg: bool,
    /// `fmt` — the `Json`-indexing leaf's `fmt.Sprintf` panic message,
    /// or `object_store.put`'s non-record `fmt.Sprint` fallback.
    /// Neither has a `Needs` flag (indexing/put payload shape aren't
    /// scanned), so this one flag is a textual scan of the rendered
    /// body — safe because it's a substring unique to those two call
    /// sites' own literal source text, not a heuristic guess.
    pub needs_fmt: bool,
    /// `github.com/redis/go-redis/v9` — `cache.*`. Unreachable this
    /// milestone (`Component::Cache` stays `CIAC0011`-refused until
    /// M6/M7 — see the module doc), kept structurally correct the same
    /// way `needs_fmt` is.
    pub needs_redis: bool,
    /// 27UpdatePlan.md M7: `internal/world` — the world-guarded
    /// `db.query`/`count`/`delete_where` branches spell `world.Row`/
    /// `world.JSONEq`/... directly in their own predicate closure. No
    /// dedicated `Needs` flag exists for this (predicate compilation
    /// isn't scanned), so this is a textual fallback the same way
    /// `needs_fmt`/`needs_redis` already are.
    pub needs_world: bool,
}

/// Builds the render context for one typed handler node. `name` is the
/// handler's declared name (`node.component.name()`).
pub fn render(ir: &NormalizedIr, name: &str, hir: &HandlerBody) -> LogicFileCtx {
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
    // 27UpdatePlan.md M7: the *declared* instance name (distinct from
    // the `*_field` values above, the Go `AppState` field) — `World`'s
    // own per-instance maps are keyed by this, mirroring TypeScript's
    // own `instance_of` closure exactly.
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

    let params = hir
        .params
        .iter()
        .map(|(n, ty)| ParamCtx {
            name: n.clone(),
            go_type: go_type(ir, ty),
        })
        .collect();

    let db_engine = db_engine_of(ir, hir);
    let mut branching = Vec::new();
    let body = match &hir.body {
        Some(stmts) => {
            collect_branching_lets(hir, stmts, &mut branching);
            let syntax = GoSyntax {
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
                zero_return: go_zero(ir, &hir.return_ty),
                tmp: std::cell::Cell::new(0),
                branching_locals: branching.iter().cloned().collect(),
            };
            lower::lower_body_stmt(&syntax, ir, hir, "\t")
        }
        None => vec![format!("\tpanic(\"not implemented\")")],
    };

    let needs_schemas = !needs.records.is_empty() || !needs.enums.is_empty();
    let needs_time = needs.datetime
        || ty_mentions_timestamp(&hir.return_ty)
        || hir.params.iter().any(|(_, t)| ty_mentions_timestamp(t))
        || branching.iter().any(|(_, t)| ty_mentions_timestamp(t));
    let body_text = body.join("\n");

    LogicFileCtx {
        class_name: name.to_owned(),
        module: name.to_snake_case(),
        is_extern: hir.body.is_none(),
        params,
        return_type: go_type(ir, &hir.return_ty),
        needs_db: access.db.is_some(),
        needs_cache: access.cache_expr.is_some(),
        needs_queue: needs.queue,
        rust_db_field: access.rust_db_field,
        rust_cache_field: access.rust_cache_field,
        db_engine: db_engine.to_owned(),
        extras,
        hoisted_locals: branching
            .into_iter()
            .map(|(name, ty)| HoistedLocalCtx {
                name,
                go_type: go_type(ir, &ty),
            })
            .collect(),
        needs_schemas,
        needs_time,
        needs_uuid_pkg: needs.uuid,
        // `needs.json` covers every fallible leaf that spells `json.`
        // directly in its own rendered text (cache); `object_store.put`
        // no longer does (M7: it reuses `json_body`, the same single-
        // valued Record/Json/scalar branch `search.index`/`http.call`
        // already use, none of which spell `json.` themselves — the
        // marshaling now happens inside `ObjectStore.Put` itself, not
        // the caller's body) — an earlier version of this flag OR'd in
        // `needs.object_store_put` unconditionally, which forced an
        // `encoding/json` import even on a body with no `json.` text
        // at all (a scalar `object_store.put` call), an "imported and
        // not used" failure caught live. `Json` indexing (`HostSyntax
        // ::index`) isn't itself tracked by the shared scanner (no
        // dedicated `Needs` flag exists for it — see `render`'s own
        // `needs_fmt`/`needs_redis` for the same situation), so the
        // textual fallback below still catches that case (and cache's).
        needs_json: needs.json || body_text.contains("json."),
        // `needs.db_get` covers `sql.ErrNoRows`; a `transaction {}`
        // block's own world-guarded `var __tx *sql.Tx` (v0.24 M9) is a
        // second, independent reason this file needs `database/sql` --
        // the shared `Needs` scanner has no dedicated transaction flag
        // (see `needs_fmt`/`needs_redis`'s own textual-fallback
        // pattern just below for why this file leans on `body_text`
        // for cases the scanner doesn't track).
        needs_sql_pkg: needs.db_get || body_text.contains("*sql.Tx"),
        needs_fmt: body_text.contains("fmt."),
        needs_redis: body_text.contains("redis."),
        needs_world: body_text.contains("world."),
        body,
    }
}

/// Whether `ty` (recursing through `Option`/`List`) is `Timestamp` —
/// the one case the shared `Needs` scanner's own type walk doesn't
/// track (it only follows `Record`), needed here for `needs_time`'s
/// own signature-position half (see `LogicFileCtx::needs_time`'s doc).
fn ty_mentions_timestamp(ty: &HirType) -> bool {
    match ty {
        HirType::Timestamp => true,
        HirType::Option(inner) | HirType::List(inner) => ty_mentions_timestamp(inner),
        _ => false,
    }
}

/// Engine of the handler's bound db instance, straight from the IR
/// node its binding edge points at — `postgres` when unbound, matching
/// every other backend's own `db_engine_of` exactly (v0.13 M1's
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
