//! The reference skeleton for authoring a new in-process CIaC backend
//! (v0.22 M5 — `22UpdatePlan.md` Pillar 5/6). **Not a real target**:
//! it is never registered in `crates/ciac/src/commands.rs::backends()`
//! and `supports()` always refuses, so it can never be reached through
//! the real CLI. Its only job is to compile, under the workspace
//! build, as a living demonstration of the trait/`TargetInfo`/emission-
//! table recipe `docs/backends.md` walks through — copy this crate,
//! not a blank file, when starting a real one.
//!
//! What this skeleton demonstrates: the `Backend` trait impl,
//! `TargetInfo` construction, and the declarative `Emit` table
//! (`ciac_codegen::emit`) rendering its whole (tiny, synthetic) file
//! set. What it does **not** demonstrate: per-language type filters,
//! `HostSyntax` leaf lowering, or per-item emission (one file per
//! declared api/worker/job/...). The first two now *are* a frozen,
//! real contract — `ciac_codegen::lower::HostSyntax`, completed as a
//! `22UpdatePlan.md` Pillar 3 continuation after M3 shipped only the
//! shared `Needs` scanner — but this skeleton stays deliberately tiny
//! (no typed inline handlers in its synthetic file set) rather than
//! also implementing all ~50 leaves; see `ciac-backend-python`/
//! `-rust`'s own `lower.rs` for two worked implementations, and
//! `ciac_codegen::lower::{IdentitySyntax, IdentitySyntaxStatement}`
//! for the contract's own minimal reference implementation. Per-item
//! emission (one file per declared api/worker/job/consumer/channel/
//! resource/call-target) remains real, scoped follow-up work — M5's
//! `Emit` table still covers only the static/conditional-single-file
//! subset, and a real backend hand-writes that per-item loop today,
//! the same way `ciac-backend-python`/`-rust` do.

use ciac_codegen::emit::{self, Emit};
use ciac_codegen::model as context;
use ciac_codegen::{
    Backend, BackendError, DevCommands, GenOptions, GeneratedProject, RestartStyle, SimSupport,
    TargetInfo,
};
use ciac_ir::{Component, NormalizedIr};
use include_dir::{include_dir, Dir};

static TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

/// The whole standard file set this skeleton emits, as data (Pillar
/// 5): a build-file stand-in, a README, and AGENTS.md — the "always"
/// tier of a real backend's own emission table, with none of the
/// conditional/per-item rows a real target would add as it un-gates
/// `supports()` (walkthrough step 3 onward).
const EMIT: &[Emit] = &[
    Emit::always("README.md", "README.md.j2"),
    Emit::always("AGENTS.md", "AGENTS.md.j2"),
];

static TARGET_INFO: TargetInfo = TargetInfo {
    project_marker: "skeleton.marker",
    migrations_dir: "migrations",
    migration_filename: |seq, _slug| format!("{seq:04}_migration.sql"),
    validate: &[],
    ci_test_steps:
        "      # skeleton-internal has no real CI story; see a real backend's own steps\n",
    compose: ciac_codegen::compose::BackendComposeOpts {
        db_url_scheme: "",
        workers_command: "[]",
        mysql_url_scheme: "",
        sqlite_url_prefix: "",
        sqlite_url_suffix: "",
        data_mount: "",
    },
    dev: DevCommands {
        rebuild: &[],
        restart: RestartStyle::Restart,
    },
    source_extension: "skel",
    sim: SimSupport::None {
        reason: "skeleton-internal is a reference crate, not a real target",
    },
};

#[derive(Debug, Default)]
pub struct SkeletonBackend;

impl Backend for SkeletonBackend {
    fn id(&self) -> &'static str {
        "skeleton-internal"
    }

    fn description(&self) -> &'static str {
        "Reference skeleton for authoring a new in-process CIaC backend (not a real target)"
    }

    fn supports(&self, _component: &Component) -> bool {
        // Gated: a fresh copy of this skeleton supports nothing until
        // its author starts un-gating capabilities (walkthrough step
        // 3 onward) — this is the "everything compiles; `ciac
        // targets` lists the new id with its gated (empty) support
        // set" state the authoring guide describes as day one.
        false
    }

    fn target_info(&self) -> &'static TargetInfo {
        &TARGET_INFO
    }

    fn generate(
        &self,
        ir: &NormalizedIr,
        opts: &GenOptions,
    ) -> Result<GeneratedProject, BackendError> {
        let model = context::build_system(ir, opts);
        let env = ciac_codegen::template::environment(TEMPLATES.files().map(|f| {
            (
                f.path().to_str().expect("template names are utf-8"),
                f.contents_utf8().expect("templates are utf-8"),
            )
        }))?;

        let mut project = GeneratedProject::new();
        for ctx in &model.services {
            let prefix = if model.multi {
                format!("{}/", ctx.dir)
            } else {
                String::new()
            };
            emit::run(&env, ctx, EMIT, &prefix, &mut project)?;
        }
        Ok(project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The "one registry test" the plan's Pillar 5 names: proves the
    /// skeleton's `Emit` table actually renders, independent of
    /// `check_support` (a real author calls `generate()` directly like
    /// this while a fresh copy's `supports()` still refuses
    /// everything — the same gap the walkthrough's step 1 describes).
    #[test]
    fn emits_its_declared_file_set() {
        let src = "service Ping;\n";
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = ciac_diagnostics::Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        let ir = ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()));

        let backend = SkeletonBackend;
        let project = backend
            .generate(&ir, &GenOptions::default())
            .expect("skeleton generates");
        let paths: Vec<&str> = project.files().map(|(p, _)| p).collect();
        assert_eq!(paths, vec!["AGENTS.md", "README.md"]);
        let (_, readme) = project.files().find(|(p, _)| *p == "README.md").unwrap();
        assert!(readme.contains("Ping"), "{readme}");
    }

    #[test]
    fn supports_nothing_yet() {
        let backend = SkeletonBackend;
        assert!(!backend.supports(&Component::Api {
            name: "X".to_owned(),
            request: None,
            config: ciac_ir::ApiConfig {
                method: ciac_ir::HttpMethod::Get,
                path: Some("/x".to_owned()),
                scope: None,
            },
        }));
    }

    #[test]
    fn target_info_is_populated_and_gated() {
        let backend = SkeletonBackend;
        let info = backend.target_info();
        assert_eq!(info.project_marker, "skeleton.marker");
        assert!(matches!(info.sim, SimSupport::None { .. }));
        assert!(info.validate.is_empty());
    }
}
