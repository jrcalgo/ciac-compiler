use anyhow::{bail, Context, Result};
use ciac_codegen::manifest::{
    build_manifest, hash_bytes, load_manifest, manifest_path, write_manifest,
};
use ciac_codegen::regen::{
    apply_regeneration, plan_regeneration, ApplyMode, RegenMode, RegenPlan, RegenStatus,
};
use ciac_codegen::{Backend, GenOptions, GeneratedProject};
use ciac_diagnostics::render::{AriadneRenderer, Render};
use ciac_diagnostics::{Diagnostics, ErrorCode, SourceMap};
use ciac_ir::NormalizedIr;
use std::io::IsTerminal;
use std::path::Path;
use std::process::{Command, ExitCode};

/// All registered code-generation backends, in stable order.
/// Adding a target is one line here plus the backend crate itself.
fn backends() -> Vec<Box<dyn Backend>> {
    vec![
        Box::new(ciac_backend_python::PythonBackend),
        Box::new(ciac_backend_rust::RustBackend),
    ]
}

/// Runs the front-end (parse + analyze) on a source file, printing all
/// diagnostics. Returns the IR when the program is valid.
fn front_end(file: &Path) -> Result<(Option<NormalizedIr>, bool)> {
    let src =
        std::fs::read_to_string(file).with_context(|| format!("cannot read {}", file.display()))?;
    let mut sources = SourceMap::new();
    let file_id = sources.add_file(file.display().to_string(), src.clone());
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::parse(&src, file_id, &mut diags);
    let ir = ciac_sema::analyze(&program, &mut diags);

    diags.sort();
    let has_errors = diags.has_errors();
    let renderer = AriadneRenderer {
        color: std::io::stderr().is_terminal(),
    };
    let mut stderr = std::io::stderr().lock();
    for diag in diags.iter() {
        renderer.render(diag, &sources, &mut stderr)?;
    }
    Ok((ir, has_errors))
}

pub fn check(file: &Path) -> Result<ExitCode> {
    let (ir, has_errors) = front_end(file)?;
    if has_errors || ir.is_none() {
        return Ok(ExitCode::FAILURE);
    }
    eprintln!("{}: no errors", file.display());
    Ok(ExitCode::SUCCESS)
}

pub fn build(
    file: &Path,
    target: &str,
    out: &Path,
    force: bool,
    adopt: bool,
    name: Option<String>,
) -> Result<ExitCode> {
    if force && adopt {
        bail!("--force and --adopt cannot be used together");
    }

    let (backend, project, source_hash) = generate(file, target, name)?;

    if force {
        if out.exists() {
            let clobbered = list_files(out)?;
            for path in clobbered {
                eprintln!("clobber: {}", path.display());
            }
            std::fs::remove_dir_all(out)
                .with_context(|| format!("cannot clear {}", out.display()))?;
        }
        project
            .write_to(out)
            .with_context(|| format!("cannot write to {}", out.display()))?;
        let manifest = build_manifest(
            &project,
            env!("CARGO_PKG_VERSION"),
            source_hash,
            backend.id(),
        );
        write_manifest(out, &manifest)
            .with_context(|| format!("cannot write manifest in {}", out.display()))?;
    } else {
        let manifest_file = manifest_path(out);
        let manifest = if manifest_file.exists() {
            Some(load_manifest(out).with_context(|| {
                format!(
                    "cannot read regeneration manifest {}",
                    manifest_file.display()
                )
            })?)
        } else {
            None
        };

        if manifest.is_none() && output_dir_nonempty(out)? && !adopt {
            eprintln!(
                "error[{}]: output directory {} has no regeneration manifest (pass --adopt to preserve existing files, or choose a clean directory)",
                ErrorCode::MissingManifest,
                out.display()
            );
            return Ok(ExitCode::FAILURE);
        }

        let mode = if adopt {
            RegenMode::Adopt
        } else {
            RegenMode::Normal
        };
        let plan = plan_regeneration(&project, out, manifest.as_ref(), mode)
            .with_context(|| format!("cannot compare generated files with {}", out.display()))?;
        if plan.has_errors() && !adopt {
            apply_regeneration(&plan, out, ApplyMode::SidecarsOnly).with_context(|| {
                format!("cannot write regeneration sidecars in {}", out.display())
            })?;
            report_regen_plan(&plan, adopt, true);
            return Ok(ExitCode::FAILURE);
        }
        apply_regeneration(&plan, out, ApplyMode::Full)
            .with_context(|| format!("cannot apply regeneration to {}", out.display()))?;
        report_regen_plan(&plan, adopt, true);
        let manifest = build_manifest(
            &project,
            env!("CARGO_PKG_VERSION"),
            source_hash,
            backend.id(),
        );
        write_manifest(out, &manifest)
            .with_context(|| format!("cannot write manifest in {}", out.display()))?;
    }

    eprintln!(
        "generated {} files in {} ({} backend)",
        project.len(),
        out.display(),
        backend.id()
    );
    for note in &project.notes {
        eprintln!("note: {note}");
    }
    Ok(ExitCode::SUCCESS)
}

