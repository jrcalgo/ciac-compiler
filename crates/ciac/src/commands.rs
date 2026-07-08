use anyhow::{bail, Context, Result};
use ciac_codegen::evolution::{diff_records, snapshot_boundary_records, RecordSchema};
use ciac_codegen::manifest::{
    build_manifest, hash_bytes, load_manifest, manifest_path, write_manifest,
};
use ciac_codegen::migrations::{diff_schema, snapshot_schema, TableSchema};
use ciac_codegen::regen::{
    apply_regeneration, plan_regeneration, ApplyMode, RegenMode, RegenPlan, RegenStatus,
};
use ciac_codegen::{Backend, GenOptions, GeneratedProject};
use ciac_diagnostics::render::{AriadneRenderer, Render};
use ciac_diagnostics::{Diagnostics, ErrorCode, SourceMap};
use ciac_ir::NormalizedIr;
use std::collections::BTreeMap;
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

/// Runs the front-end (module resolution + parse + analyze) on an entry
/// source file, printing all diagnostics. Returns the IR when the
/// program is valid, plus the full resolved [`SourceMap`] (every file
/// `import "path";` pulled in, v0.8 M1) so callers that need the whole
/// source set (e.g. hashing it for the manifest) don't re-resolve it.
fn front_end(file: &Path) -> Result<(Option<NormalizedIr>, bool, SourceMap)> {
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::load(file, &mut sources, &mut diags)
        .with_context(|| format!("cannot read {}", file.display()))?;
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
    Ok((ir, has_errors, sources))
}

pub fn check(file: &Path) -> Result<ExitCode> {
    let (ir, has_errors, _sources) = front_end(file)?;
    if has_errors || ir.is_none() {
        return Ok(ExitCode::FAILURE);
    }
    eprintln!("{}: no errors", file.display());
    Ok(ExitCode::SUCCESS)
}

/// `--deploy k8s` and its two image-naming knobs, bundled since they
/// only ever travel together from `ciac build`'s CLI args.
pub struct DeployOpts {
    pub deploy: Option<String>,
    pub image_prefix: Option<String>,
    pub image_tag: String,
}

pub fn build(
    file: &Path,
    target: &str,
    out: &Path,
    force: bool,
    adopt: bool,
    deploy: DeployOpts,
    name: Option<String>,
) -> Result<ExitCode> {
    if force && adopt {
        bail!("--force and --adopt cannot be used together");
    }
    let k8s_image = match deploy.deploy.as_deref() {
        Some("k8s") => Some((deploy.image_prefix.as_deref(), deploy.image_tag.as_str())),
        Some(other) => bail!("unknown --deploy target `{other}`; available: k8s"),
        None => None,
    };

    let Generated {
        backend,
        project,
        source_hash,
        tables,
        next_migration_seq,
        records,
    } = generate(file, target, out, name, k8s_image)?;

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
        let mut manifest = build_manifest(
            &project,
            env!("CARGO_PKG_VERSION"),
            source_hash,
            backend.id(),
        );
        manifest.tables = tables;
        manifest.next_migration_seq = next_migration_seq;
        manifest.records = records;
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
        let mut manifest = build_manifest(
            &project,
            env!("CARGO_PKG_VERSION"),
            source_hash,
            backend.id(),
        );
        manifest.tables = tables;
        manifest.next_migration_seq = next_migration_seq;
        manifest.records = records;
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
    let Generated { project, .. } = generate(file, target, out, name, None)?;
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
    system: bool,
    name: Option<String>,
) -> Result<ExitCode> {
    let Generated {
        backend,
        project,
        source_hash,
        tables,
        next_migration_seq,
        records,
    } = generate(file, target, out, name, None)?;
    if live {
        eprintln!("warning: --live health probing is not implemented yet; running static verify");
    }

    if !output_dir_nonempty(out)? {
        project
            .write_to(out)
            .with_context(|| format!("cannot write initial project to {}", out.display()))?;
        let mut manifest = build_manifest(
            &project,
            env!("CARGO_PKG_VERSION"),
            source_hash,
            backend.id(),
        );
        manifest.tables = tables;
        manifest.next_migration_seq = next_migration_seq;
        manifest.records = records;
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
            .filter(|entry| {
                entry.status != RegenStatus::Unchanged && entry.status != RegenStatus::OrphanLeft
            })
            .collect();
        if !drift.is_empty() {
            for entry in drift {
                eprintln!("drift: {} {}", entry.status.as_str(), entry.path);
            }
            return Ok(ExitCode::FAILURE);
        }
    }

    let static_result = validate_generated(out, backend.id())?;
    if static_result != ExitCode::SUCCESS || !system {
        return Ok(static_result);
    }
    verify_system(out)
}

