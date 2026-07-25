//! The `HostSyntax` contract (`22UpdatePlan.md` Pillar 3, Parts 2-3 —
//! the deferred half of the backend factory: a shared statement/
//! expression dispatcher plus this leaf-lowering trait). Roughly 50
//! leaf constructor methods a target implements against the shared
//! walker in `dispatch.rs`; see that module's doc comment for exactly
//! what the walker owns and why the leaf surface is shaped the way it
//! is (context-carrying: leaves take stable IR ids like [`TableId`]/
//! [`RecordId`] plus already-lowered child strings, never raw
//! [`ciac_ir::HirExpr`] trees, and never precompute a per-language
//! "context struct" the shared crate would have to own).

use super::dispatch::{apply_dest, Dest, Wrap};
use ciac_ir::{BinOp, HirExpr, HirType, NodeId, PredOp, RecordId, TableId, UnOp, Verb};

/// Whether a target's control flow and statement-shaped verbs
/// (`if`/`match`/`db.insert`/`db.update`/`db.delete`/`db.query`/
/// `db.count`/`db.delete_where`) can be lowered as plain nested
/// values, or must decompose into a statement sequence ending in an
/// assignment/return/discard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Rust today. `if`/`match`/every db verb are real expressions; a
    /// block is `{ stmts; tail }`.
    Expression,
    /// Python today; Go and Java per `24UpdatePlan.md`'s/
    /// `25UpdatePlan.md`'s own stated intent ("Go runs in the
    /// StatementOriented mode Python already exercises"; "Java runs
    /// StatementOriented... third consumer of the mode Python
    /// validated and Go re-validated").
    Statement,
}

/// A structural fact the dispatcher computes so a leaf never has to
/// pattern-match a raw `HirExpr` itself: was an index key a literal
/// string (Rust needs the raw text, not a generically-rendered owned
/// `String`, to index `serde_json::Value` idiomatically — see
/// `HostSyntax::index`'s own doc), or a general already-lowered
/// expression?
#[derive(Debug, Clone)]
pub enum IndexKey<'a> {
    StrKey(&'a str),
    Expr(String),
}

/// One `match`/`if`-arm's rendered pattern label and already-lowered
/// body, handed to [`HostSyntax::match_expr`].
#[derive(Debug, Clone)]
pub struct MatchArm {
    /// `None` is the wildcard (`_`) arm.
    pub variant: Option<String>,
    pub body: String,
}

/// A `db.query`/`db.count`/`db.delete_where` predicate with every
/// term's *value* already lowered — the chain-vs-raw-SQL formatting
/// stays entirely a leaf decision (Python's SQLAlchemy `.where(..)`
/// chain and Rust's ` WHERE field = $N` text are structurally
/// different artifacts, not just a spelling difference); only each
/// term's value expression is genuinely neutral, so only that gets
/// pre-lowered by the shared dispatcher.
#[derive(Debug, Clone)]
pub struct LoweredPredicate {
    pub terms: Vec<LoweredPredTerm>,
}

#[derive(Debug, Clone)]
pub struct LoweredPredTerm {
    pub field: String,
    pub field_ty: HirType,
    pub op: PredOp,
    pub value: PredValue,
}

#[derive(Debug, Clone)]
pub enum PredValue {
    /// A fully-lowered generic expression string (via the shared
    /// scalar dispatch).
    Rendered(String),
    /// A bare enum-literal term value — the *raw* variant name. Both
    /// current backends bind this as a quoted string, not a named
    /// `Type::Variant` path (a predicate term binds a raw column
    /// value, not a typed enum constructor), so the leaf decides the
    /// exact spelling rather than the dispatcher.
    EnumVariant(String),
    /// A bare bool-literal term value — kept distinct from `Rendered`
    /// so Python's bare-column boolean-filter idiom
    /// (`.where(Model.x)` instead of `.where(Model.x == True)`,
    /// avoiding `ruff`'s E712) can pattern-match it; every other host
    /// may treat it exactly like `Rendered`.
    BoolLit(bool),
}

