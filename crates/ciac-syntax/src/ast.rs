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
    /// `api <Name>[: <Record>];`
    Api(ApiDecl),
    /// `worker <Name> [on <Stream>];`
    Worker(WorkerDecl),
    /// `crud <Name>[: <Record>];` — expands to API + Auth + Service + Database.
    Crud(CrudDecl),
    /// `events <Name>;` — expands to Stream + Worker.
    Events(ComponentDecl),
    /// `handler <Name> { db: main; .. }` — binds a handler to capability instances.
    Handler(HandlerDecl),
    /// `pipeline <Name>: Step -> Step -> ..;`
    Pipeline(PipelineDecl),
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
    Crud(CrudDecl),
    Events(ComponentDecl),
    Handler(HandlerDecl),
    Pipeline(PipelineDecl),
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
    pub span: Span,
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
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct HandlerBinding {
    pub capability: Ident,
    pub instance: Ident,
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