/// v0.8 M4: runs the compose-backed `tests/system/` suite
/// (`ciac_codegen::system_tests`) `ciac verify --system` asks for. A
/// no-op success when the program had no whole-system edges to test
/// (`tests/system/` was never generated). Teardown always runs,
/// regardless of whether `up` or the test run itself failed, so a
/// failing `--system` run never leaves containers behind.
fn verify_system(out: &Path) -> Result<ExitCode> {
    let system_dir = out.join("tests/system");
    if !system_dir.exists() {
        eprintln!("info: no system-level edges to test; `--system` is a no-op here");
        return Ok(ExitCode::SUCCESS);
    }

    let compose_file = out.join("docker-compose.yml");
    let up_result = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(&compose_file)
        .args(["up", "-d", "--wait"])
        .status();

    let result: Result<()> = match up_result {
        Ok(status) if status.success() => run_in(&system_dir, "uv", &["sync", "-q"])
            .and_then(|_| run_in(&system_dir, "uv", &["run", "pytest", "-q"])),
        Ok(status) => Err(anyhow::anyhow!("docker compose up failed ({status})")),
        Err(err) => Err(anyhow::anyhow!(
            "cannot run `docker compose` ({err}); `ciac verify --system` requires Docker"
        )),
    };

    let _ = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(&compose_file)
        .args(["down", "-v", "--remove-orphans"])
        .status();

    match result {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(err) => {
            eprintln!("error: system verification failed: {err:#}");
            Ok(ExitCode::FAILURE)
        }
    }
}

pub fn graph(file: &Path, format: &str) -> Result<ExitCode> {
    let (ir, has_errors, _sources) = front_end(file)?;
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

/// v0.8 external-backend protocol M1: dumps the wire contract a
/// `ciac-backend-<target>` executable would receive on stdin, without
/// running any backend (no such executable exists yet — this is the
/// request half only, for inspection). Read-only, like `graph`: no
/// file writes, no manifest.
pub fn codegen_request(file: &Path, target: &str, name: Option<String>) -> Result<ExitCode> {
    let (ir, has_errors, _sources) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        return Ok(ExitCode::FAILURE);
    };
    let opts = GenOptions { project_name: name };
    let system = ciac_codegen::model::build_system(&ir, &opts);
    let request = ciac_codegen::protocol::CodegenRequest::new(target, opts.project_name, system);
    println!("{}", serde_json::to_string_pretty(&request)?);
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

/// Result of [`generate`]: the backend used, the generated project (with
/// any new migration file already injected), the source hash, and the
/// table/record schema state callers must stamp onto the manifest they
/// write (`tables` as of this build, `next_migration_seq` for the *next*
/// one, `records` as of this build).
struct Generated {
    backend: Box<dyn Backend>,
    project: GeneratedProject,
    source_hash: String,
    tables: BTreeMap<String, TableSchema>,
    next_migration_seq: u32,
    records: BTreeMap<String, RecordSchema>,
}

fn generate(
    file: &Path,
    target: &str,
    out: &Path,
    name: Option<String>,
    k8s_image: Option<(Option<&str>, &str)>,
) -> Result<Generated> {
    let all = backends();
    let backend: Box<dyn Backend> = match all.into_iter().find(|b| b.id() == target) {
        Some(backend) => backend,
        // v0.8 external-backend protocol M2: no built-in target
        // matches, so try `ciac-backend-<target>` on $PATH before
        // giving up — `ExternalBackend`'s own spawn error becomes the
        // final "unknown target" message if nothing's actually there
        // (see `ciac_codegen::external::spawn_error`), so there's no
        // separate existence check (and no TOCTOU race) here.
        None => Box::new(ciac_codegen::external::ExternalBackend::new(target)),
    };

    let (ir, has_errors, sources) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        bail!("front-end failed");
    };
    // Hashes the whole resolved source set (entry file plus every
    // transitively `import`ed file, v0.8 M1), not just the entry file's
    // bytes — so the manifest's `source_hash` changes when an imported
    // file changes too, even if the entry file itself didn't.
    let source_hash = {
        let mut buf = String::new();
        for file in sources.files() {
            buf.push_str(&file.name);
            buf.push('\0');
            buf.push_str(&file.src);
            buf.push('\0');
        }
        hash_bytes(buf.as_bytes())
    };

    if let Err(err) = ciac_codegen::check_support(backend.as_ref(), &ir) {
        eprintln!("error[{}]: {err}", ErrorCode::UnsupportedConstruct);
        bail!("unsupported construct");
    }

    let opts = GenOptions { project_name: name };
    let mut project = backend.generate(&ir, &opts)?;

    // v0.8 M4: compose-backed system tests, added the same way regardless
    // of target — they exercise wire-level contracts (HTTP/NATS/WS), not
    // target-language ones, so there's exactly one generator, not one per
    // backend. `ciac verify --system` runs them; plain `ciac verify` never
    // does (see `find_project_dirs`'s exclusion of `tests/system`).
    if let Some(files) = ciac_codegen::system_tests::build(&ir) {
        for (path, content) in files {
            project.add_file(format!("tests/system/{path}"), content);
        }
    }

    // v0.8 M6: opt-in Kubernetes manifests (`ciac build --deploy k8s`).
    // Compose remains the dev default and is always emitted by each
    // backend above; this is additive production deployment posture.
    if let Some((image_prefix, image_tag)) = k8s_image {
        for (path, content) in ciac_codegen::k8s::build(&ir, image_prefix, image_tag) {
            project.add_file(path, content);
        }
    }

    let manifest_file = manifest_path(out);
    let previous = if manifest_file.exists() {
        Some(load_manifest(out).with_context(|| {
            format!(
                "cannot read regeneration manifest {}",
                manifest_file.display()
            )
        })?)
    } else {
        None
    };
    let old_tables = previous
        .as_ref()
        .map(|m| m.tables.clone())
        .unwrap_or_default();
    let next_migration_seq = previous.as_ref().map_or(1, |m| m.next_migration_seq);
    let new_tables = snapshot_schema(&ir);

    let next_migration_seq = match diff_schema(&old_tables, &new_tables) {
        Ok(None) => next_migration_seq,
        Ok(Some(sql)) => {
            add_migration_files(&mut project, backend.id(), next_migration_seq, &sql);
            next_migration_seq + 1
        }
        Err(change) => {
            eprintln!("error[{}]: {change}", ErrorCode::UnsupportedSchemaChange);
            bail!("unsupported schema change");
        }
    };

    // v0.8 M5: a record used across a service boundary (a `call`
    // payload, or a stream published in one service and consumed in
    // another) must stay backward compatible between builds, since the
    // two services redeploy independently.
    let old_records = previous.map(|m| m.records).unwrap_or_default();
    let new_records = snapshot_boundary_records(&ir);
    if let Err(changes) = diff_records(&old_records, &new_records, &ir) {
        for change in &changes {
            eprintln!("error[{}]: {change}", ErrorCode::BreakingRecordChange);
        }
        bail!("breaking record change");
    }

    Ok(Generated {
        backend,
        project,
        source_hash,
        tables: new_tables,
        next_migration_seq,
        records: new_records,
    })
}

