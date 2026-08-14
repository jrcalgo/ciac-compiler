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

pub mod backfill;
pub mod ci;
pub mod compose;
pub mod emit;
pub mod evolution;
pub mod external;
pub mod format_batch;
pub mod k8s;
pub mod lower;
pub mod manifest;
pub mod migrations;
pub mod model;
pub mod openapi;
mod project;
pub mod protocol;
pub mod regen;
pub mod semantic_diff;
pub mod semantic_model;
pub mod system_tests;
pub mod template;
pub mod terraform;
pub mod ts_client;
pub mod users;

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

    /// The whole CLI/CI/compose/dev-loop/sim integration surface for this
    /// target, as data (v0.22 M1 — `22UpdatePlan.md` Pillar 1). Every
    /// caller that used to `match target { "python" => .., "rust" => ..
    /// }` reads this instead, so adding a target is a registry entry, not
    /// an edit to six files.
    fn target_info(&self) -> &'static TargetInfo;
}

/// One command `ciac verify`/`ciac build` runs against a generated
/// project, in order, with its own environment and a human-readable
/// clause for error messages (v0.22 M1).
#[derive(Debug, Clone, Copy)]
pub struct ValidateStep {
    /// e.g. `"uv"`, `"cargo"`, `"npm"`.
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// e.g. `[("RUSTFLAGS", "-D warnings")]`, `[("CGO_ENABLED", "0")]`.
    pub env: &'static [(&'static str, &'static str)],
    /// Names what this step proves, so a failure reads "type-checks
    /// failed" rather than only "npm exited 1".
    pub purpose: &'static str,
}

/// Whether `ciac dev` restarts the target's process on rebuild or
/// delegates to the target's own file watcher. Every current target is
/// restart-style; the field exists so a future watcher-style target
/// doesn't force a special case into `dev.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartStyle {
    Restart,
    DelegatedWatch,
}

/// Commands `ciac dev`'s watch loop uses to rebuild/restart a generated
/// project on change (v0.22 M1).
#[derive(Debug, Clone, Copy)]
pub struct DevCommands {
    /// Re-run after a successful regeneration, e.g. `npm run build` /
    /// `go build ./...`. Empty for targets whose restart implies rebuild
    /// (both current targets).
    pub rebuild: &'static [ValidateStep],
    pub restart: RestartStyle,
}

/// A target's `ciac sim` support level (v0.22 M1, generalizing the
/// v0.17 M11 Rust narrowing). `None` with a reason is a permanently
/// valid state — a refusal must be clean and specific, never a silent
/// no-op.
#[derive(Clone, Copy)]
pub enum SimSupport {
    /// Every verb/capability the language supports is simulatable
    /// (Python).
    Full,
    /// Only a subset is simulatable; the function names the specific
    /// unsupported verbs/capabilities a program uses, if any (promoted
    /// from `ciac_backend_rust::unsupported_sim_capabilities`).
    Narrow {
        unsupported: fn(&NormalizedIr) -> Vec<String>,
    },
    /// Simulation is not implemented for this target at all.
    None { reason: &'static str },
}

impl std::fmt::Debug for SimSupport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SimSupport::Full => write!(f, "Full"),
            SimSupport::Narrow { .. } => write!(f, "Narrow"),
            SimSupport::None { reason } => write!(f, "None({reason})"),
        }
    }
}

/// The whole per-target integration surface, as data (v0.22 M1 —
/// `22UpdatePlan.md` Pillar 1). Replaces ~25 scattered
/// `match target { "python" => .., "rust" => .. }` sites across
/// `commands.rs`/`ci.rs`/`vocab.rs`/`dev.rs`/`compose.rs`.
#[derive(Debug)]
pub struct TargetInfo {
    /// Project marker file identifying a generated project root, e.g.
    /// `"pyproject.toml"`, `"Cargo.toml"`. Consumed by
    /// `find_project_dirs` everywhere.
    pub project_marker: &'static str,
    /// Where CIaC-owned migration SQL lives inside the project, e.g.
    /// `"app/migrations"`, `"migrations"`.
    pub migrations_dir: &'static str,
    /// Per-target migration filename mapping. Identity for every
    /// current target.
    pub migration_filename: fn(seq: u32, slug: &str) -> String,
    /// Commands `ciac verify`/`build` run to validate a generated
    /// project, in order.
    pub validate: &'static [ValidateStep],
    /// `32UpdatePlan.md` M8 item 5: `validate[validate_parallel_from..]`
    /// is order-independent by this target's own contract (neither
    /// writes into the project tree nor reads output another step in
    /// that suffix writes) and may run concurrently once
    /// `validate[..validate_parallel_from]` completes in order.
    /// `None` (the default for every target until proven otherwise —
    /// see `32UpdatePlan.md`'s open question 4) means fully serial,
    /// matching this field's absence before this milestone exactly.
    pub validate_parallel_from: Option<usize>,
    /// The literal CI test-step YAML `ci.rs` embeds for this target.
    pub ci_test_steps: &'static str,
    /// Compose parameterization (the pre-existing `BackendComposeOpts`,
    /// now reached through the trait instead of a separate per-call-site
    /// match).
    pub compose: compose::BackendComposeOpts,
    /// Commands `ciac dev` uses to rebuild/restart on change.
    pub dev: DevCommands,
    /// Extension (no dot) of this target's seeded handler source files
    /// under `app/services`/`src/services`, e.g. `"py"`, `"rs"`. `ciac
    /// dev`'s watch loop unions this across every registered backend
    /// (v0.22 M1) rather than hardcoding the current two — unchanged
    /// behavior today, automatic coverage for the next target.
    pub source_extension: &'static str,
    /// Simulation support level.
    pub sim: SimSupport,
    /// Whether this target's `ciac sim` runner implements `--record`/
    /// `--replay` (27UpdatePlan.md M1). Decoupled from `sim` on
    /// purpose: simulation depth (`SimSupport`) and replay-tape support
    /// are separate capabilities — a target can simulate every verb
    /// the language has and still not implement a replay tape, and the
    /// reverse is meaningless but the types don't need to enforce that.
    /// Before this field existed, `sim_inner` inferred replay support
    /// from `SimSupport::Narrow`, which would have quietly (and
    /// wrongly) become "replay works everywhere" the moment a `Narrow`
    /// target flipped to `Full`. Only Python's runner implements replay
    /// today; every other target names its own scope explicitly here
    /// rather than inheriting an unrelated flag's truth.
    pub sim_replay: bool,
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
