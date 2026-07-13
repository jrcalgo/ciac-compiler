//! Typed high-level IR for handler bodies (v0.7 M2).
//!
//! `ciac-syntax`'s `Expr`/`Stmt` are pure surface syntax: no name
//! resolution, no types. `ciac-sema`'s type checker (`typeck.rs`) walks
//! that AST and lowers it into the types here — every name resolved to a
//! local slot, every verb resolved to a `(capability instance, operation,
//! table)` triple, every expression annotated with its [`HirType`].
//! Backends (once they exist for this construct) consume only this HIR,
//! never the raw AST.

use crate::record::RecordId;
use serde::Serialize;

/// Index of a [`Table`] in [`crate::SystemGraph`]'s table side table,
/// mirroring [`RecordId`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct TableId(pub u32);

/// A resolved `table <Name>: <Record>;` declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Table {
    pub name: String,
    pub record: RecordId,
}

/// The type of a value inside a handler body. Extends [`crate::FieldType`]
/// with HIR-only types that never appear as a record field's surface type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum HirType {
    Str,
    Int,
    Float,
    Bool,
    Uuid,
    Timestamp,
    Json,
    Enum {
        variants: Vec<String>,
    },
    /// A value of a declared record type.
    Record(RecordId),
    /// `db.get` and friends: absent vs. present, not an error.
    Option(Box<HirType>),
    /// `db.query`, `object_store.list`, `search.query` (v0.14 M1): an
    /// ordered collection, returned whole (there is no loop/iteration
    /// construct in the language — a handler either passes a list
    /// straight through as its `return` value or stores it as-is).
    List(Box<HirType>),
    /// The type of a verb call or `publish` with nothing meaningful to
    /// return, and of a statement-position expression.
    Unit,
    /// The bottom type: a block whose every path `return`s or `fail`s.
    /// Unifies with any type (a diverging `if`/`match` branch never
    /// actually produces a value, so it can't disagree with the other
    /// branch) — this is what makes
    /// `let x = match .. { A -> { return v; } B -> { fail E(); } };`
    /// type-check even though neither arm "returns" a value of `x`'s type.
    Never,
}

impl HirType {
    /// Unifies two branch/arm types for `if`/`match`: `Never` (a
    /// diverging branch) unifies with anything; otherwise the two types
    /// must be identical.
    pub fn unify(a: HirType, b: HirType) -> Result<HirType, (HirType, HirType)> {
        if a == b {
            Ok(a)
        } else if a == HirType::Never {
            Ok(b)
        } else if b == HirType::Never {
            Ok(a)
        } else {
            Err((a, b))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum UnOp {
    Neg,
    Not,
}

/// The closed set of capability operations a handler body may call,
/// already resolved to which bound capability instance and (for `db`)
/// which declared table they operate on. Deliberately small for M2 —
/// see `07UpdatePlan.md`'s own scope-creep warning; adding a verb is one
/// new match arm in `typeck.rs`'s verb table, not a redesign.
///
/// v0.14 M1 extends the set with query/mutation verbs across every
/// capability. Lowering (turning these into generated Python/Rust code)
/// is out of scope for M1 — see `14UpdatePlan.md`'s M2-M4 — so both
/// backends' `lower.rs` currently `todo!()` on them; typeck fully
/// validates and constructs the HIR regardless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verb {
    DbInsert(TableId),
    DbGet(TableId),
    /// Update the row named by a `Uuid` key with a full record value;
    /// `None` (absent vs. present, like `DbGet`) if no row has that key.
    DbUpdate(TableId),
    /// Delete the row named by a `Uuid` key; `true` if a row was deleted.
    DbDelete(TableId),
    /// `db.query(Table) [where <predicate>]` — every matching row, or
    /// every row when no predicate is given. See [`HirExpr::Query`].
    DbQuery(TableId),
    /// `db.count(Table) [where <predicate>]` — the number of matching
    /// (or all) rows. See [`HirExpr::Query`].
    DbCount(TableId),
    /// `db.delete_where(Table) [where <predicate>]` — deletes every
    /// matching (or all) rows, yielding the number deleted. See
    /// [`HirExpr::Query`].
    DbDeleteWhere(TableId),
    CacheGet,
    CacheSet,
    CacheDelete,
    ObjectStorePut,
    ObjectStoreGet,
    ObjectStoreDelete,
    /// `object_store.list(prefix)` — every key under `prefix`.
    ObjectStoreList,
    /// `email.send(to, subject, body)`.
    EmailSend,
    /// `search.index(doc_id, value)` — upserts `value` under `doc_id`.
    SearchIndex,
    /// `search.query(query)` — every matching document.
    SearchQuery,
    /// `external_http.request(url, body)` — a synchronous POST, returning
    /// the response body as `Json`.
    HttpCall,
}

/// Niladic builtin functions available in any handler body, independent
/// of capability bindings (`Uuid.new()`, `Timestamp.now()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Builtin {
    UuidNew,
    TimestampNow,
}

impl Builtin {
    pub fn ty(self) -> HirType {
        match self {
            Builtin::UuidNew => HirType::Uuid,
            Builtin::TimestampNow => HirType::Timestamp,
        }
    }
}

/// A resolved [`crate::ast`]-independent mirror of
/// `ciac_syntax::ast::PredOp` (v0.14 M1) — kept as a separate type so
/// `ciac-ir` doesn't depend on `ciac-syntax`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum PredOp {
    Eq,
    NotEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Contains,
}