/// Adds the diffed migration SQL, under each generated deployable
/// project's migration directory (there may be more than one in a
/// multi-service system; every service context already carries the
/// program's full table set, so each gets the same migration file).
/// Migration files are seeded: once written, later builds that stop
/// re-emitting a given sequence number leave the on-disk file alone
/// (`RegenStatus::OrphanLeft`) rather than deleting it.
fn add_migration_files(project: &mut GeneratedProject, target: &str, seq: u32, sql: &str) {
    let filename = format!("{seq:04}_migration.sql");
    let rel = match target {
        "python" => format!("app/migrations/{filename}"),
        _ => format!("migrations/{filename}"),
    };
    for prefix in service_roots(project, target) {
        project.add_seeded_file(format!("{prefix}{rel}"), sql.to_owned());
    }
}

/// The prefix (possibly empty) of each deployable project inside a
/// generated tree, identified by its marker file (`pyproject.toml` /
/// `Cargo.toml`) — one root for a single-service build, several for a
/// multi-service system.
fn service_roots(project: &GeneratedProject, target: &str) -> Vec<String> {
    let marker = match target {
        "python" => "pyproject.toml",
        _ => "Cargo.toml",
    };
    let mut roots: Vec<String> = project
        .files_with_roles()
        .filter_map(|(path, _, _)| path.strip_suffix(marker).map(str::to_owned))
        .collect();
    if roots.is_empty() {
        roots.push(String::new());
    }
    roots.sort();
    roots
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
                // v0.8 M4: `tests/system/` is a compose-backed project run
                // only by `ciac verify --system`, never by plain `ciac
                // verify`'s per-service walk (it has no infra during a
                // normal build/verify run, so trying to run its own
                // pytest suite here would always fail).
                if child.file_name().is_some_and(|name| name == "system")
                    && path.file_name().is_some_and(|name| name == "tests")
                {
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
