//! `ciac dev` (v0.13 M4): the watch loop.
//!
//! Watches the entry file, every file it (transitively) imports (the
//! resolved [`SourceMap`] set), and the generated project's seeded
//! `services/` sources; on change it recompiles, regenerates through
//! the same sidecar-safe path `ciac build` uses, restarts the compose
//! stack, and re-probes every service's `/health` route.
//!
//! Two properties are load-bearing:
//!
//! * **Compile errors never kill the loop or the running stack.**
//!   Diagnostics render and the last good system keeps serving.
//! * **Regeneration is the `ciac build` path**, manifest and sidecar
//!   discipline included — `ciac dev` can never clobber an edit that
//!   `ciac build` would have protected.
//!
//! Restarts are deliberately whole-stack (`docker compose up -d
//! --build`): compose itself only recreates containers whose images
//! or configuration actually changed, which keeps the conservative
//! choice cheap in practice. Per-service restart heuristics are out
//! of scope for v0.13.
//!
//! Known edge (inherent to `--poll` mode): the polling watcher
//! baselines on its first tick after registration, so a save landing
//! inside that ~500ms window is absorbed into the baseline — the
//! *next* save picks it up. Set `CIAC_DEV_TRACE=1` to log every raw
//! filesystem event when diagnosing missed rebuilds.

use anyhow::{Context, Result};
use notify::Watcher;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crate::commands;

#[allow(clippy::too_many_arguments)]
pub fn run(
    file: &Path,
    target: &str,
    out: &Path,
    name: Option<String>,
    keep: bool,
    no_docker: bool,
    poll: bool,
) -> Result<ExitCode> {
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = Arc::clone(&stop);
        ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst))
            .context("cannot install the Ctrl-C handler")?;
    }

    let mut watch_set = rebuild(file, target, out, name.clone(), no_docker)?;

    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let mut watcher: Box<dyn Watcher> = if poll {
        let tx = tx.clone();
        Box::new(notify::PollWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            notify::Config::default().with_poll_interval(Duration::from_millis(500)),
        )?)
    } else {
        let tx = tx.clone();
        Box::new(notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        })?)
    };
    let mut registered: BTreeSet<PathBuf> = BTreeSet::new();
    register(watcher.as_mut(), &watch_set, out, &mut registered);

    eprintln!(
        "dev: watching {} source file(s) + seeded services (Ctrl-C to stop)",
        watch_set.len()
    );

    loop {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok(event)) if trace(&event) && is_relevant(&event, &watch_set, out) => {
                // Debounce: editors write in bursts; absorb the rest
                // of the burst before rebuilding.
                while rx.recv_timeout(Duration::from_millis(300)).is_ok() {}
                eprintln!("dev: change detected — rebuilding");
                match rebuild(file, target, out, name.clone(), no_docker) {
                    Ok(new_set) => {
                        register(watcher.as_mut(), &new_set, out, &mut registered);
                        watch_set = new_set;
                    }
                    Err(err) => eprintln!("dev: rebuild failed: {err:#}"),
                }
                // Regeneration itself touches the output tree; drop
                // whatever that produced so it can't echo back.
                while rx.try_recv().is_ok() {}
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    eprintln!("dev: stopping");
    if !no_docker {
        commands::compose_down_or_keep(&out.join("docker-compose.yml"), keep);
    }
    Ok(ExitCode::SUCCESS)
}

/// One dev-loop cycle: compile (rendering diagnostics), regenerate
/// through the `ciac build` path, restart + probe the stack. Always
/// returns the current watch set — on compile errors, the set still
/// covers every file the failed compile touched, so a fix anywhere
/// (including a newly added import) re-triggers the loop.
fn rebuild(
    file: &Path,
    target: &str,
    out: &Path,
    name: Option<String>,
    no_docker: bool,
) -> Result<BTreeSet<PathBuf>> {
    let (ir, has_errors, sources) = commands::front_end(file)?;
    let watch_set = watch_files(&sources);
    if has_errors || ir.is_none() {
        eprintln!("dev: compile errors — keeping the last good build; fix and save to retry");
        return Ok(watch_set);
    }
    let ir = ir.expect("checked above");

    let code = commands::build_inner(
        file,
        target,
        out,
        false,
        false,
        commands::DeployOpts {
            deploy: Vec::new(),
            image_prefix: None,
            image_tag: "latest".to_owned(),
            profile: "dev".to_owned(),
            secrets: false,
        },
        Vec::new(),
        name,
    )?;
    if code != ExitCode::SUCCESS {
        eprintln!("dev: regeneration failed — keeping the last good build");
        return Ok(watch_set);
    }

    if no_docker {
        eprintln!("dev: regenerated (--no-docker: not starting the stack)");
        return Ok(watch_set);
    }

    let compose_file = out.join("docker-compose.yml");
    let status = std::process::Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg(&compose_file)
        .args(["up", "-d", "--build", "--wait"])
        .status()
        .map_err(|err| {
            anyhow::anyhow!("cannot run `docker compose` ({err}); this step requires Docker")
        })?;
    if !status.success() {
        eprintln!("dev: docker compose up failed ({status}) — fix and save to retry");
        return Ok(watch_set);
    }

    let model = ciac_codegen::model::build_system(&ir, &ciac_codegen::GenOptions::default());
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    for ctx in &model.services {
        let mut healthy = commands::health_probe(ctx.host_port);
        while !healthy && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_secs(2));
            healthy = commands::health_probe(ctx.host_port);
        }
        eprintln!(
            "dev: {} (localhost:{}/health) {}",
            ctx.service_name,
            ctx.host_port,
            if healthy { "up" } else { "DOWN" }
        );
    }
    Ok(watch_set)
}

