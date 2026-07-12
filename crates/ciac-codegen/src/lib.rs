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

pub mod compose;
pub mod evolution;
pub mod external;
pub mod k8s;
pub mod manifest;
pub mod migrations;
pub mod model;
mod project;
pub mod protocol;
pub mod regen;
pub mod system_tests;
pub mod template;
pub mod terraform;

pub use project::{FileRole, GeneratedProject};

use ciac_ir::{Component, NormalizedIr};

/// Options shared by all backends.
#[derive(Debug, Clone, Default)]
pub struct GenOptions {
    /// Overrides the generated project's package/crate name
    /// (defaults to the kebab-cased service name).
    pub project_name: Option<String>,
}

/// Deployment sizing profile (v0.11): selected with `--profile`,
/// threaded into the k8s and Terraform generators. Compose is the
/// dev-only path and ignores it. Generated *application* code is
/// profile-independent by design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Profile {
    #[default]
    Dev,
    Staging,
    Prod,
}

impl Profile {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "dev" => Some(Self::Dev),
            "staging" => Some(Self::Staging),
            "prod" => Some(Self::Prod),
            _ => None,
        }
    }

    pub fn is_dev(self) -> bool {
        self == Self::Dev
    }

    /// k8s Deployment replicas per service.
    pub fn replicas(self) -> u32 {
        match self {
            Self::Dev => 1,
            Self::Staging => 2,
            Self::Prod => 3,
        }
    }

    pub fn db_instance_class(self) -> &'static str {
        match self {
            Self::Dev => "db.t4g.micro",
            Self::Staging => "db.t4g.small",
            Self::Prod => "db.m6g.large",
        }
    }

    pub fn db_storage_gb(self) -> u32 {
        match self {
            Self::Dev => 20,
            Self::Staging => 50,
            Self::Prod => 100,
        }
    }

    pub fn cache_node_type(self) -> &'static str {
        match self {
            Self::Dev => "cache.t4g.micro",
            Self::Staging => "cache.t4g.small",
            Self::Prod => "cache.m6g.large",
        }
    }

    pub fn kafka_brokers(self) -> u32 {
        match self {
            Self::Dev | Self::Staging => 2,
            Self::Prod => 3,
        }
    }

    pub fn kafka_instance_type(self) -> &'static str {
        match self {
            Self::Dev | Self::Staging => "kafka.t3.small",
            Self::Prod => "kafka.m5.large",
        }
    }
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
