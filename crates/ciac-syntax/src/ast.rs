//! Abstract syntax tree for CIaC programs.
//!
//! The AST mirrors the surface grammar: it preserves declaration order and
//! spans but performs no name resolution or validation — that is the job of
//! `ciac-sema`, which lowers the AST into the typed system graph.

use ciac_diagnostics::Span;
use serde::Serialize;

/// An identifier with its source location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Ident {
    pub text: String,
    pub span: Span,
}

/// A whole CIaC source file.
#[derive(Debug, Clone, Serialize)]
pub struct Program {
    pub items: Vec<Item>,
}

/// A top-level declaration.
#[derive(Debug, Clone, Serialize)]
pub enum Item {
    /// `import "path";` (v0.8) — textually splices another file's items
    /// in at this position; resolved before semantic analysis ever sees
    /// the program, so nothing downstream of the parser is aware
    /// multi-file programs exist. See `crate::module`.
    Import(ImportDecl),
    /// `blueprint <Name><<TypeParam>: record> { .. }` (v0.8) — a
    /// parameterized template, expanded per `expand` site rather than
    /// processed directly; never reaches `ciac-sema`'s graph builder.
    /// See `ciac_sema::blueprints`.
    Blueprint(BlueprintDecl),
    /// `expand <Blueprint><<Record>> { param: value; .. };` (v0.8) — at
    /// top level (single-service programs); see `ServiceItem::Expand`
    /// for the form inside a `service { .. }` block.
    Expand(ExpandStmt),
    /// `project <Name>;` — names a multi-service project.
    Project(ProjectDecl),
    /// `service <Name>;` — names the system being described.
    Service(ServiceDecl),
    /// `service <Name> { .. }` — a deployable service scope.
    ServiceBlock(ServiceBlock),
    /// `use { auth JWT; db Postgres; .. }` — capability requirements.
    Use(UseBlock),
    /// `record <Name> { field: Type; .. }` — a typed data schema.
    Record(RecordDecl),
    /// `stream <Name>: <Record>;` — a named, typed message channel.
    Stream(StreamDecl),
    /// `table <Name>: <Record>;` — a named, typed persistent table (v0.7).
    Table(TableDecl),
    /// `api <Name>[: <Record>];`
    Api(ApiDecl),
    /// `worker <Name> [on <Stream>];`
    Worker(WorkerDecl),
    /// `job <Name> { schedule: "..."; }`
    Job(JobDecl),
    /// `channel <Name> on <Stream>;`
    Channel(ChannelDecl),
    /// `crud <Name>[: <Record>];` — expands to API + Auth + Service + Database.
    Crud(CrudDecl),
    /// `events <Name>;` — expands to Stream + Worker.
    Events(ComponentDecl),
    /// `handler <Name> { db: main; .. }` — binds a handler to capability instances.
    Handler(HandlerDecl),
    /// `pipeline <Name>: Step -> Step -> ..;`
    Pipeline(PipelineDecl),
}

