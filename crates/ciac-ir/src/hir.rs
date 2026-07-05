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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Verb {
    DbInsert(TableId),
    DbGet(TableId),
    CacheGet,
    CacheSet,
    ObjectStorePut,
    ObjectStoreGet,
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
            | HirExpr::VerbCall { ty, .. } => ty.clone(),
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
