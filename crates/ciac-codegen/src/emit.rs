//! The declarative emission-plan helper (v0.22 M5 — `22UpdatePlan.md`
//! Pillar 5): "which files exist under which condition" as data,
//! executed by one shared loop, instead of a hand-rolled sequence of
//! `project.add_file(..)` calls a backend author has to get right by
//! copying an existing one and hoping nothing was missed.
//!
//! Scope, disclosed: this milestone ships the mechanism and proves it
//! end-to-end in `backends/skeleton-internal/`, which emits its whole
//! (small, synthetic) file set through it. Porting the two *real*
//! backends' full emission sequences — each backend also has per-item
//! loops (one file per declared api/worker/job/consumer/channel/
//! resource/call-target), which this milestone's `Emit` shape doesn't
//! yet cover — is real, contained follow-up work, recorded in
//! `22UpdatePlan.md`'s M5 section rather than attempted here under
//! time pressure. The static/conditional-single-file subset (the
//! "standard file set" step 3 of the authoring walkthrough describes:
//! build file, Dockerfile, README, config, state, observability) is
//! exactly what `Emit` already models completely.

use crate::model::Ctx;
use crate::project::FileRole;
use crate::{BackendError, GeneratedProject};
use minijinja::Environment;

/// One row: a path (relative to the service's own prefix), the
/// template that renders it, its regeneration ownership role, and the
/// condition under which it's emitted at all. `cond` takes the shared,
/// language-neutral `Ctx` — the same context every backend already
/// builds from `ciac_codegen::model::build_system` — so a condition
/// like "this service declares auth" means the same thing regardless
/// of which backend's table it appears in.
#[derive(Debug)]
pub struct Emit {
    pub path: &'static str,
    pub template: &'static str,
    pub role: FileRole,
    pub cond: fn(&Ctx) -> bool,
}

impl Emit {
    /// Unconditional: every service gets this file.
    pub const fn always(path: &'static str, template: &'static str) -> Self {
        Emit {
            path,
            template,
            role: FileRole::Owned,
            cond: |_| true,
        }
    }

    /// Emitted only when `cond(ctx)` holds.
    pub const fn when(path: &'static str, template: &'static str, cond: fn(&Ctx) -> bool) -> Self {
        Emit {
            path,
            template,
            role: FileRole::Owned,
            cond,
        }
    }

    /// Like [`Emit::always`], but seeded (user-owned after first
    /// write) rather than compiler-owned.
    pub const fn seeded_always(path: &'static str, template: &'static str) -> Self {
        Emit {
            path,
            template,
            role: FileRole::Seeded,
            cond: |_| true,
        }
    }
}

/// Renders every row of `table` whose condition holds against `ctx`,
/// writing each under `prefix` (the service's own output subdirectory
/// in a multi-service system, `""` for single-service). Each template
/// renders with the same `c => ctx` binding every hand-written
/// `emit_service` already used, so porting a hand-rolled call site to
/// a table row changes nothing about what the template sees.
pub fn run(
    env: &Environment<'_>,
    ctx: &Ctx,
    table: &[Emit],
    prefix: &str,
    project: &mut GeneratedProject,
) -> Result<(), BackendError> {
    let base = minijinja::Value::from_serialize(ctx);
    for entry in table {
        if !(entry.cond)(ctx) {
            continue;
        }
        let content = env
            .get_template(entry.template)?
            .render(minijinja::context! { c => base })?;
        let path = format!("{prefix}{}", entry.path);
        match entry.role {
            FileRole::Owned => project.add_file(path, content),
            FileRole::Seeded => project.add_seeded_file(path, content),
        }
    }
    Ok(())
}