pub fn diff(
    file: &Path,
    target: &str,
    out: &Path,
    patch: bool,
    name: Option<String>,
) -> Result<ExitCode> {
    let (_backend, project, _source_hash) = generate(file, target, name)?;
    let manifest_file = manifest_path(out);
    let manifest = if manifest_file.exists() {
        Some(load_manifest(out).with_context(|| {
            format!(
                "cannot read regeneration manifest {}",
                manifest_file.display()
            )
        })?)
    } else {
        None
    };
    let plan = plan_regeneration(&project, out, manifest.as_ref(), RegenMode::Normal)
        .with_context(|| format!("cannot compare generated files with {}", out.display()))?;
    for entry in &plan.entries {
        println!("{:13} {}", entry.status.as_str(), entry.path);
        if let Some(sidecar) = &entry.sidecar_path {
            println!("{:13} {}", "sidecar", sidecar);
        }
        if patch {
            print_patch(entry);
        }
    }
    if plan.has_errors() {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

pub fn verify(
    file: &Path,
    target: &str,
    out: &Path,
    live: bool,
    name: Option<String>,
) -> Result<ExitCode> {
    let (backend, project, source_hash) = generate(file, target, name)?;
    if live {
        eprintln!("warning: --live health probing is not implemented yet; running static verify");
    }

    if !output_dir_nonempty(out)? {
        project
            .write_to(out)
            .with_context(|| format!("cannot write initial project to {}", out.display()))?;
        let manifest = build_manifest(
            &project,
            env!("CARGO_PKG_VERSION"),
            source_hash,
            backend.id(),
        );
        write_manifest(out, &manifest)
            .with_context(|| format!("cannot write manifest in {}", out.display()))?;
    } else {
        let manifest_file = manifest_path(out);
        if !manifest_file.exists() {
            eprintln!(
                "error[{}]: output directory {} has no regeneration manifest",
                ErrorCode::MissingManifest,
                out.display()
            );
            return Ok(ExitCode::FAILURE);
        }
        let manifest = load_manifest(out).with_context(|| {
            format!(
                "cannot read regeneration manifest {}",
                manifest_file.display()
            )
        })?;
        let plan = plan_regeneration(&project, out, Some(&manifest), RegenMode::Normal)
            .with_context(|| format!("cannot compare generated files with {}", out.display()))?;
        report_regen_plan(&plan, false, false);
        let drift: Vec<_> = plan
            .entries
            .iter()
            .filter(|entry| entry.status != RegenStatus::Unchanged)
            .collect();
        if !drift.is_empty() {
            for entry in drift {
                eprintln!("drift: {} {}", entry.status.as_str(), entry.path);
            }
            return Ok(ExitCode::FAILURE);
        }
    }

    validate_generated(out, backend.id())
}

pub fn graph(file: &Path, format: &str) -> Result<ExitCode> {
    let (ir, has_errors) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        return Ok(ExitCode::FAILURE);
    };
    match format {
        "json" => println!("{}", serde_json::to_string_pretty(&ir)?),
        "dot" => print!("{}", ir.to_dot()),
        other => bail!("unknown format `{other}`"),
    }
    Ok(ExitCode::SUCCESS)
}

pub fn explain(code: &str) -> Result<ExitCode> {
    let Some(code) = ErrorCode::parse(code) else {
        bail!("unknown error code `{code}`; codes look like CIAC0001 (see docs/errors.md)");
    };
    println!("{}: {}", code.code(), code.title());
    println!();
    println!("{}", code.explanation());
    Ok(ExitCode::SUCCESS)
}

pub fn targets() -> Result<ExitCode> {
    for backend in backends() {
        println!("{:10} {}", backend.id(), backend.description());
    }
    Ok(ExitCode::SUCCESS)
}

fn generate(
    file: &Path,
    target: &str,
    name: Option<String>,
) -> Result<(Box<dyn Backend>, GeneratedProject, String)> {
    let all = backends();
    let Some(index) = all.iter().position(|b| b.id() == target) else {
        let known: Vec<&str> = all.iter().map(|b| b.id()).collect();
        bail!("unknown target `{target}`; available: {}", known.join(", "));
    };
    let backend = all.into_iter().nth(index).expect("index was found");

    let source = std::fs::read(file).with_context(|| format!("cannot read {}", file.display()))?;
    let source_hash = hash_bytes(&source);
    let (ir, has_errors) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        bail!("front-end failed");
    };

    if let Err(err) = ciac_codegen::check_support(backend.as_ref(), &ir) {
        eprintln!("error[{}]: {err}", ErrorCode::UnsupportedConstruct);
        bail!("unsupported construct");
    }

    let opts = GenOptions { project_name: name };
    let project = backend.generate(&ir, &opts)?;
    Ok((backend, project, source_hash))
}