/// Roughly 50 leaf constructor methods completing `22UpdatePlan.md`
/// Pillar 3 Parts 2-3: the shared dispatcher (`dispatch.rs`) owns the
/// HIR walk — precedence, block/tail shaping, enum-literal use-site
/// recovery, float-literal fidelity, divergence truncation — and calls
/// exactly one of these per leaf-shaped HIR node. A concrete host
/// implements only the methods its `ORIENTATION` reaches; the other
/// mode's methods keep their `unimplemented!()` defaults and are
/// structurally unreachable — proven by the full example sweep in
/// both backends' own test suites, and by the
/// [`super::identity::IdentitySyntax`]/
/// [`super::identity::IdentitySyntaxStatement`] pair exercising both
/// modes against the same HIR corpus.
///
/// Every method receives already-lowered child strings, never raw
/// [`HirExpr`] — the one deliberate exception is
/// [`HostSyntax::value_for_record_field`]'s `original` parameter,
/// which exists purely so Rust's E0382 clone hook can pattern-match
/// what *produced* the string, not the string itself.
pub trait HostSyntax {
    const ORIENTATION: Orientation;

    // --- literals & access ---
    fn int_lit(&self, n: i64) -> String;
    /// Receives the raw `f64`; Rust's implementation calls the shared
    /// [`super::dispatch::fidelity_checked_float`] to preserve
    /// today's `1.0` spelling, Python deliberately keeps today's bare
    /// `format!("{f}")` — this refactor changes neither.
    fn float_lit(&self, f: f64) -> String;
    fn str_lit(&self, s: &str) -> String;
    fn bool_lit(&self, b: bool) -> String;
    /// Default: a bare local name is spelled identically everywhere
    /// today.
    fn local(&self, name: &str) -> String {
        name.to_owned()
    }
    fn field_access(&self, base: &str, field: &str) -> String;
    /// `key` distinguishes a literal string key from a general
    /// expression — Rust's existing `Index` lowering special-cases a
    /// string-literal key to avoid the generic string-literal
    /// rendering's `.to_owned()` suffix, since `serde_json::Value`'s
    /// index operator wants `&str`, not `String`.
    fn index(&self, base: &str, key: IndexKey<'_>) -> String;
    fn uuid_new(&self) -> String;
    fn timestamp_now(&self) -> String;

    /// A bare enum variant, resolved at its use site. `enum_name` is
    /// `Some` for record-field values and enum comparisons (Rust needs
    /// the named type there); `None` in every other position,
    /// including predicate-term values (both current backends bind
    /// those as a quoted string, never a named path) and generic
    /// recursion with no enclosing context. A host that structurally
    /// needs a name (Rust) panics on `None`; Python ignores
    /// `enum_name` entirely.
    fn enum_literal(&self, enum_name: Option<&str>, variant: &str) -> String;

    /// Called on every record-construction field value (and on
    /// `..base`) right after it is lowered, before `record_cons` sees
    /// it — this is where Rust's `FieldAccess`-clone (E0382)
    /// discipline lives; every other host's default is a documented
    /// no-op. `original` is the *unlowered* HIR so the hook can
    /// pattern-match its shape without re-deriving it from the
    /// rendered string.
    fn value_for_record_field(&self, rendered: String, original: &HirExpr) -> String {
        let _ = original;
        rendered
    }
    /// Owns the entire fresh-vs-functional-update branch — Python's
    /// two shapes (`Cls(f=v, ...)` vs.
    /// `base.model_copy(update={...})`) are not just a suffix
    /// difference the way Rust's (`Cls { f: v, .. }` vs.
    /// `Cls { f: v, ..base }`) is.
    fn record_cons(
        &self,
        record_name: &str,
        fields: &[(String, String)],
        base: Option<&str>,
    ) -> String;

    /// Receives the closed [`BinOp`] enum directly (not a pre-rendered
    /// operator string) plus both operands' [`HirType`] — the
    /// string-concat special case (mixed Str/non-Str operands) is a
    /// per-language idiom, decided inside the leaf.
    fn binary(&self, op: BinOp, lhs: &str, rhs: &str, lhs_ty: &HirType, rhs_ty: &HirType)
        -> String;
    fn unary(&self, op: UnOp, operand: &str) -> String;

    // --- Expression-oriented block leaves (Rust today) ---
    fn if_expr(&self, cond: &str, then_block: &str, else_block: &str) -> String {
        let _ = (cond, then_block, else_block);
        unimplemented!("if_expr is Expression-oriented only")
    }
    fn match_expr(&self, enum_name: &str, scrutinee: &str, arms: &[MatchArm]) -> String {
        let _ = (enum_name, scrutinee, arms);
        unimplemented!("match_expr is Expression-oriented only")
    }
    /// `in_tx` (`26UpdatePlan.md` M1): `true` when this verb is lowered
    /// inside a `transaction {}` block's real (non-simulated) branch —
    /// see [`HostSyntax::transaction_expr`]. A leaf that cares (Rust:
    /// picks `&mut *__tx` over the pool) reads it; every other
    /// Expression-oriented leaf ignores it exactly as before.
    fn db_insert_expr(&self, table: TableId, value: &str, in_tx: bool) -> String {
        let _ = (table, value, in_tx);
        unimplemented!("db_insert_expr is Expression-oriented only")
    }
    fn db_update_expr(&self, table: TableId, key: &str, value: &str, in_tx: bool) -> String {
        let _ = (table, key, value, in_tx);
        unimplemented!("db_update_expr is Expression-oriented only")
    }
    fn db_delete_expr(&self, table: TableId, key: &str, in_tx: bool) -> String {
        let _ = (table, key, in_tx);
        unimplemented!("db_delete_expr is Expression-oriented only")
    }
    fn query_expr(&self, verb: Verb, predicate: Option<&LoweredPredicate>, in_tx: bool) -> String {
        let _ = (verb, predicate, in_tx);
        unimplemented!("query_expr is Expression-oriented only")
    }
    fn let_binding(&self, name: &str, value: &str) -> String {
        let _ = (name, value);
        unimplemented!("let_binding is Expression-oriented only")
    }
    /// How a bare `Expr` statement's already-lowered value is shaped —
    /// every mid-block statement and an empty block's synthesized
    /// [`HostSyntax::unit_literal`] alike (`;`-terminated, discarded),
    /// or a genuine block tail (passed through bare, or `Ok(..)`-
    /// wrapped for the function body's own tail) — see [`Wrap`].
    fn wrap_tail(&self, value: &str, wrap: Wrap) -> String {
        let _ = (value, wrap);
        unimplemented!("wrap_tail is Expression-oriented only")
    }
    fn unit_literal(&self) -> String {
        unimplemented!("unit_literal is Expression-oriented only")
    }
    /// `26UpdatePlan.md` M1: the dispatcher lowers the transaction
    /// body *twice* — `world_branch` (`in_tx=false`, identical to how
    /// every verb inside it already renders outside a transaction,
    /// including its own `self.world` check) and `real_branch`
    /// (`in_tx=true`, each db verb executing against the held
    /// executor) — because whether a real `sqlx::Transaction` can be
    /// opened at all depends on whether `self.world` is `Some` (opening
    /// one under simulation would attempt a real, absent database
    /// connection), and that can only be decided once, by the host, not
    /// independently by each nested verb. A leaf that implements this
    /// wraps both branches in an outer `self.world`-presence check and
    /// binds the transaction handle only in the `real_branch` arm.
    fn transaction_expr(&self, world_branch: &str, real_branch: &str) -> String {
        let _ = (world_branch, real_branch);
        unimplemented!("transaction_expr is Expression-oriented only")
    }

    // --- Statement-oriented block leaves (Python today) ---
    fn if_tail(
        &self,
        cond: &str,
        then_lines: Vec<String>,
        else_lines: Vec<String>,
        indent: &str,
    ) -> Vec<String> {
        let _ = (cond, then_lines, else_lines, indent);
        unimplemented!("if_tail is Statement-oriented only")
    }
    fn match_tail(
        &self,
        scrutinee: &str,
        arms: &[(Option<String>, Vec<String>)],
        indent: &str,
    ) -> Vec<String> {
        let _ = (scrutinee, arms, indent);
        unimplemented!("match_tail is Statement-oriented only")
    }
    fn db_insert_tail(
        &self,
        table: TableId,
        value: &str,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let _ = (table, value, dest, indent, in_tx);
        unimplemented!("db_insert_tail is Statement-oriented only")
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
        let _ = (table, key, value, dest, indent, in_tx);
        unimplemented!("db_update_tail is Statement-oriented only")
    }
    fn db_delete_tail(
        &self,
        table: TableId,
        key: &str,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let _ = (table, key, dest, indent, in_tx);
        unimplemented!("db_delete_tail is Statement-oriented only")
    }
    fn query_tail(
        &self,
        verb: Verb,
        predicate: Option<&LoweredPredicate>,
        dest: &Dest,
        indent: &str,
        in_tx: bool,
    ) -> Vec<String> {
        let _ = (verb, predicate, dest, indent, in_tx);
        unimplemented!("query_tail is Statement-oriented only")
    }
    fn assign(&self, name: &str, value: &str, indent: &str) -> String {
        let _ = (name, value, indent);
        unimplemented!("assign is Statement-oriented only")
    }
    fn discard_stmt(&self, value: &str, indent: &str) -> String {
        let _ = (value, indent);
        unimplemented!("discard_stmt is Statement-oriented only")
    }
    /// A syntactically-empty statement list's rendering (Python:
    /// `pass`) — kept a real leaf rather than a shared assumption
    /// because "what does an empty block look like" is exactly as
    /// language-specific as any other literal.
    fn empty_block_stmt(&self, indent: &str) -> Vec<String> {
        let _ = indent;
        unimplemented!("empty_block_stmt is Statement-oriented only")
    }
    fn transaction_stmt(&self, inner_lines: Vec<String>, indent: &str) -> Vec<String> {
        let _ = (inner_lines, indent);
        unimplemented!("transaction_stmt is Statement-oriented only")
    }

    // --- statements shared by both orientations (each produces
    // exactly one already-indented line in both of today's backends,
    // so a single String-returning signature serves both modes) ---
    fn return_stmt(&self, value: Option<&str>, indent: &str) -> String;
    fn fail(&self, error: RecordId, args: &[String], indent: &str) -> String;
    fn publish(&self, subject: &str, value: &str, value_ty: &HirType, indent: &str) -> String;

    // --- db.get: a plain scalar value in both orientations today
    // (Python's ternary expression and Rust's block-expression are
    // both just values, never statement-decomposed) ---
    fn db_get(&self, table: TableId, key: &str) -> String;

    // --- cache / object store / email / search / http: every one of
    // these already fits a single expression in both backends today ---
    fn cache_get(&self, key: &str) -> String;
    fn cache_set(&self, key: &str, value: &str, value_ty: &HirType) -> String;
    fn cache_delete(&self, key: &str) -> String;
    fn object_store_put(&self, key: &str, value: &str, value_ty: &HirType) -> String;
    fn object_store_get(&self, key: &str) -> String;
    fn object_store_delete(&self, key: &str) -> String;
    fn object_store_list(&self, prefix: &str) -> String;
    fn email_send(&self, to: &str, subject: &str, body: &str) -> String;
    fn search_index(&self, doc_id: &str, document: &str, document_ty: &HirType) -> String;
    fn search_query(&self, query: &str) -> String;
    fn http_call(&self, url: &str, json_body: &str, body_ty: &HirType) -> String;

    // --- the error-idiom amendment (`24UpdatePlan.md` M4) ---
    //
    // Every leaf above this point that reaches `Statement` orientation
    // renders exactly one value, applied to its statement's `Dest` by
    // the shared `apply_dest`/`lower_tail` fallback — correct for
    // Python (exceptions) and TS (exceptions) alike, since neither
    // needs any extra syntax at the call site to propagate a failure.
    // Go has no such implicit propagation: a fallible call is a
    // multiple-return `(T, error)`, and using its result at all
    // requires its own `if err != nil { ... }` statement *before* the
    // value is usable — there is no expression-position error
    // operator the way Rust's `?` lets `db_insert_expr` stay a single
    // expression string even in `Expression` orientation.
    //
    // Each `..._tail` pair below gives a `Statement`-oriented host a
    // real decomposition point for one of the leaves above — `Dest`-
    // and `indent`-aware, exactly like `db_insert_tail`/`db_update_tail`
    // /`db_delete_tail`/`query_tail` already are. The **default**
    // implementation is not `unimplemented!()`: it is defined to
    // reproduce today's "compute the plain leaf value, then
    // `apply_dest` it" behavior byte-for-byte (call the existing
    // scalar leaf, then run the same three-way `Dest` dispatch
    // `apply_dest` already performs) — so Python, TS, and
    // `IdentitySyntaxStatement` need **zero** code changes and emit
    // **zero** output difference from before this amendment; only a
    // host that actually overrides one of these (Go) changes shape.
    // This is the amendment's own byte-identical proof, provable by
    // construction rather than only by snapshot diff — the snapshot
    // diff (or its absence) is the executable confirmation.
    fn db_get_tail(&self, table: TableId, key: &str, dest: &Dest, indent: &str) -> Vec<String> {
        let value = self.db_get(table, key);
        let mut out = Vec::new();
        apply_dest(self, dest, &value, indent, &mut out);
        out
    }
    fn cache_get_tail(&self, key: &str, dest: &Dest, indent: &str) -> Vec<String> {
        let value = self.cache_get(key);
        let mut out = Vec::new();
        apply_dest(self, dest, &value, indent, &mut out);
        out
    }
    fn cache_set_tail(
        &self,
        key: &str,
        value: &str,
        value_ty: &HirType,
        dest: &Dest,
        indent: &str,
    ) -> Vec<String> {
        let rendered = self.cache_set(key, value, value_ty);
        let mut out = Vec::new();
        apply_dest(self, dest, &rendered, indent, &mut out);
        out
    }
    fn cache_delete_tail(&self, key: &str, dest: &Dest, indent: &str) -> Vec<String> {
        let value = self.cache_delete(key);
        let mut out = Vec::new();
        apply_dest(self, dest, &value, indent, &mut out);
        out
    }
    fn object_store_put_tail(
        &self,
        key: &str,
        value: &str,
        value_ty: &HirType,
        dest: &Dest,
        indent: &str,
    ) -> Vec<String> {
        let rendered = self.object_store_put(key, value, value_ty);
        let mut out = Vec::new();
        apply_dest(self, dest, &rendered, indent, &mut out);
        out
    }
    fn object_store_get_tail(&self, key: &str, dest: &Dest, indent: &str) -> Vec<String> {
        let value = self.object_store_get(key);
        let mut out = Vec::new();
        apply_dest(self, dest, &value, indent, &mut out);
        out
    }
    fn object_store_delete_tail(&self, key: &str, dest: &Dest, indent: &str) -> Vec<String> {
        let value = self.object_store_delete(key);
        let mut out = Vec::new();
        apply_dest(self, dest, &value, indent, &mut out);
        out
    }
    fn object_store_list_tail(&self, prefix: &str, dest: &Dest, indent: &str) -> Vec<String> {
        let value = self.object_store_list(prefix);
        let mut out = Vec::new();
        apply_dest(self, dest, &value, indent, &mut out);
        out
    }
    fn email_send_tail(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        dest: &Dest,
        indent: &str,
    ) -> Vec<String> {
        let value = self.email_send(to, subject, body);
        let mut out = Vec::new();
        apply_dest(self, dest, &value, indent, &mut out);
        out
    }
    fn search_index_tail(
        &self,
        doc_id: &str,
        document: &str,
        document_ty: &HirType,
        dest: &Dest,
        indent: &str,
    ) -> Vec<String> {
        let rendered = self.search_index(doc_id, document, document_ty);
        let mut out = Vec::new();
        apply_dest(self, dest, &rendered, indent, &mut out);
        out
    }
    fn search_query_tail(&self, query: &str, dest: &Dest, indent: &str) -> Vec<String> {
        let value = self.search_query(query);
        let mut out = Vec::new();
        apply_dest(self, dest, &value, indent, &mut out);
        out
    }
    fn http_call_tail(
        &self,
        url: &str,
        json_body: &str,
        body_ty: &HirType,
        dest: &Dest,
        indent: &str,
    ) -> Vec<String> {
        let value = self.http_call(url, json_body, body_ty);
        let mut out = Vec::new();
        apply_dest(self, dest, &value, indent, &mut out);
        out
    }
}

/// Resolves a publish target's subject string — identical logic both
/// backends duplicated verbatim before this move; kept `pub(super)`
/// (not a leaf) because resolving a stream node's subject is
/// target-neutral, only *using* the resolved subject in a runtime
/// call is target-specific.
pub(super) fn stream_subject(ir: &ciac_ir::NormalizedIr, stream: NodeId) -> String {
    match &ir.node(stream).component {
        ciac_ir::Component::Stream { subject, .. } => subject.clone(),
        other => unreachable!("publish target is a stream, found {other:?}"),
    }
}
