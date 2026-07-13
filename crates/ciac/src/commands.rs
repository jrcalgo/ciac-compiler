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
fn front_end_quiet(file: &Path) -> Result<(Option<NormalizedIr>, bool, SourceMap, Diagnostics)> {
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
    for kind in &deploy.deploy {
        match kind.as_str() {
            "k8s" => k8s_image = Some((deploy.image_prefix.as_deref(), deploy.image_tag.as_str())),
            "terraform" => terraform = Some(profile),
            other => bail!("unknown --deploy target `{other}`; available: k8s, terraform"),
        }
    }
    let mut ts_client = false;
    for kind in &client {
        match kind.as_str() {
            "ts" => ts_client = true,
            other => bail!("unknown --client target `{other}`; available: ts"),
        }
    }

    let Generated {
        backend,
        project,
        source_hash,
        tables,
        next_migration_seq,
        records,
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
    name: Option<String>,
    json: bool,
) -> Result<ExitCode> {
    if json {
        let (envelope, code) = with_json_envelope("verify", file, || {
            verify_inner(file, target, out, live, system, keep, name)
        })?;
        crate::json_out::emit(&envelope);
        return Ok(code);
    }
    verify_inner(file, target, out, live, system, keep, name)
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
    )?;

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

/// The generated tree's own `AGENTS.md` (v0.13 M5) — regenerated
/// alongside every other compiler-owned file, so it can never drift
/// out of date the way a hand-written note would.
fn agents_md(target: &str) -> String {
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
\n\
## Machine-readable output\n\
\n\
`ciac check|build|diff|verify --json` (run from beside the `.ciac`\n\
source) each print one JSON envelope on stdout — human narration\n\
stays on stderr. `ciac describe` prints the language's full\n\
vocabulary as one versioned JSON document. `ciac mcp` exposes the\n\
same commands as a Model Context Protocol server over stdio.\n\
"
    )
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
}