fn output_dir_nonempty(out: &Path) -> Result<bool> {
    if !out.exists() {
        return Ok(false);
    }
    Ok(out
        .read_dir()
        .with_context(|| format!("cannot read {}", out.display()))?
        .next()
        .is_some())
}

/// Reports a regeneration plan. `sidecars_written` tells the truth about
/// whether `.ciac-new` sidecars were actually written to disk for this
/// call: `build` writes them (in both the successful and the failed-build
/// path, see `ApplyMode`), but `verify` never writes anything.
fn report_regen_plan(plan: &RegenPlan, adopt: bool, sidecars_written: bool) {
    for entry in &plan.entries {
        let sidecar = entry.sidecar_path.as_deref().unwrap_or("<sidecar>");
        match entry.status {
            RegenStatus::Conflict if adopt => {
                eprintln!(
                    "warning[{}]: preserving existing file {}; generated content written to {}",
                    ErrorCode::RegenerationConflict,
                    entry.path,
                    sidecar
                );
            }
            RegenStatus::Conflict if sidecars_written => {
                eprintln!(
                    "error[{}]: compiler-owned file {} was modified; generated content written to {}",
                    ErrorCode::RegenerationConflict,
                    entry.path,
                    sidecar
                );
            }
            RegenStatus::Conflict => {
                eprintln!(
                    "error[{}]: compiler-owned file {} differs from the generated content; run `ciac build` to write {}",
                    ErrorCode::RegenerationConflict,
                    entry.path,
                    sidecar
                );
            }
            RegenStatus::SeededDrift if sidecars_written => {
                eprintln!(
                    "warning[{}]: seeded file {} changed; generated seed written to {}",
                    ErrorCode::SeededFileDrift,
                    entry.path,
                    sidecar
                );
            }
            RegenStatus::SeededDrift => {
                eprintln!(
                    "warning[{}]: seeded file {} changed; run `ciac build` to write the generated seed to {}",
                    ErrorCode::SeededFileDrift,
                    entry.path,
                    sidecar
                );
            }
            RegenStatus::OrphanLeft => {
                eprintln!(
                    "warning[{}]: generated file {} is no longer produced and was left in place",
                    ErrorCode::OrphanedGeneratedFile,
                    entry.path
                );
            }
            _ => {}
        }
    }
}

