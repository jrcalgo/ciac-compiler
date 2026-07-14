//! v0.18 M4/M5: `ciac rename` — the CLI front door onto
//! `ciac_syntax::rename_index`'s whole-program resolver. Dry-run by
//! default; `--apply` stages every affected source file (sibling temp
//! write + backup + a recovery journal), re-verifies the edited
//! program compiles, and — for each `--out <tree>` given — replays that
//! tree's checked-in build recipe against the renamed source and
//! refuses the whole rename (source included) if the tree can't
//! regenerate safely. Only once every check passes does it commit:
//! rename the staged files into place, apply each `--out` tree's
//! regeneration, and remove the backups/journal.
//!
//! Deliberately out of scope for this milestone (18UpdatePlan.md Pillar
//! 6/7): recognizing a rename as an explicit identity in the migration
//! differ (emitting `ALTER ... RENAME` instead of drop+add) and the live
//! Postgres data-preservation proof that identity would enable — both
//! need surgery inside `ciac-codegen::migrations` deep enough to risk
//! v0.16's relational-migration work, so they're deferred rather than
//! rushed. JSON/MCP/LSP surfaces are v0.18 M7 (Pillar 8).

use anyhow::{bail, Context, Result};
use ciac_diagnostics::render::{AriadneRenderer, Render};
use ciac_diagnostics::{Diagnostics, SourceMap};
use ciac_syntax::module::load_with_origins;
use ciac_syntax::rename_index::{build_index, RenamePlan, ResolvedSymbol};
use serde::{Deserialize, Serialize};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[allow(clippy::too_many_arguments)]
pub fn rename(
    entry: &Path,
    target_file: Option<&Path>,
    line: Option<u32>,
    column: Option<u32>,
    to: Option<&str>,
    old: Option<&str>,
    new_name_positional: Option<&str>,
    out_roots: &[PathBuf],
    apply: bool,
) -> Result<ExitCode> {
    let position_mode = target_file.is_some() || line.is_some() || column.is_some() || to.is_some();
    let qualified_mode = old.is_some() || new_name_positional.is_some();
    if position_mode == qualified_mode {
        bail!(
            "specify exactly one form: `--file/--line/--column --to <name>` (position-based) \
             or `<Old> <New>` (qualified convenience form)"
        );
    }
    if !out_roots.is_empty() && !apply {
        bail!("--out only makes sense with --apply (a dry run never touches generated output)");
    }

    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let (program, origins) = load_with_origins(entry, &mut sources, &mut diags)
        .with_context(|| format!("cannot read {}", entry.display()))?;
    if !diags.is_empty() {
        render_diagnostics(&diags, &sources)?;
        return Ok(ExitCode::FAILURE);
    }
    let index = build_index(&program);

    let (resolved, new_name): (ResolvedSymbol, &str) = if position_mode {
        let (Some(tf), Some(line), Some(col), Some(new_name)) = (target_file, line, column, to)
        else {
            bail!("position-based rename needs all of --file, --line, --column, and --to");
        };
        let canonical = tf
            .canonicalize()
            .with_context(|| format!("cannot resolve {}", tf.display()))?;
        let canonical_str = canonical.display().to_string();
        let file_id = sources
            .files()
            .enumerate()
            .find(|(_, f)| f.name == canonical_str)
            .map(|(i, _)| ciac_diagnostics::FileId(i as u32))
            .with_context(|| {
                format!(
                    "{} is not part of {}'s resolved source set",
                    tf.display(),
                    entry.display()
                )
            })?;
        let offset = sources
            .file(file_id)
            .offset_of(line, col)
            .with_context(|| format!("{}:{line}:{col} is out of range", tf.display()))?;
        let hits = index.resolve_at(file_id, offset);
        match hits.len() {
            0 => bail!("no renamable symbol at {}:{line}:{col}", tf.display()),
            1 => (hits.into_iter().next().unwrap(), new_name),
            _ => bail!(
                "ambiguous position {}:{line}:{col}: matches {} symbols",
                tf.display(),
                hits.len()
            ),
        }
    } else {
        let (Some(old_name), Some(new_name)) = (old, new_name_positional) else {
            bail!("qualified rename needs both <Old> and <New>");
        };
        let hits = index.resolve_qualified(old_name);
        match hits.len() {
            0 => bail!("no symbol named `{old_name}`"),
            1 => (hits.into_iter().next().unwrap(), new_name),
            _ => {
                eprintln!("`{old_name}` is ambiguous; candidates:");
                for hit in &hits {
                    let (l, c) = sources.file(hit.def_span.file).line_col(hit.def_span.start);
                    eprintln!(
                        "  {} ({}) at {}:{l}:{c}",
                        hit.name,
                        hit.kind.label(),
                        sources.file(hit.def_span.file).name
                    );
                }
                bail!("re-run with --file/--line/--column to pick one");
            }
        }
    };

    let plan = match index.plan_rename(&origins, resolved.id, new_name) {
        Ok(plan) => plan,
        Err(err) => {
            eprintln!("error: {err}");
            return Ok(ExitCode::FAILURE);
        }
    };

    let site_count = plan
        .edits_by_file
        .iter()
        .map(|(_, f)| f.edits.len())
        .sum::<usize>();
    eprintln!(
        "rename {} `{}` -> `{}` ({} file{}, {} site{})",
        resolved.kind.label(),
        plan.old_name,
        plan.new_name,
        plan.edits_by_file.len(),
        if plan.edits_by_file.len() == 1 {
            ""
        } else {
            "s"
        },
        site_count,
        if site_count == 1 { "" } else { "s" },
    );
    for (file_id, fix) in &plan.edits_by_file {
        let file = sources.file(*file_id);
        eprintln!("  {}", file.name);
        for edit in &fix.edits {
            let (l, c) = file.line_col(edit.span.start);
            eprintln!("    {l}:{c}: `{}` -> `{}`", plan.old_name, edit.replacement);
        }
    }

    if !apply {
        eprintln!("(dry run — pass --apply to write these files)");
        return Ok(ExitCode::SUCCESS);
    }

    apply_plan(entry, &sources, &plan, out_roots)
}

