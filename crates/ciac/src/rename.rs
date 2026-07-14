//! v0.18 M4: `ciac rename` — the CLI front door onto
//! `ciac_syntax::rename_index`'s whole-program resolver. Dry-run by
//! default; `--apply` writes the affected source files directly and
//! re-verifies the edited program compiles, rolling back every touched
//! file on failure.
//!
//! Deliberately out of scope for this milestone (18UpdatePlan.md Pillar
//! 6/7, tracked as v0.18 M5): the transactional staging/journal/rollback
//! layer for crash safety across many files, `--out` participation
//! (replaying generated-output regeneration and emitting `ALTER ...
//! RENAME`), and scanning seeded files for stale old-name references.
//! `--apply` here is a direct, single-shot write-then-verify-then-
//! rollback-in-memory — safe against a failed *recompile*, not against
//! the process being killed mid-write across multiple files. JSON/MCP/
//! LSP surfaces are v0.18 M7 (Pillar 8).

use anyhow::{bail, Context, Result};
use ciac_diagnostics::render::{AriadneRenderer, Render};
use ciac_diagnostics::{Diagnostics, SourceMap};
use ciac_syntax::module::load_with_origins;
use ciac_syntax::rename_index::{build_index, ResolvedSymbol};
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
        plan.edits_by_file
            .iter()
            .map(|(_, f)| f.edits.len())
            .sum::<usize>(),
        if plan
            .edits_by_file
            .iter()
            .map(|(_, f)| f.edits.len())
            .sum::<usize>()
            == 1
        {
            ""
        } else {
            "s"
        },
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

    apply_plan(entry, &sources, &plan)
}

/// Writes every affected file's edited text directly (no staging/
/// journal — see module doc comment), then re-runs the full compiler
/// front end against the now-edited files. On failure, every touched
/// file is restored from the pre-edit snapshot already held in
/// `sources` and the diagnostics are rendered; the source tree is never
/// left in a half-renamed state on disk.
fn apply_plan(
    entry: &Path,
    sources: &SourceMap,
    plan: &ciac_syntax::rename_index::RenamePlan,
) -> Result<ExitCode> {
    let mut written: Vec<(PathBuf, String)> = Vec::new();
    for (file_id, fix) in &plan.edits_by_file {
        let file = sources.file(*file_id);
        let path = PathBuf::from(&file.name);
        let original = file.src.clone();
        let edited = fix.apply(&original);
        std::fs::write(&path, &edited)
            .with_context(|| format!("cannot write {}", path.display()))?;
        written.push((path, original));
    }

    let mut verify_sources = SourceMap::new();
    let mut verify_diags = Diagnostics::new();
    let recompiled = ciac_syntax::module::load(entry, &mut verify_sources, &mut verify_diags)
        .with_context(|| format!("cannot re-read {}", entry.display()))?;
    if verify_diags.is_empty() {
        let _ = ciac_sema::analyze(&recompiled, &mut verify_diags);
    }
    verify_diags.sort();

    if !verify_diags.has_errors() {
        eprintln!("applied: {} file(s) written", written.len());
        return Ok(ExitCode::SUCCESS);
    }

    eprintln!("error: the renamed program no longer compiles — rolling back");
    for (path, original) in &written {
        std::fs::write(path, original)
            .with_context(|| format!("cannot restore {}", path.display()))?;
    }
    render_diagnostics(&verify_diags, &verify_sources)?;
    Ok(ExitCode::FAILURE)
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
