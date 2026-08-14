//! External backend protocol M2: a [`Backend`] that shells out to an
//! external `ciac-backend-<target>` executable instead of running
//! linked-in Rust code — the seam protobuf's `protoc-gen-<lang>`
//! plugins use, applied to CIaC's already-shared [`crate::model`].
//!
//! Speaks [`crate::protocol::CodegenRequest`]/[`CodegenResponse`] over
//! the child's stdin/stdout. `stderr` is inherited, not captured, so
//! an external backend's own diagnostics show up live — the same
//! treatment `commands.rs` already gives `docker compose`/`uv`/`cargo`
//! subprocesses elsewhere in this codebase.

use crate::model::build_system;
use crate::protocol::{CodegenRequest, CodegenResponse, PROTOCOL_VERSION};
use crate::{
    Backend, BackendError, DevCommands, FileRole, GenOptions, GeneratedProject, RestartStyle,
    SimSupport, TargetInfo,
};
use ciac_ir::{Component, NormalizedIr};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, Stdio};

/// A generic, permissive `TargetInfo` for external-protocol backends
/// (v0.22 M1): they have no compile-time-known `TargetInfo` of their
/// own, and `ciac verify`'s static validation loop
/// (`commands.rs::validate_generated`) never reaches this value — it
/// resolves targets through the *built-in* registry only (`python`/
/// `rust`), so an external target is refused there exactly as before
/// this milestone (`ExternalBackend` is never a member of that
/// registry). This value exists only to satisfy the `Backend` trait
/// and to give the CI/compose/migrations plumbing something harmless
/// to read if it's ever handed an external target directly. The
/// `project_marker`/`migrations_dir` values match this crate's
/// pre-v0.22 non-Python fallback (`_ => "Cargo.toml"` /
/// `_ => "migrations"`) so any codepath that *does* consult them sees
/// the same behavior as before.
static EXTERNAL_TARGET_INFO: TargetInfo = TargetInfo {
    project_marker: "Cargo.toml",
    migrations_dir: "migrations",
    migration_filename: |seq, _slug| format!("{seq:04}_migration.sql"),
    validate: &[],
    validate_parallel_from: None,
    ci_test_steps: crate::ci::GENERIC_TEST_STEPS,
    compose: crate::compose::BackendComposeOpts {
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
    source_extension: "",
    sim: SimSupport::None {
        reason: "external-protocol backends have no simulation wire surface (v0.8 M2 non-goal)",
    },
    sim_replay: false,
};

/// A backend resolved by name at the moment it's used, not registered
/// up front — `--target <name>` falls back to this when `name` isn't
/// one of the built-in targets. See `commands.rs::generate()`.
#[derive(Debug)]
pub struct ExternalBackend {
    target: String,
    /// The executable to run. A bare name (no path separators, the
    /// common case from [`ExternalBackend::new`]) still gets the OS's
    /// own `$PATH` search when handed to [`Command::new`] — this field
    /// only needs to hold something other than that when a caller
    /// wants to bypass `$PATH` and point at a specific binary
    /// directly, e.g. `tests/tests/external_backend.rs` pointing at
    /// the compiled `stub-backend` fixture without touching `$PATH` or
    /// process-wide env vars (both awkward in a multi-threaded test
    /// binary, and mutating env vars now requires `unsafe`, which this
    /// workspace forbids outright).
    executable: OsString,
}

impl ExternalBackend {
    /// Resolves `ciac-backend-<target>` via `$PATH` at generation time
    /// — the normal, production path.
    pub fn new(target: impl Into<String>) -> Self {
        let target = target.into();
        let executable = format!("ciac-backend-{target}").into();
        Self { target, executable }
    }

    /// Runs a specific executable directly, bypassing `$PATH`
    /// resolution — for callers (tests, or a future explicit
    /// `--backend-path` flag) that already know exactly which binary
    /// to run.
    pub fn with_executable(target: impl Into<String>, executable: impl AsRef<Path>) -> Self {
        Self {
            target: target.into(),
            executable: executable.as_ref().as_os_str().to_owned(),
        }
    }
}

impl Backend for ExternalBackend {
    fn id(&self) -> &'static str {
        // Leaked once per process invocation of this specific target —
        // `Backend::id` is `&'static str` everywhere else because
        // built-in backends are singletons with compile-time-known
        // names; an externally named target only exists at runtime, so
        // this is the one place in the trait that genuinely needs a
        // runtime string coerced to fit the existing signature rather
        // than widening `Backend::id`'s return type for every
        // implementor over one caller's sake.
        Box::leak(self.target.clone().into_boxed_str())
    }

    fn description(&self) -> &'static str {
        "external backend (resolved via $PATH at build time)"
    }

    fn supports(&self, _component: &Component) -> bool {
        // No capability-negotiation protocol (v0.8 M2 non-goal): an
        // external backend is trusted to attempt everything and fail
        // loudly — a non-zero exit with a clear stderr message — for
        // whatever it can't handle, rather than `ciac` pre-filtering
        // via a second request/response round trip it has no way to
        // interpret generically across arbitrary external targets.
        true
    }

    fn target_info(&self) -> &'static TargetInfo {
        &EXTERNAL_TARGET_INFO
    }

    fn generate(
        &self,
        ir: &NormalizedIr,
        opts: &GenOptions,
    ) -> Result<GeneratedProject, BackendError> {
        let system = build_system(ir, opts);
        let request = CodegenRequest::new(&self.target, opts.project_name.clone(), system);
        let payload = serde_json::to_vec(&request)
            .map_err(|err| BackendError::Other(format!("cannot serialize request: {err}")))?;

        // Display form only — the actual spawn uses `self.executable`
        // directly (which may be an absolute path from
        // `with_executable`, not just a bare `$PATH`-searched name).
        let exe = self.executable.to_string_lossy().into_owned();
        let mut child = Command::new(&self.executable)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|err| spawn_error(&self.target, &exe, err))?;

        // The request payload can exceed a typical OS pipe buffer
        // (~64KB; M1's live proof measured ~37KB for a five-service
        // program, and real systems will exceed that) — writing it on
        // a separate thread while the main thread waits for output
        // avoids the classic deadlock where both sides block on a full
        // pipe at once.
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let writer = std::thread::spawn(move || -> io::Result<()> {
            stdin.write_all(&payload)?;
            Ok(())
        });

        let output = child
            .wait_with_output()
            .map_err(|err| BackendError::Other(format!("`{exe}` failed to run: {err}")))?;

        if !output.status.success() {
            // A write error on the stdin-writer thread almost always
            // means the child exited before reading everything (e.g.
            // it crashed early) — which this exit-status check already
            // catches, so the writer's own result is only surfaced
            // here, as extra context, not as an independent failure.
            let write_note = match writer.join() {
                Ok(Ok(())) => String::new(),
                Ok(Err(err)) => format!(" (writing its request also failed: {err})"),
                Err(_) => " (the request-writer thread panicked)".to_owned(),
            };
            return Err(BackendError::Other(format!(
                "`{exe}` exited with {}{write_note} (see its stderr output above)",
                output.status
            )));
        }

        writer
            .join()
            .map_err(|_| BackendError::Other(format!("`{exe}`'s request-writer thread panicked")))?
            .map_err(|err| {
                BackendError::Other(format!("cannot write request to `{exe}`: {err}"))
            })?;

        let response: CodegenResponse = serde_json::from_slice(&output.stdout).map_err(|err| {
            let snippet = String::from_utf8_lossy(&output.stdout);
            let snippet = snippet.chars().take(200).collect::<String>();
            BackendError::Other(format!(
                "`{exe}` did not write a valid CodegenResponse to stdout: {err} \
                 (first 200 chars of its output: {snippet:?})"
            ))
        })?;

        if response.protocol_version != PROTOCOL_VERSION {
            return Err(BackendError::Other(format!(
                "`{exe}` speaks protocol version {}, but this ciac speaks version {PROTOCOL_VERSION}",
                response.protocol_version
            )));
        }

        let mut project = GeneratedProject::new();
        for file in response.files {
            match file.role {
                FileRole::Owned => project.add_file(file.path, file.content),
                FileRole::Seeded => project.add_seeded_file(file.path, file.content),
                FileRole::Migration => project.add_migration_file(file.path, file.content),
            }
        }
        project.notes.extend(response.notes);
        Ok(project)
    }
}

fn spawn_error(target: &str, exe: &str, err: io::Error) -> BackendError {
    if err.kind() == io::ErrorKind::NotFound {
        BackendError::Other(format!(
            "unknown target `{target}`; not a built-in target, and no `{exe}` executable found on $PATH"
        ))
    } else {
        BackendError::Other(format!("cannot run `{exe}`: {err}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_target_with_no_executable_is_a_clear_error() {
        let backend = ExternalBackend::new("definitely-not-a-real-target-xyz");
        let ir_src = "service Notes;\nuse { db Postgres; }\nrecord Note { id: Uuid; title: String; }\ncrud Note;\n";
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", ir_src);
        let mut diags = ciac_diagnostics::Diagnostics::new();
        let program = ciac_syntax::parse(ir_src, file, &mut diags);
        let ir = ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()));

        let err = backend
            .generate(&ir, &GenOptions::default())
            .expect_err("no such executable exists");
        let message = err.to_string();
        assert!(
            message.contains("definitely-not-a-real-target-xyz"),
            "{message}"
        );
        assert!(
            message.contains("ciac-backend-definitely-not-a-real-target-xyz"),
            "{message}"
        );
    }
}