/// One resolved `<field> <op> <value>` comparison of a [`HirPredicate`]
/// (v0.14 M1). `field`/`field_ty` are resolved against the target
/// verb's table; `value` is a fully type-checked expression.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HirPredTerm {
    pub field: String,
    pub field_ty: HirType,
    pub op: PredOp,
    pub value: HirExpr,
}

/// A resolved `where` clause: a conjunction of [`HirPredTerm`]s (v0.14
/// M1). See `ciac_syntax::ast::Predicate`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HirPredicate {
    pub terms: Vec<HirPredTerm>,
}

/// A type-checked expression. Every variant carries (or is) enough to
/// determine its [`HirType`] without re-walking children.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum HirExpr {
    /// A resolved reference to a parameter or `let` binding. `slot`
    /// indexes `HandlerBody::locals` (params occupy the first
    /// `params.len()` slots, in declaration order; `let`s follow).
    Local {
        slot: u32,
        ty: HirType,
    },
    IntLit(i64),
    FloatLit(f64),
    StrLit(String),
    BoolLit(bool),
    BuiltinCall(Builtin),
    /// A bare enum variant name (`Ready`) resolved from the expected type
    /// at its use site (a comparison operand or a record field value) —
    /// there is no other way to know which enum a bare variant name
    /// belongs to.
    EnumLit {
        variants: Vec<String>,
        variant: String,
    },
    FieldAccess {
        base: Box<HirExpr>,
        field: String,
        ty: HirType,
    },
    /// `<base>[<index>]` — `Json` values only.
    Index {
        base: Box<HirExpr>,
        index: Box<HirExpr>,
    },
    /// Full record construction (`base_value: None`) or a functional
    /// update (`base_value: Some(existing value)`); either way, fields
    /// not listed take the corresponding field from `base_value` when
    /// present, or must all be listed when constructing fresh.
    RecordCons {
        record: RecordId,
        base_value: Option<Box<HirExpr>>,
        fields: Vec<(String, HirExpr)>,
    },
    Binary {
        op: BinOp,
        lhs: Box<HirExpr>,
        rhs: Box<HirExpr>,
        ty: HirType,
    },
    Unary {
        op: UnOp,
        expr: Box<HirExpr>,
        ty: HirType,
    },
    If {
        cond: Box<HirExpr>,
        then_branch: Vec<HirStmt>,
        else_branch: Vec<HirStmt>,
        ty: HirType,
    },
    Match {
        scrutinee: Box<HirExpr>,
        arms: Vec<HirArm>,
        ty: HirType,
    },
    /// A capability verb call, already resolved to the bound instance
    /// node it targets.
    VerbCall {
        capability: crate::NodeId,
        verb: Verb,
        args: Vec<HirExpr>,
        ty: HirType,
    },
    /// `db.query`/`db.count`/`db.delete_where`, with or without a
    /// `where` clause (v0.14 M1). Kept distinct from [`HirExpr::VerbCall`]
    /// because these three verbs carry a predicate, which isn't a plain
    /// argument list — see `ciac_syntax::ast::Expr::Query`.
    Query {
        capability: crate::NodeId,
        verb: Verb,
        predicate: Option<HirPredicate>,
        ty: HirType,
    },
}

