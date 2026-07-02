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
    /// `service <Name>;` — names the system being described.
    Service(ServiceDecl),
    /// `use { auth JWT; db Postgres; .. }` — capability requirements.
    Use(UseBlock),
    /// `api <Name>;`
    Api(ComponentDecl),
    /// `worker <Name>;`
    Worker(ComponentDecl),
    /// `crud <Name>;` — expands to API + Auth + Service + Database (+ Cache).
    Crud(ComponentDecl),
    /// `events <Name>;` — expands to Queue + Worker + Storage.
    Events(ComponentDecl),
    /// `pipeline <Name>: Step -> Step -> ..;`
    Pipeline(PipelineDecl),
}

#[derive(Debug, Clone, Serialize)]
pub struct ServiceDecl {
    pub name: Ident,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentDecl {
    pub name: Ident,
    pub span: Span,
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
    pub provider: Ident,
    pub span: Span,
}

#[derive(Debug, Clone, Serialize)]
pub struct PipelineDecl {
    pub name: Ident,
    pub steps: Vec<Ident>,
    pub span: Span,
}