fn print_patch(entry: &ciac_codegen::regen::RegenEntry) {
    let old = entry.old_content.as_deref().unwrap_or_default();
    let new = entry.new_content.as_deref().unwrap_or_default();
    if old == new {
        return;
    }
    let diff = similar::TextDiff::from_lines(old, new);
    print!(
        "{}",
        diff.unified_diff()
            .header(&format!("a/{}", entry.path), &format!("b/{}", entry.path))
    );
}

fn list_files(root: &Path) -> Result<Vec<std::path::PathBuf>> {
    fn walk(path: &Path, files: &mut Vec<std::path::PathBuf>) -> Result<()> {
        if path.is_dir() {
            for entry in std::fs::read_dir(path)
                .with_context(|| format!("cannot read {}", path.display()))?
            {
                walk(&entry?.path(), files)?;
            }
        } else {
            files.push(path.to_path_buf());
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn validate_generated(root: &Path, target: &str) -> Result<ExitCode> {
    let marker = match target {
        "python" => "pyproject.toml",
        "rust" => "Cargo.toml",
        other => bail!("cannot verify unknown generated target `{other}`"),
    };
    let projects = find_project_dirs(root, marker)?;
    if projects.is_empty() {
        bail!(
            "no generated {target} project found under {}",
            root.display()
        );
    }
    for project in projects {
        match target {
            "python" => validate_python_project(&project)?,
            "rust" => validate_rust_project(&project)?,
            _ => unreachable!("matched above"),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn validate_python_project(project: &Path) -> Result<()> {
    run_in(project, "uv", &["sync", "-q"])?;
    run_in(project, "uv", &["run", "ruff", "check", "."])?;
    run_in(project, "uv", &["run", "pytest", "-q"])
}

fn validate_rust_project(project: &Path) -> Result<()> {
    let status = Command::new("cargo")
        .arg("check")
        .env("RUSTFLAGS", "-D warnings")
        .current_dir(project)
        .status()
        .with_context(|| format!("failed to run cargo check in {}", project.display()))?;
    if !status.success() {
        bail!("cargo check failed in {}", project.display());
    }
    run_in(project, "cargo", &["test", "-q", "--lib"])
}

fn run_in(project: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status = Command::new(program)
        .args(args)
        .current_dir(project)
        .status()
        .with_context(|| {
            format!(
                "failed to run `{program} {}` in {}",
                args.join(" "),
                project.display()
            )
        })?;
    if !status.success() {
        bail!(
            "`{program} {}` failed in {}",
            args.join(" "),
            project.display()
        );
    }
    Ok(())
}

fn find_project_dirs(root: &Path, marker: &str) -> Result<Vec<std::path::PathBuf>> {
    fn walk(path: &Path, marker: &str, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
        if path.join(marker).is_file() {
            out.push(path.to_path_buf());
            return Ok(());
        }
        if path.is_dir() {
            for entry in std::fs::read_dir(path)
                .with_context(|| format!("cannot read {}", path.display()))?
            {
                let entry = entry?;
                let child = entry.path();
                if child.file_name().is_some_and(|name| {
                    name == ".git" || name == "target" || name == ".venv" || name == "__pycache__"
                }) {
                    continue;
                }
                walk(&child, marker, out)?;
            }
        }
        Ok(())
    }
    let mut projects = Vec::new();
    walk(root, marker, &mut projects)?;
    projects.sort();
    Ok(projects)
}