impl HirExpr {
    pub fn ty(&self) -> HirType {
        match self {
            HirExpr::Local { ty, .. }
            | HirExpr::FieldAccess { ty, .. }
            | HirExpr::Binary { ty, .. }
            | HirExpr::Unary { ty, .. }
            | HirExpr::If { ty, .. }
            | HirExpr::Match { ty, .. }
            | HirExpr::VerbCall { ty, .. }
            | HirExpr::Query { ty, .. } => ty.clone(),
            HirExpr::IntLit(_) => HirType::Int,
            HirExpr::FloatLit(_) => HirType::Float,
            HirExpr::StrLit(_) => HirType::Str,
            HirExpr::BoolLit(_) => HirType::Bool,
            HirExpr::Index { .. } => HirType::Json,
            HirExpr::RecordCons { record, .. } => HirType::Record(*record),
            HirExpr::BuiltinCall(builtin) => builtin.ty(),
            HirExpr::EnumLit { variants, .. } => HirType::Enum {
                variants: variants.clone(),
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HirArm {
    /// `None` is the wildcard (`_`) arm.
    pub variant: Option<String>,
    pub body: Vec<HirStmt>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum HirStmt {
    Let {
        slot: u32,
        value: HirExpr,
    },
    Expr(HirExpr),
    Return(Option<HirExpr>),
    Fail {
        error: RecordId,
        args: Vec<HirExpr>,
    },
    Publish {
        stream: crate::NodeId,
        value: HirExpr,
    },
    /// `transaction { .. }` (v0.16 M1/M2): every database verb in `body`
    /// shares one database transaction. Sema (`typeck.rs`) has already
    /// rejected `return`, nested `transaction`, `publish`, and every
    /// non-database capability verb inside — `body` contains only
    /// `Let`/`Expr`/`Fail` and (recursively, inside `if`/`match`) more of
    /// the same, so lowering never has to re-check these invariants.
    Transaction {
        body: Vec<HirStmt>,
    },
}

/// The type-checked body of a `handler Name(params) -> Type { .. }` or
/// `extern handler Name(params) -> Type;` declaration.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HandlerBody {
    pub params: Vec<(String, HirType)>,
    pub return_ty: HirType,
    /// Every local slot's type, params first (see [`HirExpr::Local`]).
    pub locals: Vec<HirType>,
    /// `None` for `extern handler` — a checked signature with no body.
    pub body: Option<Vec<HirStmt>>,
}

impl HandlerBody {
    /// Every capability instance node a `VerbCall` in this body resolves
    /// to, deduplicated in first-use order. There are no `DataFlow`
    /// edges from a typed handler's (not-yet-existent, at type-check
    /// time) node to these instances the way classic handler bindings
    /// get one, so both the reachability pass (is a capability ever
    /// used?) and codegen's shared model (does this handler need a
    /// `session`/`cache`/...?) walk the HIR directly instead.
    pub fn capability_nodes(&self) -> Vec<crate::NodeId> {
        let mut nodes = Vec::new();
        if let Some(stmts) = &self.body {
            capability_nodes_block(stmts, &mut nodes);
        }
        nodes
    }
}

fn capability_nodes_block(stmts: &[HirStmt], out: &mut Vec<crate::NodeId>) {
    for stmt in stmts {
        match stmt {
            HirStmt::Let { value, .. } | HirStmt::Expr(value) => capability_nodes_expr(value, out),
            HirStmt::Return(Some(value)) => capability_nodes_expr(value, out),
            HirStmt::Return(None) => {}
            HirStmt::Fail { args, .. } => {
                for arg in args {
                    capability_nodes_expr(arg, out);
                }
            }
            HirStmt::Publish { value, .. } => capability_nodes_expr(value, out),
            HirStmt::Transaction { body } => capability_nodes_block(body, out),
        }
    }
}

fn capability_nodes_expr(expr: &HirExpr, out: &mut Vec<crate::NodeId>) {
    match expr {
        HirExpr::VerbCall {
            capability, args, ..
        } => {
            if !out.contains(capability) {
                out.push(*capability);
            }
            for arg in args {
                capability_nodes_expr(arg, out);
            }
        }
        HirExpr::FieldAccess { base, .. } | HirExpr::Unary { expr: base, .. } => {
            capability_nodes_expr(base, out)
        }
        HirExpr::Index { base, index } => {
            capability_nodes_expr(base, out);
            capability_nodes_expr(index, out);
        }
        HirExpr::RecordCons {
            base_value, fields, ..
        } => {
            if let Some(base) = base_value {
                capability_nodes_expr(base, out);
            }
            for (_, value) in fields {
                capability_nodes_expr(value, out);
            }
        }
        HirExpr::Binary { lhs, rhs, .. } => {
            capability_nodes_expr(lhs, out);
            capability_nodes_expr(rhs, out);
        }
        HirExpr::If {
            cond,
            then_branch,
            else_branch,
            ..
        } => {
            capability_nodes_expr(cond, out);
            capability_nodes_block(then_branch, out);
            capability_nodes_block(else_branch, out);
        }
        HirExpr::Match {
            scrutinee, arms, ..
        } => {
            capability_nodes_expr(scrutinee, out);
            for arm in arms {
                capability_nodes_block(&arm.body, out);
            }
        }
        HirExpr::Query {
            capability,
            predicate,
            ..
        } => {
            if !out.contains(capability) {
                out.push(*capability);
            }
            if let Some(predicate) = predicate {
                for term in &predicate.terms {
                    capability_nodes_expr(&term.value, out);
                }
            }
        }
        HirExpr::Local { .. }
        | HirExpr::IntLit(_)
        | HirExpr::FloatLit(_)
        | HirExpr::StrLit(_)
        | HirExpr::BoolLit(_)
        | HirExpr::BuiltinCall(_)
        | HirExpr::EnumLit { .. } => {}
    }
}
