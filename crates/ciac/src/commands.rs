use anyhow::{anyhow, bail, Context, Result};
use ciac_codegen::evolution::{diff_records, snapshot_boundary_records, RecordSchema};
use ciac_codegen::manifest::{
    build_manifest, hash_bytes, load_manifest, manifest_path, write_manifest,
};
use ciac_codegen::migrations::{diff_schema, snapshot_schema, TableSchema};
use ciac_codegen::regen::{
    apply_regeneration, plan_regeneration, ApplyMode, RegenMode, RegenPlan, RegenStatus,
};
use ciac_codegen::{Backend, GenOptions, GeneratedProject, SimSupport, ValidateStep};
use ciac_diagnostics::render::{AriadneRenderer, Render};
use ciac_diagnostics::{Diagnostics, ErrorCode, SourceMap};
use ciac_ir::{FieldType, NormalizedIr};
use heck::ToKebabCase;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// All registered code-generation backends, in stable order.
/// Adding a target is one line here plus the backend crate itself.
pub(crate) fn backends() -> Vec<Box<dyn Backend>> {
    vec![
        Box::new(ciac_backend_python::PythonBackend),
        Box::new(ciac_backend_rust::RustBackend),
        Box::new(ciac_backend_ts::TsBackend),
        Box::new(ciac_backend_go::GoBackend),
        Box::new(ciac_backend_java::JavaBackend),
    ]
}

/// Runs the front-end (module resolution + parse + analyze) on an entry
/// source file, printing all diagnostics. Returns the IR when the
/// program is valid, plus the full resolved [`SourceMap`] (every file
/// `import "path";` pulled in, v0.8 M1) so callers that need the whole
/// source set (e.g. hashing it for the manifest) don't re-resolve it.
pub(crate) fn front_end(file: &Path) -> Result<(Option<NormalizedIr>, bool, SourceMap)> {
    let (ir, has_errors, sources, diags) = front_end_quiet(file)?;
    let renderer = AriadneRenderer {
        color: std::io::stderr().is_terminal(),
    };
    let mut stderr = std::io::stderr().lock();
    for diag in diags.iter() {
        renderer.render(diag, &sources, &mut stderr)?;
    }
    Ok((ir, has_errors, sources))
}

/// [`front_end`] without the ariadne rendering — `--json` callers
/// (v0.10 M3) serialize the returned [`Diagnostics`] instead.
/// `pub(crate)`: also `ciac mcp`'s `fix` tool's (v0.15 M7) entry point
/// for a diagnostic's offered [`ciac_diagnostics::Fix`], which the
/// resolved-position `--json` envelope doesn't carry `Span`s for.
pub(crate) fn front_end_quiet(
    file: &Path,
) -> Result<(Option<NormalizedIr>, bool, SourceMap, Diagnostics)> {
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::load(file, &mut sources, &mut diags)
        .with_context(|| format!("cannot read {}", file.display()))?;
    let ir = ciac_sema::analyze(&program, &mut diags);
    diags.sort();
    let has_errors = diags.has_errors();
    Ok((ir, has_errors, sources, diags))
}

pub fn check(file: &Path, json: bool) -> Result<ExitCode> {
    if json {
        let (envelope, code) = check_envelope(file)?;
        crate::json_out::emit(&envelope);
        return Ok(code);
    }
    let (ir, has_errors, _sources) = front_end(file)?;
    if has_errors || ir.is_none() {
        return Ok(ExitCode::FAILURE);
    }
    eprintln!("{}: no errors", file.display());
    Ok(ExitCode::SUCCESS)
}

/// [`check`]'s JSON path as a value rather than a print — the form
/// `ciac mcp` (v0.13 M5) consumes directly, with no stdout capture in
/// between.
pub(crate) fn check_envelope(file: &Path) -> Result<(crate::json_out::Envelope, ExitCode)> {
    let (ir, has_errors, sources, diags) = front_end_quiet(file)?;
    let success = !has_errors && ir.is_some();
    let envelope = crate::json_out::envelope("check", success, &diags, &sources);
    let code = if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };
    Ok((envelope, code))
}

/// `--json` promises exactly one JSON document on stdout — but child
/// processes (`uv run pytest`, `docker compose`, `cargo test`) inherit
/// stdout by default and would corrupt it. When set, [`run_streamed`]
/// captures children's stdout and replays it on stderr instead.
static JSON_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Runs a child to completion: streaming (inherited stdio) in human
/// mode, captured-and-replayed-on-stderr in `--json` mode so stdout
/// stays reserved for the envelope.
fn run_streamed(command: &mut Command) -> std::io::Result<std::process::ExitStatus> {
    if JSON_MODE.load(std::sync::atomic::Ordering::Relaxed) {
        let output = command.output()?;
        use std::io::Write;
        let mut err = std::io::stderr().lock();
        err.write_all(&output.stdout)?;
        err.write_all(&output.stderr)?;
        Ok(output.status)
    } else {
        command.status()
    }
}

/// Wraps a command's normal execution for `--json` mode (v0.10 M3):
/// compiles once quietly for the envelope's diagnostics, short-
/// circuiting on compile errors, then runs the untouched human-mode
/// body (which recompiles — commands here already trade recompute for
/// simplicity) and reports its outcome as the envelope's `success`.
/// Returns the envelope as a value; the caller (a CLI wrapper printing
/// it, or `ciac mcp` consuming it directly) decides what happens next.
fn with_json_envelope(
    command: &'static str,
    file: &Path,
    body: impl FnOnce() -> Result<ExitCode>,
) -> Result<(crate::json_out::Envelope, ExitCode)> {
    JSON_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    let (ir, has_errors, sources, diags) = front_end_quiet(file)?;
    if has_errors || ir.is_none() {
        let envelope = crate::json_out::envelope(command, false, &diags, &sources);
        return Ok((envelope, ExitCode::FAILURE));
    }
    let code = match body() {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    };
    let envelope = crate::json_out::envelope(command, code == ExitCode::SUCCESS, &diags, &sources);
    Ok((envelope, code))
}

/// `--deploy k8s` and its two image-naming knobs, bundled since they
/// only ever travel together from `ciac build`'s CLI args.
pub struct DeployOpts {
    pub deploy: Vec<String>,
    pub image_prefix: Option<String>,
    pub image_tag: String,
    pub profile: String,
    pub secrets: bool,
    /// `--deploy ci`'s breaking-change gate (v0.18 M3, 18UpdatePlan.md
    /// Pillar 4): path to the checked-in semantic baseline the
    /// generated `semantic-compat` job diffs against. Ignored unless
    /// `deploy` contains `"ci"`.
    pub semantic_baseline: Option<PathBuf>,
}

#[allow(clippy::too_many_arguments)]
pub fn build(
    file: &Path,
    target: &str,
    out: &Path,
    force: bool,
    adopt: bool,
    deploy: DeployOpts,
    client: Vec<String>,
    name: Option<String>,
    json: bool,
) -> Result<ExitCode> {
    if json {
        let (envelope, code) = with_json_envelope("build", file, || {
            build_inner(file, target, out, force, adopt, deploy, client, name)
        })?;
        crate::json_out::emit(&envelope);
        return Ok(code);
    }
    build_inner(file, target, out, force, adopt, deploy, client, name)
}