/// The resolved source set as canonical filesystem paths. Virtual
/// entries (`std/...` blueprints, `registry:` imports) have no file
/// to watch and drop out via the failed canonicalize.
fn watch_files(sources: &ciac_diagnostics::SourceMap) -> BTreeSet<PathBuf> {
    sources
        .files()
        .filter_map(|f| Path::new(&f.name).canonicalize().ok())
        .collect()
}

/// Watches each source file directly (a polling watcher only detects
/// content changes on paths it stats itself — a non-recursive
/// directory watch sees child additions/removals, not child edits),
/// every containing directory non-recursively (so the delete+rename
/// dance most editors save with still lands), and the generated
/// seeded-services directories recursively.
fn register(
    watcher: &mut dyn Watcher,
    watch_set: &BTreeSet<PathBuf>,
    out: &Path,
    registered: &mut BTreeSet<PathBuf>,
) {
    let mut flat: BTreeSet<PathBuf> = watch_set.clone();
    flat.extend(
        watch_set
            .iter()
            .filter_map(|f| f.parent().map(Path::to_path_buf)),
    );
    for path in flat {
        if registered.insert(path.clone()) {
            if let Err(err) = watcher.watch(&path, notify::RecursiveMode::NonRecursive) {
                eprintln!("dev: cannot watch {}: {err}", path.display());
            }
        }
    }
    for seeded in [out.join("app/services"), out.join("src/services")] {
        if seeded.is_dir() && registered.insert(seeded.clone()) {
            if let Err(err) = watcher.watch(&seeded, notify::RecursiveMode::Recursive) {
                eprintln!("dev: cannot watch {}: {err}", seeded.display());
            }
        }
    }
}

/// Logs every filesystem event when `CIAC_DEV_TRACE` is set — the
/// first question in any "why didn't it rebuild" report. Always true,
/// so it can sit in the match guard without changing behavior.
fn trace(event: &notify::Event) -> bool {
    if std::env::var_os("CIAC_DEV_TRACE").is_some() {
        eprintln!("dev: event {:?} {:?}", event.kind, event.paths);
    }
    true
}

/// A change matters when it touches a watched source file or a seeded
/// service source in the output tree.
fn is_relevant(event: &notify::Event, watch_set: &BTreeSet<PathBuf>, out: &Path) -> bool {
    if matches!(event.kind, notify::EventKind::Access(_)) {
        return false;
    }
    let seeded = [out.join("app/services"), out.join("src/services")];
    event.paths.iter().any(|p| {
        let canonical = p.canonicalize().unwrap_or_else(|_| p.clone());
        watch_set.contains(&canonical)
            || seeded.iter().any(|dir| {
                canonical.starts_with(dir)
                    && matches!(
                        canonical.extension().and_then(|e| e.to_str()),
                        Some("py" | "rs")
                    )
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watch_files_keeps_real_files_and_drops_virtual_entries() {
        let dir = std::env::temp_dir().join(format!("ciac-dev-watch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let real = dir.join("main.ciac");
        std::fs::write(&real, "service S;\n").unwrap();

        let mut sources = ciac_diagnostics::SourceMap::new();
        sources.add_file(real.display().to_string(), "service S;\n".to_owned());
        sources.add_file("std/crud.ciac".to_owned(), "blueprint ...".to_owned());
        sources.add_file(
            "registry:acme/x/y.ciac@v1".to_owned(),
            "record R { id: Uuid; }".to_owned(),
        );

        let set = watch_files(&sources);
        assert_eq!(set.len(), 1, "{set:?}");
        assert!(set.contains(&real.canonicalize().unwrap()));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn relevance_filters_to_watched_sources_and_seeded_services() {
        let dir = std::env::temp_dir().join(format!("ciac-dev-rel-{}", std::process::id()));
        let out = dir.join("build");
        std::fs::create_dir_all(out.join("app/services")).unwrap();
        let source = dir.join("main.ciac");
        std::fs::write(&source, "service S;\n").unwrap();
        let seeded = out.join("app/services/handler.py");
        std::fs::write(&seeded, "pass\n").unwrap();
        let noise = out.join("app/routes.py");
        std::fs::write(&noise, "pass\n").unwrap();

        let watch_set: BTreeSet<PathBuf> = [source.canonicalize().unwrap()].into();
        let event = |p: &Path| notify::Event {
            kind: notify::EventKind::Modify(notify::event::ModifyKind::Any),
            paths: vec![p.to_path_buf()],
            attrs: Default::default(),
        };

        assert!(is_relevant(&event(&source), &watch_set, &out));
        assert!(is_relevant(&event(&seeded), &watch_set, &out));
        assert!(
            !is_relevant(&event(&noise), &watch_set, &out),
            "compiler-owned output files must not re-trigger the loop"
        );

        std::fs::remove_dir_all(&dir).ok();
    }
}