/// `import "path";` (v0.8). `path` is resolved relative to the
/// importing file's own directory, not the entry file's.
#[derive(Debug, Clone, Serialize)]
pub struct ImportDecl {
    pub path: String,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectDecl {
    pub name: Ident,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceDecl {
    pub name: Ident,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceBlock {
    pub name: Ident,
    pub items: Vec<ServiceItem>,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub enum ServiceItem {
    Use(UseBlock),
    Api(ApiDecl),
    Worker(WorkerDecl),
    Job(JobDecl),
    Channel(ChannelDecl),
    Crud(CrudDecl),
    Events(ComponentDecl),
    Handler(HandlerDecl),
    Pipeline(PipelineDecl),
    /// `expand <Blueprint><<Record>> { .. };` (v0.8) inside a service
    /// block. See `Item::Expand`.
    Expand(ExpandStmt),
}

/// `blueprint <Name><<TypeParam>: record> { params { .. } <body> }`
/// (v0.8). `type_param`'s only supported constraint is `record`
/// (checked at each `expand` site, not here); `body` is deliberately
/// narrower than the full item grammar — see `BlueprintItem`.
#[derive(Debug, Clone, Serialize)]
pub struct BlueprintDecl {
    pub name: Ident,
    pub type_param: Ident,
    /// `params { name: Type; .. }` — scalar-only (mirrors `AttrValue`'s
    /// closed `Ident | Number | Str` set: only `String`/`Int` field
    /// types are meaningful here).
    pub params: Vec<Field>,
    pub body: Vec<BlueprintItem>,
    pub span: Span,
}

/// The closed set of declarations a `blueprint` body may contain
/// (v0.8 M2). Not the full `Item`/`ServiceItem` grammar: no nested
/// `record`/`table`/`api`/`worker`/`job`/`channel`/`events`/
/// `blueprint`/`expand`/`import` — a deliberate scope limit, not an
/// oversight. No `pipeline` either: a blueprint body declares no
/// `api`/`worker`/`job` of its own for one to attach to (`crud`
/// expands to a complete REST resource on its own, with no pipeline
/// involved); a pipeline that attaches to something in the *enclosing*
/// scope belongs there, not inside the template.
#[derive(Debug, Clone, Serialize)]
pub enum BlueprintItem {
    Use(UseBlock),
    Crud(CrudDecl),
    Stream(StreamDecl),
    Handler(HandlerDecl),
}

/// `expand <Blueprint><<Record>> { field: value; .. };` (v0.8).
/// `args` reuses the same attribute grammar as `crud { .. }`/`api {
/// .. }` attribute blocks (`decl_tail`), so a bare `;` means no
/// params.
#[derive(Debug, Clone, Serialize)]
pub struct ExpandStmt {
    pub blueprint: Ident,
    pub type_arg: Ident,
    pub args: Vec<Attr>,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentDecl {
    pub name: Ident,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordDecl {
    pub name: Ident,
    pub fields: Vec<Field>,
    pub kind: RecordKind,
    pub span: Span,
}

/// Distinguishes `record` (plain data) from `error` (v0.7) declarations.
/// Both share the same field grammar; `error` records additionally back
/// the `fail <ErrorName>` control-flow construct in handler bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RecordKind {
    Data,
    Error,
}

/// One `name: Type;` line of a record body.
#[derive(Debug, Clone, Serialize)]
pub struct Field {
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

/// A field type: a named type (primitive in v0.2) or an inline enum.
#[derive(Debug, Clone, Serialize)]
pub enum TypeExpr {
    Named(Ident),
    Enum { variants: Vec<Ident>, span: Span },
}

impl TypeExpr {
    pub fn span(&self) -> Span {
        match self {
            TypeExpr::Named(ident) => ident.span,
            TypeExpr::Enum { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamDecl {
    pub name: Ident,
    pub record: Ident,
    pub attrs: Vec<Attr>,
    pub span: Span,
}

/// `table <Name>: <Record>;` — a named, typed persistent table (v0.7).
/// `db.*` verbs in handler bodies operate on tables, not raw records.
#[derive(Debug, Clone, Serialize)]
pub struct TableDecl {
    pub name: Ident,
    pub record: Ident,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct ApiDecl {
    pub name: Ident,
    /// Request body record, when the api is typed.
    pub request: Option<Ident>,
    pub attrs: Vec<Attr>,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkerDecl {
    pub name: Ident,
    /// Stream the worker consumes; `None` means the service's default
    /// stream (v0.1 behavior).
    pub stream: Option<Ident>,
    pub attrs: Vec<Attr>,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobDecl {
    pub name: Ident,
    pub attrs: Vec<Attr>,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelDecl {
    pub name: Ident,
    pub stream: Ident,
    pub attrs: Vec<Attr>,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrudDecl {
    pub name: Ident,
    /// Record supplying real columns; `None` keeps the generic
    /// keyed-document model.
    pub record: Option<Ident>,
    pub attrs: Vec<Attr>,
    pub span: Span,
}

/// A closed-registry component attribute parsed from an attribute block.
#[derive(Debug, Clone, Serialize)]
pub struct Attr {
    pub name: Ident,
    pub value: AttrValue,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub enum AttrValue {
    Ident(Ident),
    Number { value: u64, span: Span },
    Str { value: String, span: Span },
}

impl AttrValue {
    pub fn span(&self) -> Span {
        match self {
            AttrValue::Ident(ident) => ident.span,
            AttrValue::Number { span, .. } | AttrValue::Str { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UseBlock {
    pub entries: Vec<UseEntry>,
    pub span: Span,
}

/// One `capability Provider;` line of a use block, e.g. `db Postgres;`.
#[derive(Debug, Clone, Serialize)]
pub struct UseEntry {
    pub capability: Ident,
    pub name: Option<Ident>,
    pub provider: Option<Ident>,
    pub attrs: Vec<Attr>,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct HandlerDecl {
    pub name: Ident,
    pub bindings: Vec<HandlerBinding>,
    /// Typed parameters, e.g. `(v: Video)` (v0.7). Empty for the classic
    /// binding-only form, which has no signature.
    pub params: Vec<Param>,
    /// The `-> Type` return type (v0.7). `None` for the classic form.
    pub return_ty: Option<TypeExpr>,
    /// The inline `{ <stmts> }` body (v0.7). `None` means this handler is
    /// implemented out-of-band (today's stub behavior, or `extern`) —
    /// existing bare `handler Name { .. }` programs always have `None`
    /// here, so they parse identically to before v0.7.
    pub body: Option<Vec<Stmt>>,
    /// Declared via `extern handler Name(..) -> Type;` (v0.7): a typed
    /// signature with a seeded, user-implemented body. `false` for both
    /// the classic binding form and the new inline-body form.
    pub is_extern: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct HandlerBinding {
    pub capability: Ident,
    pub instance: Ident,
    pub span: Span,
}

/// One `name: Type` entry of a handler's parameter list (v0.7).
#[derive(Debug, Clone, Serialize)]
pub struct Param {
    pub name: Ident,
    pub ty: TypeExpr,
    pub span: Span,
}

/// One step of a pipeline.
#[derive(Debug, Clone, Serialize)]
pub enum StepExpr {
    /// A builtin (`Auth`, `Queue`, `Return`) or a handler name.
    Name(Ident),
    /// `publish <Stream>` — publish the current payload to a named stream.
    Publish(Ident),
    /// `call <Service>.<Api>` — synchronously invoke another service's api.
    Call(QualifiedIdent),
    /// `match field { Variant -> Step; _ -> Step; }`.
    Match {
        field: Ident,
        arms: Vec<Arm>,
        span: Span,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct QualifiedIdent {
    pub segments: Vec<Ident>,
    pub span: Span,
}

impl StepExpr {
    pub fn span(&self) -> Span {
        match self {
            StepExpr::Name(ident) | StepExpr::Publish(ident) => ident.span,
            StepExpr::Call(ident) => ident.span,
            StepExpr::Match { span, .. } => *span,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Arm {
    pub label: ArmLabel,
    pub steps: Vec<StepExpr>,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub enum ArmLabel {
    Variant(Ident),
    Default(Span),
}

impl ArmLabel {
    pub fn span(&self) -> Span {
        match self {
            ArmLabel::Variant(ident) => ident.span,
            ArmLabel::Default(span) => *span,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineDecl {
    pub name: Ident,
    pub steps: Vec<StepExpr>,
    pub span: Span,
}

// ---------------------------------------------------------------------
// v0.7 M1: the handler-body expression language. Purely syntactic: no
// name resolution, no typing — `ciac-sema` (M2+) resolves verbs, checks
// types, and lowers this into the typed HIR. Everything below mirrors the
// existing "spans, no validation" contract this file states up top.
// ---------------------------------------------------------------------

/// One statement inside a handler body.
#[derive(Debug, Clone, Serialize)]
pub enum Stmt {
    /// `let <name> = <expr>;` — single-assignment, block-scoped.
    Let {
        name: Ident,
        value: Expr,
        span: Span,
    },
    /// A bare expression statement, e.g. a capability verb call.
    Expr(Expr),
    /// `return <expr>?;`
    Return { value: Option<Expr>, span: Span },
    /// `fail <ErrorName>(<args>);` — an early, typed error response.
    Fail {
        error: Ident,
        args: Vec<Expr>,
        span: Span,
    },
    /// `publish <Stream>(<value>);` — publish a value to a named stream
    /// from inside a handler body (v0.7 M2), reusing the same stream
    /// resolution and payload-type checking as the pipeline-level
    /// `publish <Stream>` step.
    Publish {
        stream: Ident,
        value: Expr,
        span: Span,
    },
}

impl Stmt {
    pub fn span(&self) -> Span {
        match self {
            Stmt::Let { span, .. }
            | Stmt::Return { span, .. }
            | Stmt::Fail { span, .. }
            | Stmt::Publish { span, .. } => *span,
            Stmt::Expr(expr) => expr.span(),
        }
    }
}

/// An expression inside a handler body.
#[derive(Debug, Clone, Serialize)]
pub enum Expr {
    Ident(Ident),
    /// A numeric literal, kept as source text — int/float distinction is
    /// a typing concern, not a parsing one.
    Number {
        text: String,
        span: Span,
    },
    Str {
        value: String,
        span: Span,
    },
    Bool {
        value: bool,
        span: Span,
    },
    /// `<base>.<field>` — also the receiver half of a verb/method call,
    /// e.g. the `object_store.put` in `object_store.put(key, v)`.
    FieldAccess {
        base: Box<Expr>,
        field: Ident,
        span: Span,
    },
    /// `<base>[<index>]` — `Json` field indexing.
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
        span: Span,
    },
    /// `<callee>(<args>)` — a capability verb, or a builtin like
    /// `Uuid.new()`; `callee` is typically a `FieldAccess`.
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
        span: Span,
    },
    /// `<base> { field: value, .. }` — either full construction
    /// (`Video { .. }`, `base` names a record type) or a functional
    /// update (`v { status: Ready }`, `base` is a record-valued
    /// expression). The two are syntactically identical — `base` is a
    /// bare identifier either way — so telling them apart requires a
    /// symbol table and is deferred to typeck (M2), not decided here.
    RecordCons {
        base: Box<Expr>,
        fields: Vec<FieldInit>,
        span: Span,
    },
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
        span: Span,
    },
    Unary {
        op: UnOp,
        expr: Box<Expr>,
        span: Span,
    },
    /// `if <cond> { <stmts> } [else { <stmts> }]`, usable as an expression.
    If {
        cond: Box<Expr>,
        then_branch: Vec<Stmt>,
        else_branch: Option<Vec<Stmt>>,
        span: Span,
    },
    /// `match <scrutinee> { Variant -> { <stmts> } _ -> { <stmts> } }`,
    /// reusing the pipeline `match`'s [`ArmLabel`] and exhaustiveness
    /// machinery.
    Match {
        scrutinee: Box<Expr>,
        arms: Vec<ExprArm>,
        span: Span,
    },
}

impl Expr {
    pub fn span(&self) -> Span {
        match self {
            Expr::Ident(ident) => ident.span,
            Expr::Number { span, .. }
            | Expr::Str { span, .. }
            | Expr::Bool { span, .. }
            | Expr::FieldAccess { span, .. }
            | Expr::Index { span, .. }
            | Expr::Call { span, .. }
            | Expr::RecordCons { span, .. }
            | Expr::Binary { span, .. }
            | Expr::Unary { span, .. }
            | Expr::If { span, .. }
            | Expr::Match { span, .. } => *span,
        }
    }
}

/// One `name: value` entry of a record literal or functional update.
#[derive(Debug, Clone, Serialize)]
pub struct FieldInit {
    pub name: Ident,
    pub value: Expr,
    pub span: Span,
}

/// One arm of an expression-position `match`.
#[derive(Debug, Clone, Serialize)]
pub struct ExprArm {
    pub label: ArmLabel,
    pub body: Vec<Stmt>,
    pub span: Span,
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