/// [`build`]'s JSON path as a value — `ciac mcp`'s `build` tool (always
/// non-forcing, non-adopting: the plan-vs-clobber tradeoff needs a
/// human at the keyboard, so MCP only ever regenerates in place).
pub(crate) fn build_envelope(
    file: &Path,
    target: &str,
    out: &Path,
    deploy: DeployOpts,
    name: Option<String>,
) -> Result<(crate::json_out::Envelope, ExitCode)> {
    with_json_envelope("build", file, || {
        build_inner(file, target, out, false, false, deploy, Vec::new(), name)
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_inner(
    file: &Path,
    target: &str,
    out: &Path,
    force: bool,
    adopt: bool,
    deploy: DeployOpts,
    client: Vec<String>,
    name: Option<String>,
) -> Result<ExitCode> {
    if force && adopt {
        bail!("--force and --adopt cannot be used together");
    }
    let Some(profile) = ciac_codegen::Profile::parse(&deploy.profile) else {
        bail!(
            "unknown --profile `{}`; available: dev, staging, prod",
            deploy.profile
        );
    };
    let mut k8s_image = None;
    let mut terraform = None;
    let mut ci = false;
    for kind in &deploy.deploy {
        match kind.as_str() {
            "k8s" => k8s_image = Some((deploy.image_prefix.as_deref(), deploy.image_tag.as_str())),
            "terraform" => terraform = Some(profile),
            "ci" => ci = true,
            other => bail!("unknown --deploy target `{other}`; available: k8s, terraform, ci"),
        }
    }
    // v0.18 M3: `--deploy ci --semantic-baseline <path>` (18UpdatePlan.md
    // Pillar 4). The source entry and baseline must resolve beneath the
    // generated workflow's repository root — a developer's absolute
    // path can never mean anything there, so it's refused up front
    // rather than baked into YAML that would only work on this machine.
    let semantic_gate = match &deploy.semantic_baseline {
        Some(baseline) => {
            if !ci {
                bail!("--semantic-baseline requires --deploy ci");
            }
            if file.is_absolute() {
                bail!(
                    "--semantic-baseline requires the source file to be a relative path (got {}); \
                     invoke `ciac build` with a path relative to the repository root",
                    file.display()
                );
            }
            if baseline.is_absolute() {
                bail!(
                    "--semantic-baseline must be a relative path (got {}); it is embedded in the \
                     generated workflow, which runs against the repository root",
                    baseline.display()
                );
            }
            let Some(source_file) = file.to_str() else {
                bail!("source file path {} is not valid UTF-8", file.display());
            };
            let Some(baseline_str) = baseline.to_str() else {
                bail!(
                    "--semantic-baseline path {} is not valid UTF-8",
                    baseline.display()
                );
            };
            Some((source_file.to_owned(), baseline_str.to_owned()))
        }
        None => None,
    };
    let mut ts_client = false;
    for kind in &client {
        match kind.as_str() {
            "ts" => ts_client = true,
            other => bail!("unknown --client target `{other}`; available: ts"),
        }
    }

    // v0.18 M5: recorded once, verbatim, so `ciac rename --out <tree>`
    // (and any future tool needing to replay a build) never has to ask
    // the user to re-supply flags they already gave once.
    let recipe = ciac_codegen::manifest::BuildRecipe {
        entry: file.display().to_string(),
        target: target.to_owned(),
        name: name.clone(),
        deploy: deploy.deploy.clone(),
        profile: deploy.profile.clone(),
        secrets: deploy.secrets,
        image_prefix: deploy.image_prefix.clone(),
        image_tag: deploy.image_tag.clone(),
        clients: client.clone(),
        semantic_baseline: deploy
            .semantic_baseline
            .as_ref()
            .map(|p| p.display().to_string()),
    };

    let Generated {
        backend,
        project,
        source_hash,
        tables,
        next_migration_seq,
        records,
        semantic_snapshot,
    } = generate(
        file,
        target,
        out,
        name,
        k8s_image,
        terraform,
        profile,
        deploy.secrets,
        ts_client,
        ci,
        deploy.image_prefix.clone(),
        semantic_gate,
    )?;

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
            ciac_syntax::LANGUAGE_VERSION,
            source_hash,
            backend.id(),
        );
        manifest.tables = tables;
        manifest.next_migration_seq = next_migration_seq;
        manifest.records = records;
        manifest.semantic_snapshot = Some(semantic_snapshot);
        manifest.recipe = Some(recipe.clone());
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
            ciac_syntax::LANGUAGE_VERSION,
            source_hash,
            backend.id(),
        );
        manifest.tables = tables;
        manifest.next_migration_seq = next_migration_seq;
        manifest.records = records;
        manifest.semantic_snapshot = Some(semantic_snapshot);
        manifest.recipe = Some(recipe.clone());
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

/// v0.18 M5: replays a checked-in [`ciac_codegen::manifest::BuildRecipe`]
/// against `file` (the entry a rename just edited), reporting the
/// regeneration plan against `out`'s existing manifest — `ciac rename
/// --out <tree>`'s conflict check. With `commit: false` this never
/// writes anything; the caller decides whether the plan is safe to
/// proceed on. With `commit: true` it additionally applies the plan and
/// writes the updated manifest, exactly like `ciac build`'s own
/// non-force path — called only after the caller has already confirmed
/// the plan has no conflicts.
pub(crate) fn replay_recipe(
    file: &Path,
    out: &Path,
    recipe: &ciac_codegen::manifest::BuildRecipe,
    commit: bool,
) -> Result<RegenPlan> {
    let Some(profile) = ciac_codegen::Profile::parse(&recipe.profile) else {
        bail!(
            "recorded recipe has an unknown profile `{}`",
            recipe.profile
        );
    };
    let mut k8s_image = None;
    let mut terraform = None;
    let mut ci = false;
    for kind in &recipe.deploy {
        match kind.as_str() {
            "k8s" => k8s_image = Some((recipe.image_prefix.as_deref(), recipe.image_tag.as_str())),
            "terraform" => terraform = Some(profile),
            "ci" => ci = true,
            other => bail!("recorded recipe has an unknown --deploy target `{other}`"),
        }
    }
    let semantic_gate = recipe
        .semantic_baseline
        .as_ref()
        .map(|baseline| (recipe.entry.clone(), baseline.clone()));
    let ts_client = recipe.clients.iter().any(|c| c == "ts");

    let Generated {
        project,
        source_hash,
        backend,
        tables,
        next_migration_seq,
        records,
        semantic_snapshot,
    } = generate(
        file,
        &recipe.target,
        out,
        recipe.name.clone(),
        k8s_image,
        terraform,
        profile,
        recipe.secrets,
        ts_client,
        ci,
        recipe.image_prefix.clone(),
        semantic_gate,
    )?;

    let existing_manifest = if manifest_path(out).exists() {
        Some(load_manifest(out).with_context(|| {
            format!(
                "cannot read regeneration manifest {}",
                manifest_path(out).display()
            )
        })?)
    } else {
        None
    };
    let plan = plan_regeneration(&project, out, existing_manifest.as_ref(), RegenMode::Normal)
        .with_context(|| format!("cannot compare generated files with {}", out.display()))?;

    if commit {
        apply_regeneration(&plan, out, ApplyMode::Full)
            .with_context(|| format!("cannot apply regeneration to {}", out.display()))?;
        let mut manifest = build_manifest(
            &project,
            env!("CARGO_PKG_VERSION"),
            ciac_syntax::LANGUAGE_VERSION,
            source_hash,
            backend.id(),
        );
        manifest.tables = tables;
        manifest.next_migration_seq = next_migration_seq;
        manifest.records = records;
        manifest.semantic_snapshot = Some(semantic_snapshot);
        manifest.recipe = Some(recipe.clone());
        write_manifest(out, &manifest)
            .with_context(|| format!("cannot write manifest in {}", out.display()))?;
    }

    Ok(plan)
}

pub fn diff(
    file: &Path,
    target: &str,
    out: &Path,
    patch: bool,
    name: Option<String>,
    json: bool,
) -> Result<ExitCode> {
    if json {
        let (envelope, code) = diff_envelope(file, target, out, patch, name)?;
        crate::json_out::emit(&envelope);
        return Ok(code);
    }
    let plan = diff_plan(file, target, out, name)?;
    for entry in &plan.entries {
        println!("{:13} {}", entry.status.as_str(), entry.path);
        if let Some(sidecar) = &entry.sidecar_path {
            println!("{:13} {}", "sidecar", sidecar);
        }
        if patch {
            if let Some(text) = patch_text(entry) {
                print!("{text}");
            }
        }
    }
    if plan.has_errors() {
        Ok(ExitCode::FAILURE)
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// `diff` is the manifest-aware dry-run; `--json` (v0.10 M4) makes it
/// machine-readable — the regeneration plan as data in the same
/// envelope `check`/`build`/`verify --json` use. Returns the envelope
/// as a value; the CLI (`diff`) prints it, `ciac mcp`'s `diff` tool
/// (v0.13 M5) consumes it directly.
pub(crate) fn diff_envelope(
    file: &Path,
    target: &str,
    out: &Path,
    patch: bool,
    name: Option<String>,
) -> Result<(crate::json_out::Envelope, ExitCode)> {
    JSON_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    let (ir, has_errors, sources, diags) = front_end_quiet(file)?;
    if has_errors || ir.is_none() {
        let envelope = crate::json_out::envelope("diff", false, &diags, &sources);
        return Ok((envelope, ExitCode::FAILURE));
    }
    let (entries, success) = match diff_plan(file, target, out, name) {
        Ok(plan) => {
            let entries = plan
                .entries
                .iter()
                .map(|entry| crate::json_out::DiffEntry {
                    path: entry.path.clone(),
                    status: entry.status.as_str().to_owned(),
                    sidecar: entry.sidecar_path.clone(),
                    patch: if patch { patch_text(entry) } else { None },
                })
                .collect();
            (Some(entries), !plan.has_errors())
        }
        Err(err) => {
            eprintln!("error: {err:#}");
            (None, false)
        }
    };
    let mut envelope = crate::json_out::envelope("diff", success, &diags, &sources);
    envelope.entries = entries;
    let code = if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };
    Ok((envelope, code))
}

/// `ciac diff --semantic` (v0.18 M3): compares the current program's
/// canonical architecture against a baseline, disjoint from the
/// regeneration-diff mode above (that one requires `--target`/`--out`;
/// this one requires neither). Exactly one of `--against`/`--baseline`
/// is expected; with neither, falls back to the default checked-in
/// baseline path `ciac baseline` itself would use, refusing cleanly if
/// nothing is there yet. `--against` currently accepts a source file
/// path only — a git-ref form is 18UpdatePlan.md's own stated
/// direction, not implemented this milestone (disclosed, not silent).
#[allow(clippy::too_many_arguments)]
pub fn diff_semantic(
    file: &Path,
    against: Option<&Path>,
    baseline: Option<&Path>,
    deny_breaking: bool,
    format: &str,
    json: bool,
) -> Result<ExitCode> {
    if against.is_some() && baseline.is_some() {
        bail!("`--against` and `--baseline` are mutually exclusive; pass at most one");
    }
    if json {
        let (envelope, code) = diff_semantic_envelope(file, against, baseline, deny_breaking)?;
        crate::json_out::emit(&envelope);
        return Ok(code);
    }

    let (ir, has_errors, _sources) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        return Ok(ExitCode::FAILURE);
    };
    let new_model = ciac_codegen::semantic_model::SemanticModel::from_ir(&ir);

    let old_model = match load_comparison_baseline(file, against, baseline)? {
        Ok(model) => model,
        Err(message) => {
            eprintln!("error: {message}");
            return Ok(ExitCode::FAILURE);
        }
    };

    let changes = ciac_codegen::semantic_diff::diff_models(&old_model, &new_model);
    print_semantic_diff(&changes, format);

    let breaking = ciac_codegen::semantic_diff::overall_classification(&changes)
        == Some(ciac_codegen::semantic_diff::Classification::Breaking);
    if deny_breaking && breaking {
        eprintln!(
            "policy: --deny-breaking failed ({} breaking change(s))",
            changes
                .iter()
                .filter(
                    |c| c.classification == ciac_codegen::semantic_diff::Classification::Breaking
                )
                .count()
        );
        return Ok(ExitCode::FAILURE);
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn diff_semantic_envelope(
    file: &Path,
    against: Option<&Path>,
    baseline: Option<&Path>,
    deny_breaking: bool,
) -> Result<(crate::json_out::Envelope, ExitCode)> {
    JSON_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    let (ir, has_errors, sources, diags) = front_end_quiet(file)?;
    if has_errors || ir.is_none() {
        let envelope = crate::json_out::envelope("diff", false, &diags, &sources);
        return Ok((envelope, ExitCode::FAILURE));
    }
    let new_model = ciac_codegen::semantic_model::SemanticModel::from_ir(ir.as_ref().unwrap());

    let (changes, success) = match load_comparison_baseline(file, against, baseline) {
        Ok(Ok(old_model)) => {
            let changes = ciac_codegen::semantic_diff::diff_models(&old_model, &new_model);
            let breaking = ciac_codegen::semantic_diff::overall_classification(&changes)
                == Some(ciac_codegen::semantic_diff::Classification::Breaking);
            (Some(changes), !(deny_breaking && breaking))
        }
        Ok(Err(message)) => {
            eprintln!("error: {message}");
            (None, false)
        }
        Err(err) => {
            eprintln!("error: {err:#}");
            (None, false)
        }
    };

    let mut envelope = crate::json_out::envelope("diff", success, &diags, &sources);
    if let Some(changes) = changes {
        let breaking = changes
            .iter()
            .filter(|c| c.classification == ciac_codegen::semantic_diff::Classification::Breaking)
            .count();
        let additive = changes
            .iter()
            .filter(|c| c.classification == ciac_codegen::semantic_diff::Classification::Additive)
            .count();
        let internal = changes
            .iter()
            .filter(|c| c.classification == ciac_codegen::semantic_diff::Classification::Internal)
            .count();
        envelope.semantic = Some(crate::json_out::SemanticDiffResult {
            semantic_diff_version: 1,
            policy: crate::json_out::SemanticDiffPolicy {
                deny_breaking,
                passed: !(deny_breaking && breaking > 0),
            },
            summary: crate::json_out::SemanticDiffSummary {
                breaking,
                additive,
                internal,
            },
            changes,
        });
    }
    let code = if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };
    Ok((envelope, code))
}

/// Resolves the "old" side of a semantic comparison: `--against`
/// (another source file, compiled fresh), `--baseline` (a checked-in
/// baseline JSON file), or — with neither — the default path `ciac
/// baseline` itself would use. The inner `Result` distinguishes a
/// clean, expected refusal (bad/missing baseline, incompatible
/// version) from an unexpected I/O error the outer `Result` carries.
pub(crate) fn load_comparison_baseline(
    file: &Path,
    against: Option<&Path>,
    baseline: Option<&Path>,
) -> Result<std::result::Result<ciac_codegen::semantic_model::SemanticModel, String>> {
    if let Some(against) = against {
        let (ir, has_errors, _sources) = front_end(against)?;
        let Some(ir) = ir.filter(|_| !has_errors) else {
            return Ok(Err(format!(
                "{} (the --against source) failed to compile",
                against.display()
            )));
        };
        return Ok(Ok(ciac_codegen::semantic_model::SemanticModel::from_ir(
            &ir,
        )));
    }
    let path = baseline
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_baseline_path(file));
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Err(format!(
                "no baseline at {} -- pass --against/--baseline, or run `ciac baseline` first",
                path.display()
            )));
        }
        Err(err) => return Err(err).with_context(|| format!("cannot read {}", path.display())),
    };
    let parsed: ciac_codegen::semantic_model::SemanticBaseline =
        match serde_json::from_slice(&bytes) {
            Ok(b) => b,
            Err(err) => {
                return Ok(Err(format!(
                    "{} is not a valid semantic baseline: {err}",
                    path.display()
                )))
            }
        };
    if parsed.semantic_baseline_version > ciac_codegen::semantic_model::SEMANTIC_BASELINE_VERSION {
        return Ok(Err(format!(
            "{} was written by a newer, incompatible baseline format (version {}, this build \
             understands up to {})",
            path.display(),
            parsed.semantic_baseline_version,
            ciac_codegen::semantic_model::SEMANTIC_BASELINE_VERSION
        )));
    }
    Ok(Ok(parsed.model))
}

fn print_semantic_diff(changes: &[ciac_codegen::semantic_diff::Change], format: &str) {
    if changes.is_empty() {
        println!("no architecture changes detected");
        return;
    }
    match format {
        "markdown" => {
            println!("| Classification | Kind | Symbol | Message |");
            println!("|---|---|---|---|");
            for c in changes {
                println!(
                    "| {:?} | {} | `{}` | {} |",
                    c.classification, c.kind, c.symbol.display, c.message
                );
            }
        }
        _ => {
            for c in changes {
                println!(
                    "{:9} {:30} {}",
                    format!("{:?}", c.classification),
                    c.kind,
                    c.symbol.display
                );
                println!("          {}", c.message);
                for consumer in &c.consumers {
                    println!(
                        "          consumer: {} ({})",
                        consumer.service.as_deref().unwrap_or("?"),
                        consumer.kind
                    );
                }
                if c.backfill_plan_available {
                    println!("          note: `ciac backfill plan` is available for this change");
                }
            }
        }
    }
}

fn diff_plan(file: &Path, target: &str, out: &Path, name: Option<String>) -> Result<RegenPlan> {
    let Generated { project, .. } = generate(
        file,
        target,
        out,
        name,
        None,
        None,
        ciac_codegen::Profile::Dev,
        false,
        false,
        false,
        None,
        None,
    )?;
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
    plan_regeneration(&project, out, manifest.as_ref(), RegenMode::Normal)
        .with_context(|| format!("cannot compare generated files with {}", out.display()))
}

