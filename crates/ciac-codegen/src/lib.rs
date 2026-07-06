//! Backend-agnostic code generation for the CIaC compiler.
//!
//! Every target language implements the [`Backend`] trait: it consumes the
//! same validated [`NormalizedIr`] and produces a [`GeneratedProject`] — an
//! in-memory, deterministic file tree. Nothing in this crate knows about
//! any particular language; adding a new target means adding a new crate
//! that implements [`Backend`] and registering it with the CLI.
//!
//! # Determinism
//!
//! Generated output must be byte-identical for identical input:
//! * files live in a sorted map, so iteration and writing order are stable;
//! * backends must not embed timestamps, absolute paths, or randomness;
//! * IR iteration order is itself deterministic (declaration order).

pub mod evolution;
pub mod manifest;
pub mod migrations;
pub mod model;
mod project;
pub mod regen;
pub mod system_tests;
pub mod template;

pub use project::{FileRole, GeneratedProject};

use ciac_ir::{Component, NormalizedIr};

/// Options shared by all backends.
#[derive(Debug, Clone, Default)]
pub struct GenOptions {
    /// Overrides the generated project's package/crate name
    /// (defaults to the kebab-cased service name).
    pub project_name: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// The program uses a component this backend has no implementation
    /// for. The CLI reports this as diagnostic `CIAC0011`.
    #[error("backend `{backend}` does not support {construct}")]
    Unsupported {
        backend: &'static str,
        construct: String,
    },
    #[error("template error: {0}")]
    Template(#[from] minijinja::Error),
    #[error("{0}")]
    Other(String),
}

/// A code-generation target.
pub trait Backend {
    /// Stable identifier used for `--target <id>`.
    fn id(&self) -> &'static str;

    /// Human-readable description shown in `--help` and error messages.
    fn description(&self) -> &'static str;

    /// Whether this backend can implement the given component. Called for
    /// every node before generation via [`check_support`].
    fn supports(&self, component: &Component) -> bool;

    /// Generates a complete project from validated IR.
    ///
    /// Implementations may assume all components passed [`Backend::supports`]
    /// and the IR passed semantic analysis.
    fn generate(
        &self,
        ir: &NormalizedIr,
        opts: &GenOptions,
    ) -> Result<GeneratedProject, BackendError>;
}

/// Verifies every component in the IR is supported by `backend`, returning
/// the first unsupported construct as an error.
pub fn check_support(backend: &dyn Backend, ir: &NormalizedIr) -> Result<(), BackendError> {
    for node in ir.nodes() {
        if !backend.supports(&node.component) {
            return Err(BackendError::Unsupported {
                backend: backend.id(),
                construct: node.component.label(),
            });
        }
    }
    Ok(())
}

/// The project name for the generated output: explicit override or the
/// kebab-cased service name.
pub fn project_name(ir: &NormalizedIr, opts: &GenOptions) -> String {
    use heck::ToKebabCase;
    opts.project_name
        .clone()
        .unwrap_or_else(|| ir.name.to_kebab_case())
}
