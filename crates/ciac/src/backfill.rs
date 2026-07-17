//! v0.18 M6: `ciac backfill plan` — the CLI front door onto
//! `ciac_codegen::backfill`'s expand/backfill/contract ladder
//! (18UpdatePlan.md Pillar 5).
//!
//! The expand step needs no new command: an ordinary `ciac build`/
//! `ciac verify` on `--out` already writes and applies the safely
//! nullable `ALTER TABLE ... ADD COLUMN` migration for a required field
//! added to a table-backed record (`ciac-codegen::migrations::
//! diff_schema`'s existing additive-only behavior). This command's job
//! starts once that's landed: emit a seeded, target-native backfill
//! script for each such change, and — only when the caller passes
//! `--allow-destructive <plan-id>` for a plan already on record — the
//! contract migration that tightens the column to `NOT NULL`, guarded
//! so it refuses to apply until `_ciac_backfills` has a completed row
//! for that exact plan.
//!
//! `ciac` never runs the backfill script and never decides a plan is
//! complete; that's the seeded script's own job once a human has
//! filled in the real conversion and run it against the target
//! database.

use anyhow::{bail, Context, Result};
use ciac_codegen::backfill::{contract_sql, plan_backfills, seeded_script, BackfillPlan};
use ciac_codegen::manifest::{load_manifest, write_manifest};
use ciac_codegen::migrations::snapshot_schema;
use std::path::Path;
use std::process::ExitCode;

pub fn plan(
    file: &Path,
    baseline: Option<&Path>,
    out: &Path,
    allow_destructive: Option<&str>,
) -> Result<ExitCode> {
    let (ir, has_errors, _sources) = crate::commands::front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        bail!("front-end failed");
    };
    let current = ciac_codegen::semantic_model::SemanticModel::from_ir(&ir);

    let baseline_model = match crate::commands::load_comparison_baseline(file, None, baseline)? {
        Ok(model) => model,
        Err(msg) => {
            eprintln!("error: {msg}");
            return Ok(ExitCode::FAILURE);
        }
    };

    let mut manifest = load_manifest(out)
        .with_context(|| format!("cannot read regeneration manifest at {}", out.display()))?;
    let Some(recipe) = &manifest.recipe else {
        bail!(
            "{} has a legacy manifest with no recorded build recipe; rebuild it once with this \
             ciac to upgrade it before planning a backfill against it",
            out.display()
        );
    };
    let target = recipe.target.clone();

    let current_tables = snapshot_schema(&ir);
    let built_tables = manifest.tables.clone();

    let plans = plan_backfills(&baseline_model, &current, &current_tables, &built_tables);
    if plans.is_empty() {
        eprintln!(
            "no backfill-eligible changes between {} and the current program",
            {
                baseline
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "the checked-in baseline".to_owned())
            }
        );
        return Ok(ExitCode::SUCCESS);
    }

    let mut any_blocked = false;
    let mut wrote_contract = false;
    for result in &plans {
        let plan = match result {
            Ok(plan) => plan,
            Err(err) => {
                eprintln!("error: {err}");
                any_blocked = true;
                continue;
            }
        };

        let plan_dir = out.join(".ciac").join("backfills");
        std::fs::create_dir_all(&plan_dir)
            .with_context(|| format!("cannot create {}", plan_dir.display()))?;
        let plan_file = plan_dir.join(format!("{}.json", plan.plan_id));
        std::fs::write(&plan_file, serde_json::to_vec_pretty(plan)?)
            .with_context(|| format!("cannot write {}", plan_file.display()))?;

        let (script_filename, script_src) = seeded_script(plan, &target);
        let script_path = out.join(migrations_dir(&target)).join(&script_filename);
        if let Some(parent) = script_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let script_is_new = !script_path.exists();
        if script_is_new {
            std::fs::write(&script_path, &script_src)
                .with_context(|| format!("cannot write {}", script_path.display()))?;
        }

        eprintln!(
            "plan {}: {}.{} ({})",
            plan.plan_id, plan.table, plan.column, plan.message
        );
        eprintln!(
            "  backfill script: {} ({})",
            script_path.display(),
            if script_is_new {
                "written"
            } else {
                "already present, left as-is"
            }
        );
        eprintln!("  plan record: {}", plan_file.display());

        if allow_destructive == Some(plan.plan_id.as_str()) {
            write_contract_migration(out, &mut manifest, plan)?;
            wrote_contract = true;
        } else {
            eprintln!(
                "  contract migration: withheld — re-run with --allow-destructive {} once the \
                 backfill script above has been run against the target database",
                plan.plan_id
            );
        }
    }

    if wrote_contract {
        write_manifest(out, &manifest)
            .with_context(|| format!("cannot write manifest in {}", out.display()))?;
    }

    Ok(if any_blocked {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// The relative migrations directory this target's project actually
/// uses — mirrors `commands::add_migration_files`'s own convention so a
/// backfill script or contract migration lands next to the expand
/// migration `ciac build` already wrote. Resolved through the registry
/// (v0.22 M1 — `TargetInfo::migrations_dir`); an unregistered/external
/// target still falls back to `"migrations"`, same as the old `_ =>`
/// arm.
fn migrations_dir(target: &str) -> &'static str {
    crate::commands::backends()
        .into_iter()
        .find(|b| b.id() == target)
        .map(|b| b.target_info().migrations_dir)
        .unwrap_or("migrations")
}

fn write_contract_migration(
    out: &Path,
    manifest: &mut ciac_codegen::manifest::Manifest,
    plan: &BackfillPlan,
) -> Result<()> {
    let target = manifest
        .recipe
        .as_ref()
        .map(|r| r.target.as_str())
        .unwrap_or("");
    let dir = out.join(migrations_dir(target));
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;

    // A contract migration for this exact plan may already exist under
    // an earlier sequence number — the check has to be "does any file
    // for this plan_id exist", not "does the next-seq path happen to be
    // free" (the next seq is always free; that's not what matters).
    let suffix = format!("_contract_{}.sql", plan.plan_id);
    if let Some(existing) = std::fs::read_dir(&dir)
        .with_context(|| format!("cannot read {}", dir.display()))?
        .filter_map(|e| e.ok())
        .find(|e| e.file_name().to_string_lossy().ends_with(&suffix))
    {
        bail!(
            "{} already exists; a contract migration for plan {} was already written",
            existing.path().display(),
            plan.plan_id
        );
    }

    let seq = manifest.next_migration_seq;
    let filename = format!("{seq:04}{suffix}");
    let path = dir.join(&filename);
    std::fs::write(&path, contract_sql(plan))
        .with_context(|| format!("cannot write {}", path.display()))?;
    manifest.next_migration_seq = seq + 1;
    eprintln!(
        "  contract migration: {} (seq {seq:04}, guarded on plan {})",
        path.display(),
        plan.plan_id
    );
    Ok(())
}