#[allow(clippy::too_many_arguments)]
pub fn verify(
    file: &Path,
    target: &str,
    out: &Path,
    live: bool,
    system: bool,
    keep: bool,
    sim: bool,
    scenario: &[PathBuf],
    name: Option<String>,
    json: bool,
) -> Result<ExitCode> {
    if sim && scenario.is_empty() {
        bail!("--sim requires at least one --scenario");
    }
    if json {
        let (envelope, code) = if sim {
            verify_sim_envelope(file, target, out, scenario, name, None, None)?
        } else {
            with_json_envelope("verify", file, || {
                verify_inner(file, target, out, live, system, keep, name.clone())
            })?
        };
        crate::json_out::emit(&envelope);
        return Ok(code);
    }

    let static_code = verify_inner(file, target, out, live, system, keep, name.clone())?;
    if static_code != ExitCode::SUCCESS || !sim {
        return Ok(static_code);
    }
    let result = sim_inner(file, target, out, scenario, None, None, name, None)?;
    let mut all_passed = true;
    for outcome in &result.scenarios {
        if outcome.passed {
            println!("[PASS] {}", outcome.scenario);
        } else {
            all_passed = false;
            println!(
                "[FAIL] {}: {}",
                outcome.scenario,
                outcome.error.as_deref().unwrap_or("unknown error")
            );
        }
    }
    Ok(if all_passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// [`verify`]'s JSON path as a value — `ciac mcp`'s `verify` tool
/// (v0.13 M5) always runs the static check only (no `--system`/
/// `--live`: those boot Docker and belong to a human at a terminal,
/// per the plan's out-of-scope list).
pub(crate) fn verify_envelope(
    file: &Path,
    target: &str,
    out: &Path,
    name: Option<String>,
) -> Result<(crate::json_out::Envelope, ExitCode)> {
    with_json_envelope("verify", file, || {
        verify_inner(file, target, out, false, false, false, name)
    })
}

fn verify_inner(
    file: &Path,
    target: &str,
    out: &Path,
    live: bool,
    system: bool,
    keep: bool,
    name: Option<String>,
) -> Result<ExitCode> {
    let Generated {
        backend,
        project,
        source_hash,
        tables,
        next_migration_seq,
        records,
        semantic_snapshot,
    } = generate(
        file,
        target,
        out,
        name,
        None,
        None,
        ciac_codegen::Profile::Dev,
        false,
        false,
        false,
        None,
        None,
    )?;

    let materialize_result = materialize_generated(
        out,
        backend.as_ref(),
        &project,
        &source_hash,
        tables,
        next_migration_seq,
        records,
        semantic_snapshot,
    )?;
    if materialize_result != ExitCode::SUCCESS {
        return Ok(materialize_result);
    }

    let static_result = validate_generated(out, backend.id())?;
    if static_result != ExitCode::SUCCESS {
        return Ok(static_result);
    }
    if system {
        let system_result = verify_system(out, keep)?;
        if system_result != ExitCode::SUCCESS {
            return Ok(system_result);
        }
    }
    if live {
        return verify_live(file, out, keep);
    }
    Ok(ExitCode::SUCCESS)
}

/// Writes a freshly generated project into `out`, or -- when `out`
/// already holds one -- checks it for drift against its manifest and
/// refuses on anything beyond `Unchanged`/`OrphanLeft`. Shared by
/// [`verify_inner`] and [`sim`]: both need "reuse `out` if it already
/// matches, write fresh if it doesn't exist yet," neither wants its own
/// copy of the manifest bookkeeping.
#[allow(clippy::too_many_arguments)]
fn materialize_generated(
    out: &Path,
    backend: &dyn Backend,
    project: &GeneratedProject,
    source_hash: &str,
    tables: BTreeMap<String, TableSchema>,
    next_migration_seq: u32,
    records: BTreeMap<String, RecordSchema>,
    semantic_snapshot: ciac_codegen::semantic_model::SemanticModel,
) -> Result<ExitCode> {
    if !output_dir_nonempty(out)? {
        project
            .write_to(out)
            .with_context(|| format!("cannot write initial project to {}", out.display()))?;
        let mut manifest = build_manifest(
            project,
            env!("CARGO_PKG_VERSION"),
            ciac_syntax::LANGUAGE_VERSION,
            source_hash,
            backend.id(),
        );
        manifest.tables = tables;
        manifest.next_migration_seq = next_migration_seq;
        manifest.records = records;
        manifest.semantic_snapshot = Some(semantic_snapshot);
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
        let plan = plan_regeneration(project, out, Some(&manifest), RegenMode::Normal)
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
    Ok(ExitCode::SUCCESS)
}

/// The embedded bounded-child-protocol runner (v0.17 M10): `include_str!`
/// bakes `sim/pyrunner/*.py`'s source into the `ciac` binary at compile
/// time, so `ciac sim` can write it out next to whatever generated
/// project it's driving regardless of the user's cwd or how `ciac` was
/// installed. Kept as five separate files (not one concatenated blob)
/// because `auto_driver.py` imports its siblings as ordinary top-level
/// modules (`from cron import CronSchedule`, ...) -- writing them out
/// under those same names is what makes that resolve unchanged.
///
/// Read from `vendor/pyrunner/`, a physical copy checked into this
/// crate's own directory, not from the repo-root `sim/pyrunner/`
/// directly -- found live via a real `cargo publish` failure:
/// `cargo package`/`publish` only bundles files inside a crate's own
/// directory, so a `../../../sim/pyrunner/...` `include_str!` doesn't
/// exist in the package tarball. Mirrors `ciac-backend-rust/vendor/
/// ciac-sim/`'s identical fix for the identical reason. Run
/// `scripts/sync-vendored-ciac-assets.sh` after changing any of the
/// real `sim/pyrunner/*.py` files, and see this module's own
/// `vendored_pyrunner_matches_source` test, which fails loudly in a
/// normal workspace build (never from a published crate, where
/// `sim/pyrunner/` isn't reachable) if the two fall out of sync.
const PYRUNNER_WORLD: &str = include_str!("../vendor/pyrunner/world.py");
const PYRUNNER_CRON: &str = include_str!("../vendor/pyrunner/cron.py");
const PYRUNNER_SCENARIO_RUNNER: &str = include_str!("../vendor/pyrunner/scenario_runner.py");
const PYRUNNER_REPLAY: &str = include_str!("../vendor/pyrunner/replay.py");
const PYRUNNER_AUTO_DRIVER: &str = include_str!("../vendor/pyrunner/auto_driver.py");
// 28UpdatePlan.md M3c: the multi-service counterpart to
// `auto_driver.py`/its own package-aliasing seam -- written out
// alongside the single-service files always (harmless, unused dead
// weight for a single-service run) rather than conditionally, so
// `write_pyrunner` stays one unconditional list.
const PYRUNNER_MULTI_SERVICE: &str = include_str!("../vendor/pyrunner/multi_service.py");
const PYRUNNER_MULTI_DRIVER: &str = include_str!("../vendor/pyrunner/multi_driver.py");

fn write_pyrunner(sim_dir: &Path) -> Result<()> {
    for (name, content) in [
        ("world.py", PYRUNNER_WORLD),
        ("cron.py", PYRUNNER_CRON),
        ("scenario_runner.py", PYRUNNER_SCENARIO_RUNNER),
        ("replay.py", PYRUNNER_REPLAY),
        ("auto_driver.py", PYRUNNER_AUTO_DRIVER),
        ("multi_service.py", PYRUNNER_MULTI_SERVICE),
        ("multi_driver.py", PYRUNNER_MULTI_DRIVER),
    ] {
        let path = sim_dir.join(name);
        std::fs::write(&path, content)
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(())
}

/// Lexically absolutizes `path` against the current process's cwd,
/// without requiring it to exist yet -- unlike [`Path::canonicalize`],
/// which a not-yet-written `--record` target would fail. A child
/// process (`auto_driver.py`) runs with its cwd set to the generated
/// project, so paths the user gave relative to *their own* shell need
/// resolving before crossing that boundary.
fn resolve_path(path: &Path) -> Result<PathBuf> {
    std::path::absolute(path).with_context(|| format!("cannot resolve path {}", path.display()))
}

/// Runs a child to completion and captures its output. With `timeout`,
/// polls (rather than blocking on [`Command::output`]) and kills the
/// child once the wall-clock limit is exceeded — `ciac mcp`'s
/// `verify_sim` tool needs this (17UpdatePlan.md: "fixed server-side
/// step/wall limits"), since an MCP client that hangs mid-run has no
/// operator present to interrupt it the way a terminal user does.
/// `ciac sim`/`verify --sim` at a terminal pass `None`: a human is
/// already free to Ctrl+C.
fn run_captured(
    cmd: &mut Command,
    timeout: Option<std::time::Duration>,
) -> Result<std::process::Output> {
    let Some(limit) = timeout else {
        return cmd.output().context("failed to run child process");
    };
    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("failed to spawn child process")?;
    let start = std::time::Instant::now();
    loop {
        if child
            .try_wait()
            .context("polling child process status")?
            .is_some()
        {
            return child
                .wait_with_output()
                .context("collecting child process output");
        }
        if start.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            bail!("child process exceeded the {limit:?} wall-clock limit and was killed");
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// [`sim`]'s working parts, shared by the human-readable and `--json`
/// paths: materializes the generated project, builds its `SimPlan`,
/// writes the embedded runner next to it, and runs `auto_driver.py`
/// once per `--scenario`, parsing its one-line JSON reply. Errors here
/// are setup failures (bad target, drifted project, a scenario the
/// driver's auto-discovery can't wire) — never a scenario's own
/// pass/fail, which is reported as data in the returned outcomes, not
/// as an `Err`.
#[allow(clippy::too_many_arguments)]
fn sim_inner(
    file: &Path,
    target: &str,
    out: &Path,
    scenarios: &[PathBuf],
    record: Option<&Path>,
    replay: Option<&Path>,
    name: Option<String>,
    wall_timeout: Option<std::time::Duration>,
) -> Result<crate::json_out::SimResult> {
    // v0.22 M1: resolved through the *built-in* registry only (not the
    // external-protocol fallback `generate()` uses) -- `backends()`
    // (v0.23 M9: python/rust/typescript) is the actual source of
    // truth for which targets exist at all; whether one of them can
    // simulate is a separate question this function checks next via
    // `SimSupport`, not this lookup.
    let sim_backend = backends()
        .into_iter()
        .find(|b| b.id() == target)
        .ok_or_else(|| {
            anyhow!(
                "`ciac sim`: unknown target `{target}`; see `ciac targets` for the registered set"
            )
        })?;
    let target_info = sim_backend.target_info();
    let sim_support = target_info.sim;
    if let SimSupport::None { reason } = sim_support {
        bail!("`ciac sim --target {target}` cannot simulate this target: {reason}");
    }
    if record.is_some() && replay.is_some() {
        bail!("--record and --replay are mutually exclusive");
    }
    if (record.is_some() || replay.is_some()) && scenarios.len() != 1 {
        bail!("--record/--replay require exactly one --scenario");
    }
    if !target_info.sim_replay && (record.is_some() || replay.is_some()) {
        // 27UpdatePlan.md M1: `sim_replay` is its own field, decoupled
        // from `SimSupport` depth -- a target can simulate every verb
        // the language has (`Full`) and still not implement a replay
        // tape (today: every target but Python). Disclosed, not
        // silently ignored; target-generic message for the same reason
        // the `unsupported` refusal below is (v0.23 M9).
        bail!(
            "`ciac sim --target {target}` does not yet support --record/--replay ({}); see \
             docs/simulation.md",
            ciac_sim::SimCode::ReplayNotSupported
        );
    }

    let Generated {
        backend,
        project,
        source_hash,
        tables,
        next_migration_seq,
        records,
        semantic_snapshot,
    } = generate(
        file,
        target,
        out,
        name,
        None,
        None,
        ciac_codegen::Profile::Dev,
        false,
        false,
        false,
        None,
        None,
    )?;

    let materialize_result = materialize_generated(
        out,
        backend.as_ref(),
        &project,
        &source_hash,
        tables,
        next_migration_seq,
        records,
        semantic_snapshot,
    )?;
    if materialize_result != ExitCode::SUCCESS {
        bail!(
            "generated project at {} has drifted from the current program; run `ciac build` or \
             `ciac diff` first",
            out.display()
        );
    }

    // Re-derives `NormalizedIr` (the program compiled moments ago in
    // `generate`, so this cannot newly fail) to build the `SimPlan` a
    // scenario's own preflight and the driver's worker/job discovery
    // both need — `verify_live` does the same re-derivation for the
    // same reason.
    let (ir, has_errors, _sources) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        bail!("program stopped compiling between generate and sim");
    };
    if let SimSupport::Narrow { unsupported } = sim_support {
        let unsupported = unsupported(&ir);
        if !unsupported.is_empty() {
            // Target-generic on purpose (v0.23 M9): this refusal fires
            // for every `Narrow` target, not just Rust's own v0.17 M11
            // one this message used to name unconditionally -- a real
            // bug caught live when TypeScript became the second
            // `Narrow` target and this message kept saying "rust" and
            // "v0.17 M11" while actually refusing a `--target
            // typescript` run.
            bail!(
                "`ciac sim --target {target}` cannot simulate this program (disclosed scope):\n{}",
                unsupported
                    .iter()
                    .map(|reason| format!("  - {reason}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }
    }
    let plan = ciac_sim::SimPlan::from_ir(&ir, source_hash.clone());
    let plan_hash = plan.plan_hash();

    // 28UpdatePlan.md M1: each target's own generated runner still
    // parses and drives a `--scenario` file itself (Python/Rust/TS/Go/
    // Java each read the JSON independently below) -- this is a single
    // shared preflight *before* any of them run, so an unknown-service
    // reference (SIM0011) fails the same way regardless of `--target`,
    // rather than surfacing as whatever each runner's own lookup error
    // happens to say.
    for scenario_path in scenarios {
        let text = std::fs::read_to_string(scenario_path)
            .with_context(|| format!("cannot read scenario file {}", scenario_path.display()))?;
        let scenario = ciac_sim::Scenario::parse(&text)
            .with_context(|| format!("cannot parse scenario file {}", scenario_path.display()))?;
        scenario
            .validate()
            .with_context(|| format!("scenario file {} is invalid", scenario_path.display()))?;
        if let Err(err) = plan.validate_scenario(&scenario) {
            bail!(
                "`ciac sim`: scenario {} {} ({})",
                scenario_path.display(),
                err,
                ciac_sim::SimCode::UnknownService
            );
        }
    }

    // v0.22 M1: `Narrow` runs a generated-runner drive (Rust's own,
    // v0.17 M11; TypeScript's, v0.23 M9); `Full` keeps the
    // bounded-child-protocol drive (today: Python's). Two `Narrow`
    // targets share the same *shape* of driver (compile/build the
    // project's own generated runner, then execute it once per
    // scenario and parse its one-line JSON reply) but need different
    // toolchains to do it, so this dispatches on `target` itself
    // rather than trying to make one function branch internally.
    let scenario_outcomes = match sim_support {
        // target-literal-ok: `sim_drive_rust`/`sim_drive_typescript` are
        // genuinely different toolchains (cargo/npm), not a
        // `TargetInfo`-describable difference like `db_url_scheme` or a
        // `ValidateStep` list -- unlike the seam v0.22 M1's registry
        // closed (per-target string matches that *could* have been one
        // shared, data-driven code path), driving "the project's own
        // compiled runner binary" vs "the project's own compiled JS
        // entry point" are two different `std::process::Command`
        // recipes with no shared shape to factor `TargetInfo` around.
        SimSupport::Narrow { .. } if target == "rust" => {
            sim_drive_rust(out, scenarios, wall_timeout)?
        }
        // target-literal-ok: see the sibling arm above.
        SimSupport::Narrow { .. } if target == "typescript" => {
            sim_drive_typescript(out, scenarios, wall_timeout)?
        }
        // target-literal-ok: see the sibling arm above.
        SimSupport::Narrow { .. } if target == "go" => sim_drive_go(out, scenarios, wall_timeout)?,
        // target-literal-ok: see the sibling arm above.
        SimSupport::Narrow { .. } if target == "java" => {
            sim_drive_java(out, scenarios, wall_timeout)?
        }
        SimSupport::Narrow { .. } => bail!(
            "`ciac sim` has no generated-runner driver wired for target `{target}` (its \
             `TargetInfo::sim` claims `Narrow` support, but `sim_inner` doesn't know how to \
             drive it -- this is a real gap in the CLI, not a disclosed scope limit)"
        ),
        SimSupport::Full => sim_drive_python(
            out,
            scenarios,
            record,
            replay,
            &source_hash,
            &plan,
            &plan_hash,
            wall_timeout,
        )?,
        SimSupport::None { .. } => unreachable!("refused above by the `SimSupport::None` check"),
    };

    Ok(crate::json_out::SimResult {
        plan_hash,
        source_hash,
        scenarios: scenario_outcomes,
    })
}

/// Drives every `--scenario` against a generated Python project via the
/// embedded `auto_driver.py` (v0.17 M9/M10) — unchanged behavior,
/// factored out of `sim_inner` so it sits next to [`sim_drive_rust`]
/// instead of inside one large target `if`.
#[allow(clippy::too_many_arguments)]
fn sim_drive_python(
    out: &Path,
    scenarios: &[PathBuf],
    record: Option<&Path>,
    replay: Option<&Path>,
    source_hash: &str,
    plan: &ciac_sim::SimPlan,
    plan_hash: &str,
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    // v0.29 M4: `out` crosses a subprocess-cwd boundary below (the
    // driver's own `PYTHONPATH`/`current_dir`), so a relative `out`
    // (e.g. the README's own `--out ./build`) must be absolutized
    // *before* it becomes `project_dir` -- otherwise a path string
    // built from it gets re-resolved against the child's *own* cwd
    // instead of the caller's, silently landing on the wrong
    // directory (found live: `ciac sim ... --out ./build` failed with
    // `ModuleNotFoundError: No module named 'app'`).
    let projects = find_project_dirs(&resolve_path(out)?, "pyproject.toml")?;
    if projects.is_empty() {
        bail!("no generated python project found under {}", out.display());
    }

    // Deliberately *outside* any project dir: this is `ciac sim`'s own
    // scratch state (the embedded runner, the plan JSON), not part of
    // any generated project. Writing it inside a project dir was tried
    // first and found to be a real bug -- `validate_generated`'s
    // `ruff check .` (run by plain `verify` and by `verify --sim`'s own
    // static pass before it) would then lint the runner's own source as
    // if it were the user's generated code. Keyed by a hash of `out`'s
    // own canonical path (not any one project's -- 28UpdatePlan.md M3c:
    // a multi-service run has no single project to key off) so repeat
    // runs against the same `--out` reuse (and overwrite) one scratch
    // directory instead of accumulating a fresh one per invocation.
    let out_abs = resolve_path(out)?;
    let sim_dir = std::env::temp_dir().join(format!(
        "ciac-sim-{}",
        hash_bytes(out_abs.as_os_str().as_encoded_bytes())
    ));
    std::fs::create_dir_all(&sim_dir)
        .with_context(|| format!("cannot create {}", sim_dir.display()))?;
    write_pyrunner(&sim_dir)?;
    let plan_path = sim_dir.join("plan.json");
    std::fs::write(
        &plan_path,
        serde_json::to_vec_pretty(plan).context("SimPlan serializes")?,
    )
    .with_context(|| format!("cannot write {}", plan_path.display()))?;

    if let [project_dir] = projects.as_slice() {
        return sim_drive_python_single(
            project_dir,
            &sim_dir,
            &plan_path,
            scenarios,
            record,
            replay,
            source_hash,
            plan_hash,
            wall_timeout,
        );
    }
    sim_drive_python_multi(
        &projects,
        &sim_dir,
        &plan_path,
        plan,
        scenarios,
        record,
        replay,
        source_hash,
        plan_hash,
        wall_timeout,
    )
}

/// The single-service path, unchanged in shape from before 28's M3c
/// (`sim_drive_python` used to be this function's own body directly) --
/// factored out so `sim_drive_python` can dispatch to it or to
/// [`sim_drive_python_multi`] once it knows how many projects it found.
#[allow(clippy::too_many_arguments)]
fn sim_drive_python_single(
    project_dir: &Path,
    sim_dir: &Path,
    plan_path: &Path,
    scenarios: &[PathBuf],
    record: Option<&Path>,
    replay: Option<&Path>,
    source_hash: &str,
    plan_hash: &str,
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    run_in(project_dir, "uv", &["sync", "-q"])?;

    let pythonpath = std::env::join_paths([sim_dir.to_path_buf(), project_dir.to_path_buf()])
        .context("cannot build PYTHONPATH for the sim runner")?;

    let mut scenario_outcomes = Vec::new();
    for scenario_path in scenarios {
        let scenario_abs = resolve_path(scenario_path)?;
        let mut cmd = Command::new("uv");
        cmd.arg("run")
            .arg("python") // target-literal-ok: the python interpreter's own name, not a target-id match
            .arg(sim_dir.join("auto_driver.py"))
            .arg(plan_path)
            .arg(&scenario_abs)
            .arg("--source-hash")
            .arg(source_hash)
            .arg("--plan-hash")
            .arg(plan_hash)
            .current_dir(project_dir)
            .env("PYTHONPATH", &pythonpath);
        if let Some(record_path) = record {
            cmd.arg("--record").arg(resolve_path(record_path)?);
        }
        if let Some(replay_path) = replay {
            cmd.arg("--replay").arg(resolve_path(replay_path)?);
        }

        let output = run_captured(&mut cmd, wall_timeout).with_context(|| {
            format!(
                "failed to run auto_driver.py for scenario {}",
                scenario_path.display()
            )
        })?;
        if !output.stderr.is_empty() {
            use std::io::Write;
            let _ = std::io::stderr().write_all(&output.stderr);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().next_back().unwrap_or("").trim();
        if last_line.is_empty() {
            bail!(
                "auto_driver.py for scenario {} exited with {} and printed no result on stdout",
                scenario_path.display(),
                output.status
            );
        }
        let outcome: crate::json_out::SimScenarioOutcome = serde_json::from_str(last_line)
            .with_context(|| {
                format!(
                    "cannot parse auto_driver.py's result for scenario {}: {last_line:?}",
                    scenario_path.display()
                )
            })?;
        scenario_outcomes.push(outcome);
    }
    Ok(scenario_outcomes)
}

/// 28UpdatePlan.md M3c: N services, one shared world, one process --
/// `multi_driver.py` (see its own module doc for the package-aliasing
/// mechanism this depends on). Each generated project keeps its own
/// `uv`-managed venv (a service may declare dependencies none of the
/// others need — `nats-py` only where a queue is used, `aioboto3` only
/// where an object store is used, ...), so no single venv has the
/// union of every service's own packages; this function assembles that
/// union itself by pointing `PYTHONPATH` at every project's own
/// `.venv`'s `site-packages` directory (found by globbing, since the
/// exact `pythonX.Y` component varies by whatever Python `uv` resolved)
/// alongside `sim_dir` — `multi_driver.py`'s own `ServiceModules` is
/// what handles `app.*` module resolution per service; this function's
/// job is only making every *third-party* dependency importable
/// regardless of which service is currently active.
#[allow(clippy::too_many_arguments)]
fn sim_drive_python_multi(
    projects: &[PathBuf],
    sim_dir: &Path,
    plan_path: &Path,
    plan: &ciac_sim::SimPlan,
    scenarios: &[PathBuf],
    record: Option<&Path>,
    replay: Option<&Path>,
    source_hash: &str,
    plan_hash: &str,
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    // Matches each of `plan.services` (in the plan's own declaration
    // order — the order every registration in `multi_driver.py` must
    // follow) to the project directory `ciac build`'s own per-service
    // emission wrote it to: that directory's basename is exactly
    // `service.name.to_kebab_case()` (`ciac_codegen::model::Ctx::dir`'s
    // own derivation; see that field's doc comment) for a multi-service
    // system, so this is a lookup, not a guess -- and it bails with a
    // clear error rather than silently skipping a service if the
    // expected directory is somehow missing.
    let mut service_args: Vec<(String, PathBuf)> = Vec::with_capacity(plan.services.len());
    for service in &plan.services {
        let kebab = service.name.to_kebab_case();
        let dir = projects
            .iter()
            .find(|p| p.file_name().and_then(|n| n.to_str()) == Some(kebab.as_str()))
            .cloned()
            .ok_or_else(|| {
                anyhow!(
                    "`ciac sim`: no generated project directory named {kebab:?} for service \
                     {:?} (found {} project(s) under this system's --out)",
                    service.name,
                    projects.len()
                )
            })?;
        run_in(&dir, "uv", &["sync", "-q"])?;
        service_args.push((service.name.clone(), resolve_path(&dir)?));
    }

    let mut pythonpath_entries = vec![sim_dir.to_path_buf()];
    for (name, dir) in &service_args {
        let venv_lib = dir.join(".venv").join("lib");
        let site_packages = std::fs::read_dir(&venv_lib)
            .ok()
            .and_then(|mut entries| entries.find_map(|e| e.ok().map(|e| e.path().join("site-packages"))))
            .ok_or_else(|| {
                anyhow!(
                    "`ciac sim`: cannot find a site-packages directory under {} for service {name:?}",
                    venv_lib.display()
                )
            })?;
        pythonpath_entries.push(site_packages);
    }
    let pythonpath = std::env::join_paths(pythonpath_entries)
        .context("cannot build PYTHONPATH for the multi-service sim runner")?;

    // Any of the N venvs' own interpreter works equally well to *run*
    // `multi_driver.py` itself (only the third-party packages on
    // `PYTHONPATH` matter, not which `.venv`'s own `python` binary is
    // invoked) -- the first service's, arbitrarily but deterministically
    // (declaration order), keeps this reproducible.
    let (_, driver_project) = &service_args[0];

    let mut scenario_outcomes = Vec::new();
    for scenario_path in scenarios {
        let scenario_abs = resolve_path(scenario_path)?;
        let mut cmd = Command::new("uv");
        cmd.arg("run")
            .arg("--project")
            .arg(driver_project)
            .arg("python") // target-literal-ok: the python interpreter's own name, not a target-id match
            .arg(sim_dir.join("multi_driver.py"))
            .arg(plan_path)
            .arg(&scenario_abs)
            .arg("--source-hash")
            .arg(source_hash)
            .arg("--plan-hash")
            .arg(plan_hash)
            .current_dir(sim_dir)
            .env("PYTHONPATH", &pythonpath);
        for (name, dir) in &service_args {
            cmd.arg("--service")
                .arg(format!("{name}={}", dir.display()));
        }
        if let Some(record_path) = record {
            cmd.arg("--record").arg(resolve_path(record_path)?);
        }
        if let Some(replay_path) = replay {
            cmd.arg("--replay").arg(resolve_path(replay_path)?);
        }

        let output = run_captured(&mut cmd, wall_timeout).with_context(|| {
            format!(
                "failed to run multi_driver.py for scenario {}",
                scenario_path.display()
            )
        })?;
        if !output.stderr.is_empty() {
            use std::io::Write;
            let _ = std::io::stderr().write_all(&output.stderr);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().next_back().unwrap_or("").trim();
        if last_line.is_empty() {
            bail!(
                "multi_driver.py for scenario {} exited with {} and printed no result on stdout",
                scenario_path.display(),
                output.status
            );
        }
        let outcome: crate::json_out::SimScenarioOutcome = serde_json::from_str(last_line)
            .with_context(|| {
                format!(
                    "cannot parse multi_driver.py's result for scenario {}: {last_line:?}",
                    scenario_path.display()
                )
            })?;
        scenario_outcomes.push(outcome);
    }
    Ok(scenario_outcomes)
}

/// Drives every `--scenario` against a generated Rust project's own
/// `src/bin/sim_runner.rs` (v0.17 M11) — unlike the Python side, the
/// runner is generated code baked into the project itself, not a
/// scratch directory `ciac sim` writes out, since Rust needs the
/// runner compiled against this program's own concrete types. No
/// plan/source-hash arguments (the runner doesn't take any — see its
/// own doc comment for the narrower, disclosed CLI contract).
///
/// `find_project_dirs` (28UpdatePlan.md M6c) already excludes
/// `sim-shared/` and `system-runner/` from its walk, so more than one
/// remaining project directory means a real multi-service system —
/// dispatches to [`sim_drive_rust_multi`], mirroring `sim_drive_python`'s
/// own `_single`/`_multi` split.
fn sim_drive_rust(
    out: &Path,
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    // v0.29 M4: absolutize before it crosses a subprocess-cwd boundary
    // -- see the identical fix and comment on `sim_drive_python`.
    let projects = find_project_dirs(&resolve_path(out)?, "Cargo.toml")?;
    if projects.is_empty() {
        bail!("no generated rust project found under {}", out.display());
    }
    if let [project_dir] = projects.as_slice() {
        return sim_drive_rust_single(project_dir, scenarios, wall_timeout);
    }
    sim_drive_rust_multi(out, &projects, scenarios, wall_timeout)
}

/// The single-service path, unchanged in shape from before 28's M6c
/// (`sim_drive_rust` used to be this function's own body directly).
fn sim_drive_rust_single(
    project_dir: &Path,
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    if !project_dir.join("src/bin/sim_runner.rs").exists() {
        bail!(
            "no `src/bin/sim_runner.rs` in {} -- this program declares nothing v0.17 M11's \
             simulation world can fake (no `db`/`queue` use); see docs/simulation.md",
            project_dir.display()
        );
    }
    run_in(
        project_dir,
        "cargo",
        &["build", "-q", "--bin", "sim_runner"],
    )?;

    let mut scenario_outcomes = Vec::new();
    for scenario_path in scenarios {
        let scenario_abs = resolve_path(scenario_path)?;
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("-q")
            .arg("--bin")
            .arg("sim_runner")
            .arg("--")
            .arg(&scenario_abs)
            .current_dir(project_dir);

        let output = run_captured(&mut cmd, wall_timeout).with_context(|| {
            format!(
                "failed to run sim_runner for scenario {}",
                scenario_path.display()
            )
        })?;
        if !output.stderr.is_empty() {
            use std::io::Write;
            let _ = std::io::stderr().write_all(&output.stderr);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().next_back().unwrap_or("").trim();
        if last_line.is_empty() {
            bail!(
                "sim_runner for scenario {} exited with {} and printed no result on stdout",
                scenario_path.display(),
                output.status
            );
        }
        let outcome: crate::json_out::SimScenarioOutcome = serde_json::from_str(last_line)
            .with_context(|| {
                format!(
                    "cannot parse sim_runner's result for scenario {}: {last_line:?}",
                    scenario_path.display()
                )
            })?;
        scenario_outcomes.push(outcome);
    }
    Ok(scenario_outcomes)
}

/// 28UpdatePlan.md M6c: N services, one shared world, one process --
/// `system-runner/` (see `system_sim_runner.rs.j2`'s own doc comment)
/// is a normal Cargo binary crate `RustBackend::generate` already wrote
/// with path dependencies on every service crate plus `sim-shared`, so
/// unlike Python's `sim_drive_python_multi` (which has to assemble a
/// `PYTHONPATH` union across N separately-managed venvs at drive time),
/// there is nothing left to assemble here — `cargo build`/`cargo run`
/// inside `system-runner/` already resolves the whole dependency graph
/// through Cargo's own workspace-free path-dependency resolution.
fn sim_drive_rust_multi(
    out: &Path,
    projects: &[PathBuf],
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    let runner_dir = out.join("system-runner");
    if !runner_dir.join("src/main.rs").exists() {
        bail!(
            "no `system-runner/src/main.rs` under {} -- this system declares nothing \
             28UpdatePlan.md M6c's simulation world can fake in any service (no `db`/`queue`/... \
             use anywhere); see docs/simulation.md (found {} generated project(s) under this \
             system's --out)",
            out.display(),
            projects.len()
        );
    }
    run_in(&runner_dir, "cargo", &["build", "-q"])?;

    let mut scenario_outcomes = Vec::new();
    for scenario_path in scenarios {
        let scenario_abs = resolve_path(scenario_path)?;
        let mut cmd = Command::new("cargo");
        cmd.arg("run")
            .arg("-q")
            .arg("--")
            .arg(&scenario_abs)
            .current_dir(&runner_dir);

        let output = run_captured(&mut cmd, wall_timeout).with_context(|| {
            format!(
                "failed to run system-runner for scenario {}",
                scenario_path.display()
            )
        })?;
        if !output.stderr.is_empty() {
            use std::io::Write;
            let _ = std::io::stderr().write_all(&output.stderr);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().next_back().unwrap_or("").trim();
        if last_line.is_empty() {
            bail!(
                "system-runner for scenario {} exited with {} and printed no result on stdout",
                scenario_path.display(),
                output.status
            );
        }
        let outcome: crate::json_out::SimScenarioOutcome = serde_json::from_str(last_line)
            .with_context(|| {
                format!(
                    "cannot parse system-runner's result for scenario {}: {last_line:?}",
                    scenario_path.display()
                )
            })?;
        scenario_outcomes.push(outcome);
    }
    Ok(scenario_outcomes)
}

/// Drives every `--scenario` against a generated TypeScript project's
/// own `src/sim_runner.ts` (v0.23 M9) -- same generated-runner shape as
/// [`sim_drive_rust`] (the runner is generated code baked into the
/// project itself, compiled and run once per scenario, one-line JSON
/// reply on stdout), different toolchain: `npm ci` then `npm run
/// build` (tsc) instead of `cargo build`, `node dist/sim_runner.js --`
/// instead of `cargo run --bin sim_runner --`. Unlike Rust's `ciac
/// verify`, `sim_inner` never runs a target's full `validate` sequence
/// before driving the runner, so this installs dependencies itself
/// rather than assuming a prior `npm ci` already happened.
///
/// 28UpdatePlan.md M7b: dispatches to [`sim_drive_typescript_multi`] for
/// a multi-service system, mirroring [`sim_drive_rust`]'s own
/// `_single`/`_multi` split exactly.
fn sim_drive_typescript(
    out: &Path,
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    // v0.29 M4: absolutize before it crosses a subprocess-cwd boundary
    // -- see the identical fix and comment on `sim_drive_python`.
    let projects = find_project_dirs(&resolve_path(out)?, "package.json")?;
    if projects.is_empty() {
        bail!(
            "no generated typescript project found under {}",
            out.display()
        );
    }
    if let [project_dir] = projects.as_slice() {
        return sim_drive_typescript_single(project_dir, scenarios, wall_timeout);
    }
    sim_drive_typescript_multi(out, &projects, scenarios, wall_timeout)
}

/// The single-service path, unchanged in shape from before 28's M7b
/// (`sim_drive_typescript` used to be this function's own body directly).
fn sim_drive_typescript_single(
    project_dir: &Path,
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    if !project_dir.join("src/sim_runner.ts").exists() {
        bail!(
            "no `src/sim_runner.ts` in {} -- this program declares nothing v0.23 M9's \
             simulation world can fake (no `db`/`queue` use); see docs/simulation.md",
            project_dir.display()
        );
    }
    run_in(project_dir, "npm", &["ci"])?;
    run_in(project_dir, "npm", &["run", "build"])?;
    run_node_sim_runner(project_dir, scenarios, wall_timeout)
}

/// 28UpdatePlan.md M7b: N services, one shared world, one process --
/// `system-runner/` (see `system_sim_runner.ts.j2`'s own doc comment) is
/// a normal npm package `TsBackend::generate` already wrote with `file:`
/// dependencies on every service package plus `sim-shared`. Unlike Rust
/// (where `cargo build` inside `system-runner/` resolves the whole
/// path-dependency graph itself), npm's `file:` dependencies are not
/// transitively built -- confirmed live in the scratchpad (see M7a's own
/// Shipped note): each service still needs its own independent `npm ci
/// && npm run build` to produce the `dist/*.js`+`.d.ts` `system-runner`
/// imports, so this function builds `sim-shared`, then every service,
/// then `system-runner` itself, in that order, before driving it.
fn sim_drive_typescript_multi(
    out: &Path,
    projects: &[PathBuf],
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    let runner_dir = out.join("system-runner");
    if !runner_dir.join("src/sim_runner.ts").exists() {
        bail!(
            "no `system-runner/src/sim_runner.ts` under {} -- this system declares nothing \
             28UpdatePlan.md M7's simulation world can fake in any service (no `db`/`queue`/... \
             use anywhere); see docs/simulation.md (found {} generated project(s) under this \
             system's --out)",
            out.display(),
            projects.len()
        );
    }
    let sim_shared_dir = out.join("sim-shared");
    run_in(&sim_shared_dir, "npm", &["ci"])?;
    run_in(&sim_shared_dir, "npm", &["run", "build"])?;
    for project_dir in projects {
        run_in(project_dir, "npm", &["ci"])?;
        run_in(project_dir, "npm", &["run", "build"])?;
    }
    run_in(&runner_dir, "npm", &["ci"])?;
    run_in(&runner_dir, "npm", &["run", "build"])?;
    run_node_sim_runner(&runner_dir, scenarios, wall_timeout)
}

/// Shared `node dist/sim_runner.js <scenario>` drive loop both
/// [`sim_drive_typescript_single`] and [`sim_drive_typescript_multi`]
/// funnel through once their respective builds are done -- the same
/// one-line-JSON-on-stdout protocol every generated runner speaks.
fn run_node_sim_runner(
    project_dir: &Path,
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    let mut scenario_outcomes = Vec::new();
    for scenario_path in scenarios {
        let scenario_abs = resolve_path(scenario_path)?;
        let mut cmd = Command::new("node");
        cmd.arg("dist/sim_runner.js")
            .arg(&scenario_abs)
            .current_dir(project_dir);

        let output = run_captured(&mut cmd, wall_timeout).with_context(|| {
            format!(
                "failed to run sim_runner for scenario {}",
                scenario_path.display()
            )
        })?;
        if !output.stderr.is_empty() {
            use std::io::Write;
            let _ = std::io::stderr().write_all(&output.stderr);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().next_back().unwrap_or("").trim();
        if last_line.is_empty() {
            bail!(
                "sim_runner for scenario {} exited with {} and printed no result on stdout",
                scenario_path.display(),
                output.status
            );
        }
        let outcome: crate::json_out::SimScenarioOutcome = serde_json::from_str(last_line)
            .with_context(|| {
                format!(
                    "cannot parse sim_runner's result for scenario {}: {last_line:?}",
                    scenario_path.display()
                )
            })?;
        scenario_outcomes.push(outcome);
    }
    Ok(scenario_outcomes)
}

/// Drives every `--scenario` against a generated Go project's own
/// `cmd/sim_runner/main.go` (v0.24 M9) -- same generated-runner shape
/// as [`sim_drive_rust`]/[`sim_drive_typescript`] (the runner is
/// generated code baked into the project itself, compiled and run once
/// per scenario, one-line JSON reply on stdout), different toolchain:
/// `go build` then `go run` the runner's own package path. `cmd/
/// sim_runner` is a second `main` package Go's own `go build ./...`/`go
/// vet ./...`/`go test ./...` already discover automatically (the same
/// way Cargo auto-discovers `src/bin/sim_runner.rs`), so unlike Rust's
/// `--bin sim_runner` flag, no special package selector is needed
/// beyond the package path itself.
///
/// 28UpdatePlan.md M7d: dispatches to [`sim_drive_go_multi`] for a
/// multi-service system, mirroring [`sim_drive_rust`]/
/// [`sim_drive_typescript`]'s own `_single`/`_multi` split exactly.
fn sim_drive_go(
    out: &Path,
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    // v0.29 M4: absolutize before it crosses a subprocess-cwd boundary
    // -- see the identical fix and comment on `sim_drive_python`.
    let projects = find_project_dirs(&resolve_path(out)?, "go.mod")?;
    if projects.is_empty() {
        bail!("no generated go project found under {}", out.display());
    }
    if let [project_dir] = projects.as_slice() {
        return sim_drive_go_single(project_dir, scenarios, wall_timeout);
    }
    sim_drive_go_multi(out, &projects, scenarios, wall_timeout)
}

/// The single-service path, unchanged in shape from before 28's M7d
/// (`sim_drive_go` used to be this function's own body directly).
fn sim_drive_go_single(
    project_dir: &Path,
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    if !project_dir.join("cmd/sim_runner/main.go").exists() {
        bail!(
            "no `cmd/sim_runner/main.go` in {} -- this program declares nothing v0.24 M9's \
             simulation world can fake (no `db`/`queue` use); see docs/simulation.md",
            project_dir.display()
        );
    }
    run_in(
        project_dir,
        "go",
        &["build", "-o", "/dev/null", "./cmd/sim_runner"],
    )?;
    run_go_sim_runner(
        project_dir,
        &["run", "./cmd/sim_runner"],
        scenarios,
        wall_timeout,
    )
}

/// 28UpdatePlan.md M7c/M7d: N services, one shared world, one process --
/// `system-runner/` (see `system_sim_runner.go.j2`'s own doc comment) is
/// a normal Go module `GoBackend::generate` already wrote with `replace`
/// directives pointing at `../sim-shared` and every service directory.
/// Unlike single-service Go's `cmd/sim_runner` sub-package, `system-
/// runner`'s own runner is a plain package-root `main.go` (there is only
/// ever one `main` package in this module, so no sub-package split was
/// needed the way single-service Go's `cmd/api` vs `cmd/sim_runner`
/// split avoids two `main` packages colliding). Unlike TypeScript's
/// `file:` dependencies (which are not transitively built by npm --
/// M7b's own finding), `go build`/`go run` inside `system-runner/`
/// resolves the whole `replace`-directive dependency graph unaided, the
/// same as Rust's own Cargo path-dependency resolution in
/// [`sim_drive_rust_multi`] -- so there is nothing to build ahead of
/// time in any other project directory first.
fn sim_drive_go_multi(
    out: &Path,
    projects: &[PathBuf],
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    let runner_dir = out.join("system-runner");
    if !runner_dir.join("main.go").exists() {
        bail!(
            "no `system-runner/main.go` under {} -- this system declares nothing \
             28UpdatePlan.md M7's simulation world can fake in any service (no `db`/`queue`/... \
             use anywhere); see docs/simulation.md (found {} generated project(s) under this \
             system's --out)",
            out.display(),
            projects.len()
        );
    }
    run_in(&runner_dir, "go", &["build", "-o", "/dev/null", "."])?;
    run_go_sim_runner(&runner_dir, &["run", "."], scenarios, wall_timeout)
}

/// Shared `go run <args> -- <scenario>` drive loop both
/// [`sim_drive_go_single`] and [`sim_drive_go_multi`] funnel through
/// once their respective builds are done -- the same one-line-JSON-on-
/// stdout protocol every generated runner speaks. `args` carries the
/// caller's own `go run` package selector (`./cmd/sim_runner` for
/// single-service, `.` for `system-runner`).
fn run_go_sim_runner(
    project_dir: &Path,
    args: &[&str],
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    let mut scenario_outcomes = Vec::new();
    for scenario_path in scenarios {
        let scenario_abs = resolve_path(scenario_path)?;
        let mut cmd = Command::new("go");
        cmd.args(args).arg(&scenario_abs).current_dir(project_dir);

        let output = run_captured(&mut cmd, wall_timeout).with_context(|| {
            format!(
                "failed to run sim_runner for scenario {}",
                scenario_path.display()
            )
        })?;
        if !output.stderr.is_empty() {
            use std::io::Write;
            let _ = std::io::stderr().write_all(&output.stderr);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().next_back().unwrap_or("").trim();
        if last_line.is_empty() {
            bail!(
                "sim_runner for scenario {} exited with {} and printed no result on stdout",
                scenario_path.display(),
                output.status
            );
        }
        let outcome: crate::json_out::SimScenarioOutcome = serde_json::from_str(last_line)
            .with_context(|| {
                format!(
                    "cannot parse sim_runner's result for scenario {}: {last_line:?}",
                    scenario_path.display()
                )
            })?;
        scenario_outcomes.push(outcome);
    }
    Ok(scenario_outcomes)
}

/// Whether `project_dir` has a generated `SimRunner.java` anywhere
/// under `src/test/java` -- unlike Go's fixed `cmd/sim_runner/
/// main.go` path, Java's own runner lives at a package-qualified path
/// (`src/test/java/com/ciac/<pkg>/sim/SimRunner.java`) `commands.rs`
/// doesn't otherwise know, so this walks rather than checking one
/// literal path.
fn java_sim_runner_exists(project_dir: &Path) -> bool {
    fn walk(path: &Path) -> bool {
        let Ok(entries) = std::fs::read_dir(path) else {
            return false;
        };
        for entry in entries.flatten() {
            let child = entry.path();
            if child.is_dir() {
                if walk(&child) {
                    return true;
                }
            } else if child.file_name().is_some_and(|n| n == "SimRunner.java") {
                return true;
            }
        }
        false
    }
    walk(&project_dir.join("src/test/java"))
}

/// Drives every `--scenario` against a generated Java project's own
/// `src/test/java/.../sim/SimRunner.java` (v0.25 M9) -- the same
/// shape as `sim_drive_rust`/`sim_drive_typescript`/`sim_drive_go`
/// (the runner is generated code baked into the project itself,
/// compiled and run once per scenario, one-line JSON reply on
/// stdout), different toolchain: `./mvnw test-compile` (build once --
/// `SimRunner` is test-scoped, since `MockMvc`/`spring-test` never sit
/// on the packaged application's own classpath) then `./mvnw
/// exec:java` (the pom's own `exec-maven-plugin`, preconfigured with
/// `SimRunner`'s main class and the `test` classpath scope) once per
/// scenario.
///
/// 28UpdatePlan.md M8d: dispatches to [`sim_drive_java_multi`] for a
/// multi-service system, mirroring [`sim_drive_rust`]/
/// [`sim_drive_typescript`]/[`sim_drive_go`]'s own `_single`/`_multi`
/// split exactly.
fn sim_drive_java(
    out: &Path,
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    // v0.29 M4: absolutize before it crosses a subprocess-cwd boundary
    // -- see the identical fix and comment on `sim_drive_python`.
    let projects = find_project_dirs(&resolve_path(out)?, "pom.xml")?;
    if projects.is_empty() {
        bail!("no generated java project found under {}", out.display());
    }
    if let [project_dir] = projects.as_slice() {
        return sim_drive_java_single(project_dir, scenarios, wall_timeout);
    }
    sim_drive_java_multi(out, &projects, scenarios, wall_timeout)
}

/// The single-service path, unchanged in shape from before 28's M8d
/// (`sim_drive_java` used to be this function's own body directly).
fn sim_drive_java_single(
    project_dir: &Path,
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    if !java_sim_runner_exists(project_dir) {
        bail!(
            "no `SimRunner.java` under {} -- this program declares nothing v0.25 M9's simulation \
             world can fake (no `db`/`queue` use); see docs/simulation.md",
            project_dir.display()
        );
    }
    run_in(project_dir, "./mvnw", &["-q", "-B", "test-compile"])?;

    let mut scenario_outcomes = Vec::new();
    for scenario_path in scenarios {
        let scenario_abs = resolve_path(scenario_path)?;
        let scenario_arg = format!("-Dexec.args={}", scenario_abs.display());
        let mut cmd = Command::new("./mvnw");
        cmd.arg("-q")
            .arg("-B")
            .arg("exec:java")
            .arg(&scenario_arg)
            .current_dir(project_dir);

        let output = run_captured(&mut cmd, wall_timeout).with_context(|| {
            format!(
                "failed to run SimRunner for scenario {}",
                scenario_path.display()
            )
        })?;
        if !output.stderr.is_empty() {
            use std::io::Write;
            let _ = std::io::stderr().write_all(&output.stderr);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().next_back().unwrap_or("").trim();
        if last_line.is_empty() {
            bail!(
                "SimRunner for scenario {} exited with {} and printed no result on stdout",
                scenario_path.display(),
                output.status
            );
        }
        let outcome: crate::json_out::SimScenarioOutcome = serde_json::from_str(last_line)
            .with_context(|| {
                format!(
                    "cannot parse SimRunner's result for scenario {}: {last_line:?}",
                    scenario_path.display()
                )
            })?;
        scenario_outcomes.push(outcome);
    }
    Ok(scenario_outcomes)
}

/// 28UpdatePlan.md M8c/M8d: N services, one shared world, one process --
/// `system-runner/SystemSimRunner.java` (see the template's own doc
/// comment) is a single source file `JavaBackend::generate` already
/// wrote at the system root. Unlike Go's `sim-shared` module (resolved
/// via a `go.mod` `replace` directive) or TypeScript's `sim-shared` npm
/// package (resolved via a `file:` dependency), Java has no reactor/
/// aggregator pom joining N independently-built Maven services here
/// (28UpdatePlan.md M8c's own packaging decision, "Option B" --
/// surveyed against a reactor-pom alternative and rejected because it
/// would need `mvn install` to populate `~/.m2` or a `<modules>`
/// reactor, either of which would cause golden churn on every existing
/// single-service Java example): each service's own Maven build stays
/// completely untouched, and this driver instead assembles one joined
/// classpath by hand across every service -- `target/classes` +
/// `target/test-classes` (populated by that service's own `./mvnw
/// test-compile`) plus that service's own dependency jars (`./mvnw
/// dependency:build-classpath`) -- then compiles `SystemSimRunner.java`
/// directly with `javac` against it and runs it with a plain `java -cp`,
/// no Maven at all for the run phase. This is the most faithful analogue
/// of Python's own `PYTHONPATH`-union approach in
/// [`sim_drive_python_multi`] of any of the five targets' own
/// multi-service drivers.
fn sim_drive_java_multi(
    out: &Path,
    projects: &[PathBuf],
    scenarios: &[PathBuf],
    wall_timeout: Option<std::time::Duration>,
) -> Result<Vec<crate::json_out::SimScenarioOutcome>> {
    let runner_dir = out.join("system-runner");
    let runner_src = runner_dir.join("SystemSimRunner.java");
    if !runner_src.is_file() {
        bail!(
            "no `system-runner/SystemSimRunner.java` under {} -- this system declares nothing \
             28UpdatePlan.md M8's simulation world can fake in any service (no `db`/`queue`/... \
             use anywhere); see docs/simulation.md (found {} generated project(s) under this \
             system's --out)",
            out.display(),
            projects.len()
        );
    }

    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut classpath_entries: Vec<String> = Vec::new();
    for project_dir in projects {
        run_in(project_dir, "./mvnw", &["-q", "-B", "test-compile"])?;
        let cp_file = project_dir.join("target").join("ciac-sim-classpath.txt");
        let cp_arg = format!("-Dmdep.outputFile={}", cp_file.display());
        let status = run_streamed(
            Command::new("./mvnw")
                .args(["-q", "-B", "dependency:build-classpath", &cp_arg])
                .current_dir(project_dir),
        )
        .with_context(|| {
            format!(
                "failed to run `./mvnw dependency:build-classpath` in {}",
                project_dir.display()
            )
        })?;
        if !status.success() {
            bail!(
                "`./mvnw dependency:build-classpath` failed in {}",
                project_dir.display()
            );
        }
        let deps = std::fs::read_to_string(&cp_file)
            .with_context(|| format!("reading Maven classpath output {}", cp_file.display()))?;
        classpath_entries.push(project_dir.join("target/classes").display().to_string());
        classpath_entries.push(
            project_dir
                .join("target/test-classes")
                .display()
                .to_string(),
        );
        classpath_entries.push(deps.trim().to_owned());
    }
    let joined_classpath = classpath_entries.join(sep);

    let compiled_dir = runner_dir.join("target/classes");
    std::fs::create_dir_all(&compiled_dir)
        .with_context(|| format!("creating {}", compiled_dir.display()))?;
    let compiled_dir_str = compiled_dir
        .to_str()
        .context("system-runner output directory is not valid UTF-8")?;
    run_in(
        &runner_dir,
        "javac",
        &[
            "-cp",
            &joined_classpath,
            "-d",
            compiled_dir_str,
            "SystemSimRunner.java",
        ],
    )?;

    let full_classpath = format!("{}{sep}{joined_classpath}", compiled_dir.display());
    let mut scenario_outcomes = Vec::new();
    for scenario_path in scenarios {
        let scenario_abs = resolve_path(scenario_path)?;
        let mut cmd = Command::new("java");
        cmd.args(["-cp", &full_classpath, "SystemSimRunner"])
            .arg(&scenario_abs)
            .current_dir(&runner_dir);

        let output = run_captured(&mut cmd, wall_timeout).with_context(|| {
            format!(
                "failed to run SystemSimRunner for scenario {}",
                scenario_path.display()
            )
        })?;
        if !output.stderr.is_empty() {
            use std::io::Write;
            let _ = std::io::stderr().write_all(&output.stderr);
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let last_line = stdout.lines().next_back().unwrap_or("").trim();
        if last_line.is_empty() {
            bail!(
                "SystemSimRunner for scenario {} exited with {} and printed no result on stdout",
                scenario_path.display(),
                output.status
            );
        }
        let outcome: crate::json_out::SimScenarioOutcome = serde_json::from_str(last_line)
            .with_context(|| {
                format!(
                    "cannot parse SystemSimRunner's result for scenario {}: {last_line:?}",
                    scenario_path.display()
                )
            })?;
        scenario_outcomes.push(outcome);
    }
    Ok(scenario_outcomes)
}

#[allow(clippy::too_many_arguments)]
pub fn sim(
    file: &Path,
    target: &str,
    out: &Path,
    scenarios: &[PathBuf],
    record: Option<&Path>,
    replay: Option<&Path>,
    name: Option<String>,
    json: bool,
) -> Result<ExitCode> {
    if json {
        let (envelope, code) = sim_envelope(file, target, out, scenarios, record, replay, name)?;
        crate::json_out::emit(&envelope);
        return Ok(code);
    }

    let result = sim_inner(file, target, out, scenarios, record, replay, name, None)?;
    let mut all_passed = true;
    for outcome in &result.scenarios {
        if outcome.passed {
            println!("[PASS] {}", outcome.scenario);
        } else {
            all_passed = false;
            println!(
                "[FAIL] {}: {}",
                outcome.scenario,
                outcome.error.as_deref().unwrap_or("unknown error")
            );
        }
    }
    Ok(if all_passed {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// [`sim`]'s JSON path as a value.
#[allow(clippy::too_many_arguments)]
pub(crate) fn sim_envelope(
    file: &Path,
    target: &str,
    out: &Path,
    scenarios: &[PathBuf],
    record: Option<&Path>,
    replay: Option<&Path>,
    name: Option<String>,
) -> Result<(crate::json_out::Envelope, ExitCode)> {
    JSON_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    let (ir, has_errors, sources, diags) = front_end_quiet(file)?;
    if has_errors || ir.is_none() {
        let envelope = crate::json_out::envelope("sim", false, &diags, &sources);
        return Ok((envelope, ExitCode::FAILURE));
    }
    let (sim_result, success) =
        match sim_inner(file, target, out, scenarios, record, replay, name, None) {
            Ok(result) => {
                let success = result.scenarios.iter().all(|outcome| outcome.passed);
                (Some(result), success)
            }
            Err(err) => {
                eprintln!("error: {err:#}");
                (None, false)
            }
        };
    let mut envelope = crate::json_out::envelope("sim", success, &diags, &sources);
    envelope.sim = sim_result;
    let code = if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };
    Ok((envelope, code))
}

/// `verify --sim`'s wall-clock cap for a human at a terminal is
/// unbounded (`None`, same as plain `ciac sim`); `ciac mcp`'s
/// `verify_sim` tool passes [`MCP_SIM_WALL_TIMEOUT`] and caps scenario
/// count at [`MCP_SIM_MAX_SCENARIOS`] instead — see
/// [`verify_sim_envelope`].
pub(crate) const MCP_SIM_MAX_SCENARIOS: usize = 5;
pub(crate) const MCP_SIM_WALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// `verify --sim` / `ciac mcp`'s `verify_sim` tool: static verification
/// (the same truth plain `verify --json` reports) followed by, only if
/// that passed, every listed scenario run through [`sim_inner`] against
/// the same generated project — 17UpdatePlan.md's
/// `verify = generated-project static truth` /
/// `verify_sim = bounded in-process behavioral truth` split. Never
/// requests Docker/live/keep, and never accepts a `--record`/`--replay`
/// path (`verify_sim`'s own disclosed limits: it cannot write arbitrary
/// replay artifacts). `wall_timeout`/`max_scenarios` are `None`/
/// unbounded for `verify --sim` at a terminal; the MCP tool passes the
/// fixed caps above, since an agent that hangs mid-call has no operator
/// present to interrupt it.
pub(crate) fn verify_sim_envelope(
    file: &Path,
    target: &str,
    out: &Path,
    scenarios: &[PathBuf],
    name: Option<String>,
    wall_timeout: Option<std::time::Duration>,
    max_scenarios: Option<usize>,
) -> Result<(crate::json_out::Envelope, ExitCode)> {
    JSON_MODE.store(true, std::sync::atomic::Ordering::Relaxed);
    let (ir, has_errors, sources, diags) = front_end_quiet(file)?;
    if has_errors || ir.is_none() {
        let envelope = crate::json_out::envelope("verify_sim", false, &diags, &sources);
        return Ok((envelope, ExitCode::FAILURE));
    }
    if let Some(max) = max_scenarios {
        if scenarios.len() > max {
            eprintln!(
                "error: verify_sim accepts at most {max} scenarios per call, got {}",
                scenarios.len()
            );
            let envelope = crate::json_out::envelope("verify_sim", false, &diags, &sources);
            return Ok((envelope, ExitCode::FAILURE));
        }
    }

    let (sim_result, success) =
        match verify_inner(file, target, out, false, false, false, name.clone()) {
            Ok(code) if code == ExitCode::SUCCESS => {
                match sim_inner(file, target, out, scenarios, None, None, name, wall_timeout) {
                    Ok(result) => {
                        let success = result.scenarios.iter().all(|outcome| outcome.passed);
                        (Some(result), success)
                    }
                    Err(err) => {
                        eprintln!("error: {err:#}");
                        (None, false)
                    }
                }
            }
            Ok(_) => (None, false),
            Err(err) => {
                eprintln!("error: {err:#}");
                (None, false)
            }
        };
    let mut envelope = crate::json_out::envelope("verify_sim", success, &diags, &sources);
    envelope.sim = sim_result;
    let code = if success {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    };
    Ok((envelope, code))
}

/// Boots the generated compose stack (`up -d --wait`), erroring out on
/// a non-zero exit or a missing Docker binary. `--wait-timeout` bounds
/// the wait: without it, a service stuck in "starting" (a healthcheck
/// that never resolves either way) blocks forever instead of failing
/// — there's no other signal that distinguishes "still booting" from
/// "hung" once this is running unattended in CI.
fn compose_up(compose_file: &Path) -> Result<()> {
    let status = run_streamed(
        Command::new("docker")
            .arg("compose")
            .arg("-f")
            .arg(compose_file)
            .args(["up", "-d", "--wait", "--wait-timeout", "180"]),
    )
    .map_err(|err| {
        anyhow::anyhow!("cannot run `docker compose` ({err}); this step requires Docker")
    })?;
    if !status.success() {
        anyhow::bail!("docker compose up failed ({status})");
    }
    Ok(())
}

/// Best-effort teardown of the generated compose stack — or, with
/// `keep`, leaves it running and prints how to tear it down by hand.
pub(crate) fn compose_down_or_keep(compose_file: &Path, keep: bool) {
    if keep {
        eprintln!(
            "info: --keep: leaving the stack up; stop it with `docker compose -f {} down -v --remove-orphans`",
            compose_file.display()
        );
        return;
    }
    let _ = run_streamed(
        Command::new("docker")
            .arg("compose")
            .arg("-f")
            .arg(compose_file)
            .args(["down", "-v", "--remove-orphans"]),
    );
}

/// v0.8 M4: runs the compose-backed `tests/system/` suite
/// (`ciac_codegen::system_tests`) `ciac verify --system` asks for. A
/// no-op success when the program had no whole-system edges to test
/// (`tests/system/` was never generated). Teardown always runs on
/// failure; on success, `--keep` (v0.9 M4) leaves the stack up for
/// local poking.
fn verify_system(out: &Path, keep: bool) -> Result<ExitCode> {
    let system_dir = out.join("tests/system");
    if !system_dir.exists() {
        eprintln!("info: no system-level edges to test; `--system` is a no-op here");
        return Ok(ExitCode::SUCCESS);
    }

    let compose_file = out.join("docker-compose.yml");
    let result: Result<()> = compose_up(&compose_file).and_then(|()| {
        run_in(&system_dir, "uv", &["sync", "-q"])
            .and_then(|_| run_in(&system_dir, "uv", &["run", "pytest", "-q"]))
    });

    // On failure, the services ran detached (`up -d`), so their stdout/
    // stderr never reached this process — dump it now, before teardown
    // erases the only copy, so a bare test failure doesn't hide a
    // server-side traceback.
    if result.is_err() {
        eprintln!("--- docker compose logs (service stdout/stderr) ---");
        let _ = run_streamed(
            Command::new("docker")
                .arg("compose")
                .arg("-f")
                .arg(&compose_file)
                .args(["logs", "--no-color"]),
        );
    }

    // `--keep` only applies to a green stack: a failed run always
    // tears down so it never leaves broken containers behind.
    compose_down_or_keep(&compose_file, keep && result.is_ok());

    match result {
        Ok(()) => Ok(ExitCode::SUCCESS),
        Err(err) => {
            eprintln!("error: system verification failed: {err:#}");
            Ok(ExitCode::FAILURE)
        }
    }
}

/// v0.9 M3: `--live` health probing, replacing the long-standing stub.
/// Boots the generated compose stack, polls every service's `/health`
/// route with a bounded backoff, reports per-service up/down, and
/// tears the stack down (unless `--keep`, on all-green).
fn verify_live(file: &Path, out: &Path, keep: bool) -> Result<ExitCode> {
    // Recompute the model for service names + host ports; the program
    // compiled moments ago in `generate`, so this cannot newly fail.
    let (ir, has_errors, _sources) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        anyhow::bail!("program stopped compiling between generate and --live probing");
    };
    let model = ciac_codegen::model::build_system(&ir, &GenOptions::default());
    let services: Vec<(String, u16)> = model
        .services
        .iter()
        .map(|ctx| (ctx.service_name.clone(), ctx.host_port))
        .collect();

    let compose_file = out.join("docker-compose.yml");
    if let Err(err) = compose_up(&compose_file) {
        eprintln!("error: --live probing failed: {err:#}");
        return Ok(ExitCode::FAILURE);
    }

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut all_healthy = true;
    for (service, port) in &services {
        let mut healthy = health_probe(*port);
        while !healthy && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_secs(2));
            healthy = health_probe(*port);
        }
        eprintln!(
            "live: {service} (localhost:{port}/health) {}",
            if healthy { "up" } else { "DOWN" }
        );
        all_healthy &= healthy;
    }

    compose_down_or_keep(&compose_file, keep && all_healthy);
    Ok(if all_healthy {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// One `GET /health` over a plain TcpStream — a dependency-free probe
/// is all a fixed, tiny request like this needs.
pub(crate) fn health_probe(port: u16) -> bool {
    use std::io::{Read, Write};
    let timeout = std::time::Duration::from_secs(2);
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&addr, timeout) else {
        return false;
    };
    if stream.set_read_timeout(Some(timeout)).is_err()
        || stream.set_write_timeout(Some(timeout)).is_err()
    {
        return false;
    }
    let request =
        format!("GET /health HTTP/1.1\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut response = String::new();
    if stream.read_to_string(&mut response).is_err() {
        return false;
    }
    response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200")
}

pub fn graph(file: &Path, format: &str) -> Result<ExitCode> {
    match graph_document(file, format)? {
        Some(text) => {
            if format == "dot" {
                print!("{text}");
            } else {
                println!("{text}");
            }
            Ok(ExitCode::SUCCESS)
        }
        None => Ok(ExitCode::FAILURE),
    }
}

/// [`graph`]'s rendering as a value — `None` on compile errors (the
/// CLI path renders those itself via [`front_end`]; `ciac mcp`'s
/// `graph` tool (v0.13 M5) points a caller at `check` instead).
pub(crate) fn graph_document(file: &Path, format: &str) -> Result<Option<String>> {
    let (ir, has_errors, _sources) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        return Ok(None);
    };
    let text = match format {
        "json" => serde_json::to_string_pretty(&ir)?,
        "dot" => ir.to_dot(),
        other => bail!("unknown format `{other}`"),
    };
    Ok(Some(text))
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

/// v0.10 M2: prints the JSON Schema for the external-backend wire
/// contract — derived from the same Rust types that serialize the real
/// payloads, so it cannot drift from what `ciac` actually sends and
/// accepts. `docs/protocol-schema.json` is this output checked in,
/// held identical by an integration test.
pub fn codegen_schema() -> Result<ExitCode> {
    println!(
        "{}",
        serde_json::to_string_pretty(&ciac_codegen::protocol::schema_document())?
    );
    Ok(ExitCode::SUCCESS)
}

/// v0.18 M1: prints the JSON Schema for the checked-in semantic
/// baseline document — mirrors [`codegen_schema`]'s pattern.
/// `docs/semantic-baseline-schema.json` is this output checked in, held
/// identical by an integration test.
pub fn semantic_baseline_schema() -> Result<ExitCode> {
    println!(
        "{}",
        serde_json::to_string_pretty(&ciac_codegen::semantic_model::baseline_schema_document())?
    );
    Ok(ExitCode::SUCCESS)
}

/// The default checked-in baseline path for an entry file with no
/// explicit `--out` (18UpdatePlan.md Pillar 1): `<entry-dir>/.ciac/
/// baselines/<entry-stem>.semantic.json`.
fn default_baseline_path(file: &Path) -> std::path::PathBuf {
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "main".to_owned());
    dir.join(".ciac")
        .join("baselines")
        .join(format!("{stem}.semantic.json"))
}

/// Writes `baseline` to `path` via a sibling temporary file and atomic
/// rename (18UpdatePlan.md Pillar 1: "writes use a sibling temporary
/// file and atomic replacement") — a crash mid-write can never leave a
/// half-written baseline where a reader expects a complete one.
fn write_baseline_atomically(
    path: &Path,
    baseline: &ciac_codegen::semantic_model::SemanticBaseline,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(baseline)?;
    let tmp_path = path.with_extension(format!(
        "{}.tmp-{}",
        path.extension().and_then(|e| e.to_str()).unwrap_or("json"),
        std::process::id()
    ));
    std::fs::write(&tmp_path, [bytes, b"\n".to_vec()].concat())
        .with_context(|| format!("cannot write {}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

/// `ciac baseline <file> [--out <path>] [--update] [--accept-breaking]
/// [--reason <text>]` (v0.18 M1, 18UpdatePlan.md Pillar 1): creates or
/// replaces the checked-in semantic baseline generated CI gates on.
///
/// Lifecycle: first creation always succeeds; an unchanged
/// `semantic_hash` recreation is a true no-op (the file is never
/// rewritten, so it stays byte-identical); any detected architecture
/// change requires `--update`. Distinguishing a *breaking* change from
/// an additive/internal one (and therefore only requiring
/// `--accept-breaking` for the former) is v0.18 M2's classifier, which
/// doesn't exist yet — until then, this milestone conservatively
/// requires `--accept-breaking` alongside `--update` for *any* detected
/// change, disclosed here rather than silently guessed at.
pub fn baseline(
    file: &Path,
    out: Option<&Path>,
    update: bool,
    accept_breaking: bool,
    reason: Option<&str>,
) -> Result<ExitCode> {
    let (ir, has_errors, sources) = front_end(file)?;
    let Some(ir) = ir.filter(|_| !has_errors) else {
        return Ok(ExitCode::FAILURE);
    };
    let source_hash = {
        let mut buf = String::new();
        for f in sources.files() {
            buf.push_str(&f.name);
            buf.push('\0');
            buf.push_str(&f.src);
            buf.push('\0');
        }
        hash_bytes(buf.as_bytes())
    };
    let model = ciac_codegen::semantic_model::SemanticModel::from_ir(&ir);
    let entry = file.display().to_string();
    let new_baseline = ciac_codegen::semantic_model::SemanticBaseline::new(
        env!("CARGO_PKG_VERSION"),
        entry,
        source_hash,
        model,
    );

    let out_path = out
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_baseline_path(file));

    match std::fs::read(&out_path) {
        Ok(existing_bytes) => {
            let existing: ciac_codegen::semantic_model::SemanticBaseline =
                serde_json::from_slice(&existing_bytes).with_context(|| {
                    format!("{} is not a valid semantic baseline", out_path.display())
                })?;
            if existing.semantic_baseline_version
                > ciac_codegen::semantic_model::SEMANTIC_BASELINE_VERSION
            {
                bail!(
                    "{} was written by a newer, incompatible baseline format (version {}, this \
                     build understands up to {}) -- refusing to guess at its contents",
                    out_path.display(),
                    existing.semantic_baseline_version,
                    ciac_codegen::semantic_model::SEMANTIC_BASELINE_VERSION
                );
            }
            if existing.semantic_hash == new_baseline.semantic_hash {
                eprintln!(
                    "{}: unchanged (semantic_hash {})",
                    out_path.display(),
                    new_baseline.semantic_hash
                );
                return Ok(ExitCode::SUCCESS);
            }
            if !update {
                bail!(
                    "{} already exists and the architecture changed (semantic_hash {} -> {}); \
                     pass --update to replace it",
                    out_path.display(),
                    existing.semantic_hash,
                    new_baseline.semantic_hash
                );
            }
            if !accept_breaking {
                bail!(
                    "{} would change (semantic_hash {} -> {}); pass --accept-breaking to \
                     confirm -- v0.18 M1's baseline lifecycle doesn't yet classify a change's \
                     severity (that's v0.18 M2), so every detected change conservatively needs \
                     explicit confirmation",
                    out_path.display(),
                    existing.semantic_hash,
                    new_baseline.semantic_hash
                );
            }
            write_baseline_atomically(&out_path, &new_baseline)?;
            if let Some(reason) = reason {
                append_intentional_break_changelog(file, &existing, &new_baseline, reason)?;
            }
            eprintln!(
                "{}: updated ({} -> {})",
                out_path.display(),
                existing.semantic_hash,
                new_baseline.semantic_hash
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            write_baseline_atomically(&out_path, &new_baseline)?;
            eprintln!(
                "{}: created (semantic_hash {})",
                out_path.display(),
                new_baseline.semantic_hash
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => Err(err).with_context(|| format!("cannot read {}", out_path.display())),
    }
}

/// Appends a one-line entry to source-owned `CHANGELOG.ciac.md`
/// (18UpdatePlan.md Pillar 4) alongside an accepted-breaking baseline
/// update — the human-reviewable trail a generated CI job's
/// `ciac-breaking: <reason>` commit trailer cross-checks against. Lives
/// next to the baseline's entry file so a multi-service project keeps
/// one changelog per baseline rather than one at the repo root.
fn append_intentional_break_changelog(
    entry: &Path,
    before: &ciac_codegen::semantic_model::SemanticBaseline,
    after: &ciac_codegen::semantic_model::SemanticBaseline,
    reason: &str,
) -> Result<()> {
    let dir = entry.parent().unwrap_or_else(|| Path::new("."));
    let path = dir.join("CHANGELOG.ciac.md");
    let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
    if existing.is_empty() {
        existing.push_str("# Accepted breaking changes\n\n");
        existing.push_str(
            "Appended by `ciac baseline --update --accept-breaking --reason \"...\"`. \
             Each entry records why an intentionally breaking architecture change was \
             accepted into the checked-in semantic baseline.\n\n",
        );
    }
    existing.push_str(&format!(
        "- `{}` -> `{}`: {reason}\n",
        before.semantic_hash, after.semantic_hash
    ));
    std::fs::write(&path, existing).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

pub fn explain(code: &str) -> Result<ExitCode> {
    println!("{}", explain_document(code)?);
    Ok(ExitCode::SUCCESS)
}

/// [`explain`]'s rendering as a value — `ciac mcp`'s `explain` tool
/// (v0.13 M5) consumes it directly.
pub(crate) fn explain_document(code: &str) -> Result<String> {
    let Some(code) = ErrorCode::parse(code) else {
        bail!("unknown error code `{code}`; codes look like CIAC0001 (see docs/errors.md)");
    };
    Ok(format!(
        "{}: {}\n\n{}",
        code.code(),
        code.title(),
        code.explanation()
    ))
}

pub fn targets(json: bool) -> Result<ExitCode> {
    if json {
        println!("{}", serde_json::to_string_pretty(&targets_doc())?);
        return Ok(ExitCode::SUCCESS);
    }
    for backend in backends() {
        println!("{:10} {}", backend.id(), backend.description());
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(serde::Serialize)]
struct TargetsDoc {
    targets_version: u32,
    /// The compiler's own version (26UpdatePlan.md M8), distinct from
    /// `language_version` below.
    ciac_version: &'static str,
    /// The frozen CIaC language surface version this compiler build
    /// implements (`LANGUAGE_VERSION`, via `ciac_syntax`).
    language_version: &'static str,
    targets: Vec<TargetEntry>,
}

#[derive(serde::Serialize)]
struct TargetEntry {
    id: &'static str,
    description: &'static str,
    /// Always `"internal"` for anything `ciac targets --json` can
    /// actually enumerate — external-protocol backends (v0.8 M2+) are
    /// resolved dynamically from `$PATH` at generate time and have no
    /// fixed registry entry to list here; see docs/external-backends.md.
    kind: &'static str,
    project_marker: &'static str,
    validate: Vec<ValidateStepEntry>,
    sim: SimEntry,
    /// Provider capabilities this target fully implements, keyed by
    /// capability name (v0.22 M4). Sourced from `vocab::PROVIDERS`, not
    /// derived from `Backend::supports()`: `supports()` gates at
    /// `Component` (capability-kind) granularity, coarser than this
    /// field's per-provider one (it can say "this target supports
    /// `auth`", never "...specifically JWT, not OAuth2"), and every
    /// internal target's `supports()` is now unconditional `true`
    /// besides — a derivation would carry zero discriminating
    /// information regardless of target count. `PROVIDERS` stays
    /// hand-maintained, audited truthful against the real generated
    /// templates (`26UpdatePlan.md` M7 widened it from python/rust-only
    /// to all five internal targets on exactly that audit).
    capabilities: BTreeMap<&'static str, Vec<&'static str>>,
}

#[derive(serde::Serialize)]
struct ValidateStepEntry {
    program: &'static str,
    purpose: &'static str,
}

#[derive(serde::Serialize)]
#[serde(tag = "level")]
enum SimEntry {
    #[serde(rename = "full")]
    Full,
    #[serde(rename = "narrow")]
    Narrow,
    #[serde(rename = "none")]
    None { reason: &'static str },
}

fn targets_doc() -> TargetsDoc {
    let mut capabilities_by_target: BTreeMap<
        &'static str,
        BTreeMap<&'static str, Vec<&'static str>>,
    > = BTreeMap::new();
    for provider in crate::vocab::PROVIDERS {
        for &target in provider.targets {
            capabilities_by_target
                .entry(target)
                .or_default()
                .entry(provider.capability)
                .or_default()
                .push(provider.name);
        }
    }
    let targets = backends()
        .into_iter()
        .map(|backend| {
            let info = backend.target_info();
            TargetEntry {
                id: backend.id(),
                description: backend.description(),
                kind: "internal",
                project_marker: info.project_marker,
                validate: info
                    .validate
                    .iter()
                    .map(|step| ValidateStepEntry {
                        program: step.program,
                        purpose: step.purpose,
                    })
                    .collect(),
                sim: match info.sim {
                    SimSupport::Full => SimEntry::Full,
                    SimSupport::Narrow { .. } => SimEntry::Narrow,
                    SimSupport::None { reason } => SimEntry::None { reason },
                },
                capabilities: capabilities_by_target
                    .get(backend.id())
                    .cloned()
                    .unwrap_or_default(),
            }
        })
        .collect();
    TargetsDoc {
        targets_version: 1,
        ciac_version: env!("CARGO_PKG_VERSION"),
        language_version: ciac_syntax::LANGUAGE_VERSION,
        targets,
    }
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
    /// v0.18 M1: this build's canonical semantic model, stamped onto
    /// the manifest's advisory `semantic_snapshot` cache.
    semantic_snapshot: ciac_codegen::semantic_model::SemanticModel,
}

#[allow(clippy::too_many_arguments)]
fn generate(
    file: &Path,
    target: &str,
    out: &Path,
    name: Option<String>,
    k8s_image: Option<(Option<&str>, &str)>,
    terraform: Option<ciac_codegen::Profile>,
    profile: ciac_codegen::Profile,
    secrets: bool,
    ts_client: bool,
    ci: bool,
    image_prefix: Option<String>,
    semantic_gate: Option<(String, String)>,
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
    // v0.16 M4: `cardinality: one` references are fully resolved by sema
    // *and* now have real codegen (a plain FK-id field/column, migrations
    // from M3) — but `cardinality: many` is still gated: it needs a
    // separate link-table read/write path in both backends, which is
    // v0.16 M5/M6. Refuse cleanly here rather than let any of the
    // (unreachable!()-guarded) codegen match arms downstream panic.
    if let Some(name) = ir
        .records()
        .find(|(_, record)| {
            record.fields.iter().any(|f| {
                matches!(
                    f.ty,
                    FieldType::Reference {
                        cardinality: ciac_ir::Cardinality::Many,
                        ..
                    }
                )
            })
        })
        .map(|(_, record)| record.name.clone())
    {
        eprintln!(
            "error[{}]: record `{name}` has a `cardinality: many` `Reference<T>` field, which \
             no backend can generate yet (relation codegen lands in v0.16 M5/M6)",
            ErrorCode::UnsupportedConstruct
        );
        bail!("unsupported construct");
    }

    let opts = GenOptions { project_name: name };
    let mut project = backend.generate(&ir, &opts)?;

    // v0.15 M2: opt-in TypeScript client (`ciac build --client ts`),
    // independent of `--target` — it talks to whichever backend serves
    // the program's routes over HTTP, so it's generated from the same
    // `SystemModel` every backend renders from rather than per-backend.
    if ts_client {
        let system = ciac_codegen::model::build_system(&ir, &opts);
        for (path, content) in ciac_codegen::ts_client::build(&system) {
            project.add_file(path, content);
        }
    }

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
        for (path, content) in
            ciac_codegen::k8s::build(&ir, image_prefix, image_tag, profile, secrets)
        {
            project.add_file(path, content);
        }
    }
    if let Some(tf_profile) = terraform {
        for (path, content) in ciac_codegen::terraform::build(&ir, tf_profile) {
            project.add_file(path, content);
        }
    }
    // v0.15 M5: opt-in GitHub Actions workflow (`ciac build --deploy
    // ci`) — mirrors what `ciac verify` runs locally, plus an image
    // build/push and a compose smoke job.
    if ci {
        let gate =
            semantic_gate
                .as_ref()
                .map(|(source_file, baseline)| ciac_codegen::ci::SemanticGate {
                    source_file,
                    baseline,
                });
        for (path, content) in ciac_codegen::ci::build(
            &ir,
            backend.target_info().ci_test_steps,
            image_prefix.as_deref(),
            gate,
        ) {
            project.add_file(path, content);
        }
    }

    // v0.13 M5: an agent-facing front door into the generated tree
    // itself, alongside the human-facing notes above. Every build
    // target gets one, so `build`/`diff`/`verify` all account for it
    // (regeneration owns it exactly like any other compiler-owned
    // file — see `report_regen_plan`'s conflict/drift handling).
    project.add_file("AGENTS.md", agents_md(backend.id()));

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
            add_migration_files(&mut project, backend.as_ref(), next_migration_seq, &sql);
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

    let semantic_snapshot = ciac_codegen::semantic_model::SemanticModel::from_ir(&ir);

    Ok(Generated {
        backend,
        project,
        source_hash,
        tables: new_tables,
        next_migration_seq,
        records: new_records,
        semantic_snapshot,
    })
}

/// The generated tree's own `AGENTS.md` (v0.13 M5) — regenerated
/// alongside every other compiler-owned file, so it can never drift
/// out of date the way a hand-written note would.
fn agents_md(target: &str) -> String {
    // v0.22 M1: was `target == "python"` — `Full` sim support matches
    // exactly python today (rust is `Narrow`, so it still falls to the
    // `else` branch below, unchanged), and an unregistered/external
    // target falls to `else` too, same as before.
    let full_sim = backends()
        .into_iter()
        .find(|b| b.id() == target)
        .is_some_and(|b| matches!(b.target_info().sim, SimSupport::Full));
    let sim_section = if full_sim {
        "\n\
## Fast inner loop vs. outer truth (v0.17)\n\
\n\
`ciac sim <source.ciac> --target python --out . --scenario <file>` runs\n\
a portable scenario (`ciac_sim::Scenario` JSON) against this project's\n\
real code with in-memory fakes standing in for the database, broker,\n\
cache, object store, email, search, and external HTTP — no Docker, no\n\
network, no wall-clock sleep. `verify --sim` layers the same run on top\n\
of static verification. Blunt version: `sim` is the fast logic/topology\n\
loop you run constantly; `verify --system` (real provider containers)\n\
is the merge bar. A green `sim` run does not prove SQL dialect fidelity,\n\
broker durability, cryptography, or real network behavior — see\n\
docs/simulation.md's claim boundary before treating one as a substitute\n\
for the other.\n"
    } else {
        ""
    };
    format!(
        "\
# Agents working in this generated project\n\
\n\
This tree was generated by `ciac build` (target: `{target}`) from a\n\
`.ciac` source file. Two kinds of files live here, and the difference\n\
matters:\n\
\n\
- **Compiler-owned** (most files): rewritten by every `ciac build`.\n\
  Hand edits are lost on the next build — `ciac diff`/`ciac verify`\n\
  detect and report them as drift (CIAC0033) rather than silently\n\
  discarding your change.\n\
- **Seeded** (extern handler implementations and migration files —\n\
  `app/services/*.py` on the python target, `src/services/*.rs` on\n\
  rust): generated once, then yours. `ciac build` never overwrites an\n\
  existing seeded file; if the seed it *would* generate changes, it\n\
  preserves your file and writes the new seed to a `.ciac-new`\n\
  sidecar for manual reconciliation (CIAC0034).\n\
\n\
**Where logic goes**: write `extern handler` bodies in the seeded\n\
service module for the handler's name. Everything else in this tree\n\
is wiring generated from the `.ciac` source — change the behavior by\n\
editing the source and rebuilding, not by editing generated wiring.\n\
\n\
## The truth signal\n\
\n\
`ciac verify <source.ciac> --target {target} --out .` regenerates from\n\
the current source into this directory and runs the generated\n\
project's own test suite. A green exit code means the source and this\n\
tree agree *and* the generated code actually works. Add `--system`\n\
(requires Docker) to also prove cross-service edges and capability\n\
round-trips against a booted stack.\n\
{sim_section}\n\
## Machine-readable output\n\
\n\
`ciac check|build|diff|verify --json` (run from beside the `.ciac`\n\
source) each print one JSON envelope on stdout — human narration\n\
stays on stderr. `ciac describe` prints the language's full\n\
vocabulary as one versioned JSON document. `ciac mcp` exposes the\n\
same commands as a Model Context Protocol server over stdio (including\n\
`verify_sim` on the python target).\n\
"
    )
}

/// Adds the diffed migration SQL, under each generated deployable
/// project's migration directory (there may be more than one in a
/// multi-service system; every service context already carries the
/// program's full table set, so each gets the same migration file).
/// Migration files use `FileRole::Migration` (v0.27 M9; write-once
/// like `Seeded`): once written, later builds that stop re-emitting a
/// given sequence number leave the on-disk file alone
/// (`RegenStatus::OrphanLeft`) rather than deleting it, and — unlike
/// a stale `Seeded` scaffold — that state is the permanent, expected
/// steady state once the schema stops changing, so it does not warn
/// (found live: a plain, unchanged `ciac build`/`ciac sim` was
/// warning about every prior migration on every single run).
fn add_migration_files(project: &mut GeneratedProject, backend: &dyn Backend, seq: u32, sql: &str) {
    let target_info = backend.target_info();
    let filename = (target_info.migration_filename)(seq, "migration");
    let rel = format!("{}/{filename}", target_info.migrations_dir);
    for prefix in service_roots(project, target_info) {
        project.add_migration_file(format!("{prefix}{rel}"), sql.to_owned());
    }
}

/// The prefix (possibly empty) of each deployable project inside a
/// generated tree, identified by its marker file (`pyproject.toml` /
/// `Cargo.toml`) — one root for a single-service build, several for a
/// multi-service system.
fn service_roots(
    project: &GeneratedProject,
    target_info: &ciac_codegen::TargetInfo,
) -> Vec<String> {
    let marker = target_info.project_marker;
    let mut roots: Vec<String> = project
        .files_with_roles()
        .filter_map(|(path, _, _)| path.strip_suffix(marker).map(str::to_owned))
        // 28UpdatePlan.md M6b/M6c: `sim-shared/Cargo.toml` (the vendored-
        // simulation crate every Rust service in a multi-service system
        // depends on by path) and `system-runner/Cargo.toml` (the
        // generated scenario-driver crate depending on all of them)
        // both satisfy this same marker check without being a
        // deployable service in their own right -- neither owns a
        // table, so neither must ever receive its own copy of the
        // system's migration files the way every real service root does.
        .filter(|root| root != "sim-shared/" && root != "system-runner/")
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
            // A migration's `OrphanLeft` state is permanent and
            // expected (see `FileRole::Migration`), not a stale
            // scaffold to flag -- silent, same as `Unchanged`.
            RegenStatus::OrphanLeft if entry.role == ciac_codegen::FileRole::Migration => {}
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

/// Unified diff text for a changed entry; `None` when content is
/// identical (nothing to show).
fn patch_text(entry: &ciac_codegen::regen::RegenEntry) -> Option<String> {
    let old = entry.old_content.as_deref().unwrap_or_default();
    let new = entry.new_content.as_deref().unwrap_or_default();
    if old == new {
        return None;
    }
    let diff = similar::TextDiff::from_lines(old, new);
    Some(
        diff.unified_diff()
            .header(&format!("a/{}", entry.path), &format!("b/{}", entry.path))
            .to_string(),
    )
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

/// Runs each generated project's own `TargetInfo::validate` steps in
/// order (v0.22 M1 — replaces the `validate_python_project`/
/// `validate_rust_project` pair with one registry-driven loop; the
/// per-target step lists themselves are unchanged, see
/// `ciac-backend-python`/`ciac-backend-rust`'s `TARGET_INFO`).
/// `target` is resolved through the *built-in* registry only, exactly
/// as before this milestone — an external target still refuses here
/// with the same message, since `ExternalBackend` was never a member
/// of `backends()`.
fn validate_generated(root: &Path, target: &str) -> Result<ExitCode> {
    let target_info = backends()
        .into_iter()
        .find(|b| b.id() == target)
        .map(|b| b.target_info())
        .ok_or_else(|| anyhow!("cannot verify unknown generated target `{target}`"))?;
    let projects = find_project_dirs(root, target_info.project_marker)?;
    if projects.is_empty() {
        bail!(
            "no generated {target} project found under {}",
            root.display()
        );
    }
    for project in projects {
        for step in target_info.validate {
            run_validate_step(&project, step)?;
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Runs one `TargetInfo::validate` step, streaming through the same
/// `--json`-aware plumbing every other subprocess in this module uses
/// (v0.22 M1 — previously the Rust backend's `cargo check` step used a
/// raw, non-`--json`-aware `Command::status()` call; unifying it here
/// is intentional, not a behavior regression: it now matches how every
/// other validate step, including Rust's own `cargo test`, already
/// behaved).
fn run_validate_step(project: &Path, step: &ValidateStep) -> Result<()> {
    let mut cmd = Command::new(step.program);
    cmd.args(step.args).current_dir(project);
    for (key, value) in step.env {
        cmd.env(key, value);
    }
    let status = run_streamed(&mut cmd).with_context(|| {
        format!(
            "failed to run `{} {}` in {}",
            step.program,
            step.args.join(" "),
            project.display()
        )
    })?;
    if !status.success() {
        bail!(
            "`{} {}` failed in {} ({})",
            step.program,
            step.args.join(" "),
            project.display(),
            step.purpose
        );
    }
    Ok(())
}

fn run_in(project: &Path, program: &str, args: &[&str]) -> Result<()> {
    let status =
        run_streamed(Command::new(program).args(args).current_dir(project)).with_context(|| {
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
                // 28UpdatePlan.md M6b: `sim-shared/` (the vendored-
                // simulation crate a multi-service Rust system's own
                // services depend on by path) has a `Cargo.toml` like
                // any real service, but is a library, not a deployable
                // project -- it has no `src/bin/sim_runner.rs`, no
                // routes, nothing `ciac verify`/`ciac sim`'s per-service
                // walk could ever run.
                if child.file_name().is_some_and(|name| name == "sim-shared") {
                    continue;
                }
                // 28UpdatePlan.md M6c: `system-runner/` (the generated
                // multi-service scenario driver -- see `system_sim_
                // runner.rs.j2`'s own doc comment) has a `Cargo.toml`
                // too, but it is `ciac sim`'s own driver binary for this
                // system, not a deployable service in its own right --
                // no routes of its own, nothing `ciac verify`'s
                // per-service walk should run against it, and `ciac sim`
                // drives it directly (see `sim_drive_rust_multi`) rather
                // than discovering it through this walk.
                if child
                    .file_name()
                    .is_some_and(|name| name == "system-runner")
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

#[cfg(test)]
mod tests {
    use super::health_probe;
    use std::io::{Read, Write};

    /// A minimal one-shot HTTP server on an ephemeral port, answering
    /// every request with the given status line.
    fn serve_once(status_line: &'static str) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("ephemeral port");
        let port = listener.local_addr().expect("bound").port();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let _ = stream.write_all(
                    format!("{status_line}\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                        .as_bytes(),
                );
            }
        });
        port
    }

    #[test]
    fn health_probe_accepts_a_real_200() {
        let port = serve_once("HTTP/1.1 200 OK");
        assert!(health_probe(port));
    }

    #[test]
    fn health_probe_rejects_a_500() {
        let port = serve_once("HTTP/1.1 500 Internal Server Error");
        assert!(!health_probe(port));
    }

    #[test]
    fn health_probe_rejects_a_closed_port() {
        // Bind-then-drop guarantees the port exists but nothing listens.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("ephemeral port");
            listener.local_addr().expect("bound").port()
        };
        assert!(!health_probe(port));
    }

    /// v0.22 M4: `ciac targets --json`'s shape is a downstream contract
    /// (docs build, plans 23-25's checklists, any agent calling MCP
    /// `describe`) — mirrors `describe_json_is_stable_shape`'s pattern.
    #[test]
    fn targets_json_is_stable_shape() {
        let json = serde_json::to_value(super::targets_doc()).unwrap();
        assert_eq!(json["targets_version"], 1);
        assert_eq!(json["ciac_version"], env!("CARGO_PKG_VERSION"));
        assert_eq!(json["language_version"], ciac_syntax::LANGUAGE_VERSION);
        let targets = json["targets"].as_array().expect("targets array");
        assert!(targets.iter().any(|t| t["id"] == "python"), "{targets:?}");
        assert!(targets.iter().any(|t| t["id"] == "rust"), "{targets:?}");
        for target in targets {
            for key in [
                "id",
                "description",
                "kind",
                "project_marker",
                "validate",
                "sim",
                "capabilities",
            ] {
                assert!(
                    target.get(key).is_some(),
                    "missing key `{key}` on {target:?}"
                );
            }
            assert_eq!(target["kind"], "internal");
        }
    }

    /// Guards `vendor/pyrunner/`'s own reason for existing: every file
    /// already vendored there must stay byte-identical to its
    /// `sim/pyrunner/*.py` original. The file list itself is *derived*
    /// from `vendor/pyrunner/`'s own directory listing, not hardcoded
    /// here -- `scripts/sync-vendored-ciac-assets.sh` derives its list
    /// the same way, so the two can never independently drift out of
    /// sync with each other. Runs only inside the workspace, where the
    /// repo-root `sim/` directory is reachable relative to
    /// `CARGO_MANIFEST_DIR` -- never true when building from a
    /// published crate's own package tarball, which contains only the
    /// vendored copy this test would have nothing to compare it
    /// against. If this fails, run `scripts/sync-vendored-ciac-
    /// assets.sh` and re-vendor.
    #[test]
    fn vendored_pyrunner_matches_source() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let pyrunner_src = std::path::Path::new(manifest_dir).join("../../sim/pyrunner");
        if !pyrunner_src.is_dir() {
            return;
        }
        let vendor_dir = std::path::Path::new(manifest_dir).join("vendor/pyrunner");
        for entry in std::fs::read_dir(&vendor_dir)
            .unwrap_or_else(|e| panic!("reading {}: {e}", vendor_dir.display()))
        {
            let entry = entry.expect("reading vendor/pyrunner entry");
            let name = entry.file_name();
            let source = std::fs::read_to_string(pyrunner_src.join(&name))
                .unwrap_or_else(|e| panic!("reading sim/pyrunner/{name:?}: {e}"));
            let vendored = std::fs::read_to_string(entry.path())
                .unwrap_or_else(|e| panic!("reading vendor/pyrunner/{name:?}: {e}"));
            assert_eq!(
                source, vendored,
                "vendor/pyrunner/{name:?} has drifted from sim/pyrunner/{name:?} -- \
                 run scripts/sync-vendored-ciac-assets.sh"
            );
        }
    }
}