/// See the module doc comment for the full transaction shape. Source
/// edits are staged (backup + atomic tmp-then-rename) and journaled
/// before any real file is touched; a compile-check failure or an
/// unsafe `--out` regeneration plan rolls every staged file back to its
/// backup. Once every check passes, each `--out` tree's regeneration is
/// applied and the transaction is closed (backups and journal removed).
fn apply_plan(
    entry: &Path,
    sources: &SourceMap,
    plan: &RenamePlan,
    out_roots: &[PathBuf],
) -> Result<ExitCode> {
    let journal_path = journal_path(entry);
    if journal_path.exists() {
        let journal: Journal = serde_json::from_slice(&std::fs::read(&journal_path)?)
            .with_context(|| format!("cannot read stale journal {}", journal_path.display()))?;
        bail!(
            "a previous `ciac rename --apply` did not finish cleanly (journal at {}); inspect \
             and manually restore from these backups before retrying, then delete the journal:\n{}",
            journal_path.display(),
            journal
                .entries
                .iter()
                .map(|e| format!("  {} (backup: {})", e.path, e.backup))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    let mut staged: Vec<(PathBuf, PathBuf, String)> = Vec::new(); // (real, backup, edited)
    let mut journal_entries = Vec::new();
    for (file_id, fix) in &plan.edits_by_file {
        let file = sources.file(*file_id);
        let path = PathBuf::from(&file.name);
        let edited = fix.apply(&file.src);
        let backup = sibling_path(&path, "ciac-rename-bak");
        std::fs::write(&backup, &file.src)
            .with_context(|| format!("cannot write backup {}", backup.display()))?;
        journal_entries.push(JournalEntry {
            path: path.display().to_string(),
            backup: backup.display().to_string(),
        });
        staged.push((path, backup, edited));
    }
    write_journal(&journal_path, &journal_entries)?;

    for (path, _backup, edited) in &staged {
        let tmp = sibling_path(path, "ciac-rename-tmp");
        std::fs::write(&tmp, edited).with_context(|| format!("cannot write {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("cannot replace {}", path.display()))?;
    }

    let mut verify_sources = SourceMap::new();
    let mut verify_diags = Diagnostics::new();
    let recompiled = ciac_syntax::module::load(entry, &mut verify_sources, &mut verify_diags)
        .with_context(|| format!("cannot re-read {}", entry.display()))?;
    if verify_diags.is_empty() {
        let _ = ciac_sema::analyze(&recompiled, &mut verify_diags);
    }
    verify_diags.sort();
    if verify_diags.has_errors() {
        rollback(&staged, &journal_path)?;
        eprintln!("error: the renamed program no longer compiles — rolled back");
        render_diagnostics(&verify_diags, &verify_sources)?;
        return Ok(ExitCode::FAILURE);
    }

    // --out conflict check (dry, no writes) before anything is committed.
    let mut recipes = Vec::new();
    for out in out_roots {
        let manifest = match ciac_codegen::manifest::load_manifest(out) {
            Ok(manifest) => manifest,
            Err(err) => {
                rollback(&staged, &journal_path)?;
                return Err(err)
                    .with_context(|| format!("cannot read manifest at {}", out.display()));
            }
        };
        let Some(recipe) = manifest.recipe else {
            rollback(&staged, &journal_path)?;
            bail!(
                "{} has a legacy manifest with no recorded build recipe; rename refuses to guess \
                 --target/--profile/--deploy for it (run `ciac build` on it once with this ciac \
                 to upgrade it)",
                out.display()
            );
        };
        match crate::commands::replay_recipe(entry, out, &recipe, false) {
            Ok(regen_plan) => {
                if regen_plan.has_errors() {
                    rollback(&staged, &journal_path)?;
                    eprintln!(
                        "error: {} cannot regenerate safely after this rename:",
                        out.display()
                    );
                    for e in &regen_plan.entries {
                        if e.status.as_str() != "unchanged" {
                            eprintln!("  {:13} {}", e.status.as_str(), e.path);
                        }
                    }
                    return Ok(ExitCode::FAILURE);
                }
                recipes.push((out.clone(), recipe));
            }
            Err(err) => {
                rollback(&staged, &journal_path)?;
                return Err(err);
            }
        }
    }

    // Every check passed: commit each `--out` tree's regeneration, then
    // scan its seeded files for the old name (informational only).
    for (out, recipe) in &recipes {
        crate::commands::replay_recipe(entry, out, recipe, true)?;
        let hits = scan_seeded_references(out, &plan.old_name)?;
        if !hits.is_empty() {
            eprintln!(
                "note: {} possible_reference site(s) to the old name `{}` in {}'s seeded files \
                 (manual_reconciliation_required — CIaC does not parse target-language source):",
                hits.len(),
                plan.old_name,
                out.display()
            );
            for (path, line, text) in &hits {
                eprintln!("  {path}:{line}: {text}");
            }
        }
    }

    for (_, backup, _) in &staged {
        std::fs::remove_file(backup).ok();
    }
    std::fs::remove_file(&journal_path).ok();
    eprintln!(
        "applied: {} file(s) written{}",
        staged.len(),
        if out_roots.is_empty() {
            String::new()
        } else {
            format!(", {} output tree(s) regenerated", out_roots.len())
        }
    );
    Ok(ExitCode::SUCCESS)
}

fn rollback(staged: &[(PathBuf, PathBuf, String)], journal_path: &Path) -> Result<()> {
    for (path, backup, _) in staged {
        let original = std::fs::read(backup)
            .with_context(|| format!("cannot read backup {}", backup.display()))?;
        std::fs::write(path, &original)
            .with_context(|| format!("cannot restore {}", path.display()))?;
        std::fs::remove_file(backup).ok();
    }
    std::fs::remove_file(journal_path).ok();
    Ok(())
}

/// Every seeded (owned-by-the-user) file under `out`, grepped for the
/// literal old-name text. Labeled `possible_reference` deliberately —
/// CIaC does not parse arbitrary Python/Rust/TypeScript/SQL, so this is
/// a heuristic surfaced for human review, never a compile gate.
fn scan_seeded_references(out: &Path, old_name: &str) -> Result<Vec<(String, u32, String)>> {
    let manifest = ciac_codegen::manifest::load_manifest(out)
        .with_context(|| format!("cannot read manifest at {}", out.display()))?;
    let mut hits = Vec::new();
    for (path, file) in &manifest.files {
        if !matches!(file.role, ciac_codegen::FileRole::Seeded) {
            continue;
        }
        let full = out.join(path);
        let Ok(text) = std::fs::read_to_string(&full) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if line.contains(old_name) {
                hits.push((path.clone(), (i + 1) as u32, line.trim().to_owned()));
            }
        }
    }
    Ok(hits)
}

fn journal_path(entry: &Path) -> PathBuf {
    entry
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(".ciac")
        .join("rename-journal.json")
}

#[derive(Serialize, Deserialize, Clone)]
struct JournalEntry {
    path: String,
    backup: String,
}

#[derive(Serialize, Deserialize)]
struct Journal {
    entries: Vec<JournalEntry>,
}

fn write_journal(path: &Path, entries: &[JournalEntry]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let journal = Journal {
        entries: entries.to_vec(),
    };
    std::fs::write(path, serde_json::to_vec_pretty(&journal)?)
        .with_context(|| format!("cannot write journal {}", path.display()))
}

/// A same-directory sibling path with an extra suffix appended to the
/// file name (not the extension) — `order.ciac` -> `order.ciac.<suffix>`
/// — so the atomic tmp-then-rename and the backup always live on the
/// same filesystem as the real file, whatever its own extension is.
fn sibling_path(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".");
    name.push(suffix);
    path.with_file_name(name)
}

fn render_diagnostics(diags: &Diagnostics, sources: &SourceMap) -> Result<()> {
    let renderer = AriadneRenderer {
        color: std::io::stderr().is_terminal(),
    };
    let mut stderr = std::io::stderr().lock();
    for diag in diags.iter() {
        renderer.render(diag, sources, &mut stderr)?;
    }
    Ok(())
}
