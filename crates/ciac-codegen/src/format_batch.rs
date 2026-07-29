//! Shared batch-formatter seam (`30UpdatePlan.md` M3).
//!
//! `ciac-backend-java`'s own M2 finding, generalized: any backend that
//! shells out to a real formatter (rather than trusting its own Jinja
//! templates to reproduce one) pays that formatter's own startup cost
//! once per invocation. For `google-java-format` that cost is a cold
//! JVM plus javac-internals classload (~0.51s regardless of file
//! size); for `gofmt` it's ~2ms. Whether the saving is dramatic
//! (Java) or nearly free (Go), the *pattern* — one process per
//! `generate()` call, not one per file — belongs in one place so a
//! future formatter-shelling backend can't reintroduce the tax by
//! copying the wrong example. Out-of-tree backends built outside this
//! repo (`docs/external-backends.md`) cannot import this crate-private
//! helper and must implement the pattern themselves if they shell out
//! to a formatter.

use crate::{BackendError, GeneratedProject};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

/// Formats every file in `project` matching `matches` through one
/// external-process invocation.
///
/// Every matching file is written into a scratch directory *at its
/// real project-relative path* (not flattened by index) — verified
/// live for `google-java-format` before this generalized: a formatter
/// does not care whether a file's on-disk location matches its own
/// package/module declaration, so preserving the real layout sidesteps
/// that question for every future formatter this seam serves too.
/// `command_for` receives the scratch directory and the list of
/// scratch-path files already written into it, and returns a
/// configured, ready-to-run [`Command`] (its own job is only to decide
/// *how* to invoke the formatter — e.g. `gofmt -w <paths...>` vs.
/// `java -jar ... -i @argfile` — this function owns writing the files,
/// running the command, reading results back, and error remapping). It
/// may fail (an `@argfile` write, a vendored-tool materialization);
/// returning `Err` here aborts before any process is spawned.
///
/// On success, every matching file's content in `project` is replaced
/// with the formatted result via [`GeneratedProject::set_content`]. On
/// failure, the scratch directory's own paths are rewritten back to
/// the project's real paths in the formatter's error text before it is
/// returned, so a rejected file is diagnosable without knowing this
/// pass's own internals — proven for the Java caller by a dedicated
/// test that feeds it deliberately invalid source. The scratch
/// directory is removed on both the success and the error path.
pub fn format_batch(
    project: &mut GeneratedProject,
    matches: impl Fn(&str) -> bool,
    command_for: impl FnOnce(&Path, &[PathBuf]) -> Result<Command, BackendError>,
) -> Result<(), BackendError> {
    let files: Vec<(String, String)> = project
        .files()
        .filter(|(path, _)| matches(path))
        .map(|(path, content)| (path.to_owned(), content.to_owned()))
        .collect();
    if files.is_empty() {
        return Ok(());
    }

    let scratch = scratch_dir();
    let result = format_batch_in(&scratch, &files, project, command_for);
    std::fs::remove_dir_all(&scratch).ok();
    result
}

/// A scratch directory unique to this call, safe under concurrent
/// `generate()` calls from multiple threads in the same process (the
/// test suite does exactly that) and across processes (the pid
/// component).
fn scratch_dir() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("ciac-fmt-batch-{}-{n}", std::process::id()))
}

fn format_batch_in(
    scratch: &Path,
    files: &[(String, String)],
    project: &mut GeneratedProject,
    command_for: impl FnOnce(&Path, &[PathBuf]) -> Result<Command, BackendError>,
) -> Result<(), BackendError> {
    let mut path_map: HashMap<String, String> = HashMap::new();
    let mut scratch_paths = Vec::with_capacity(files.len());
    for (rel, content) in files {
        let scratch_path = scratch.join(rel);
        if let Some(parent) = scratch_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                BackendError::Other(format!("creating format-batch scratch dir: {e}"))
            })?;
        }
        std::fs::write(&scratch_path, content)
            .map_err(|e| BackendError::Other(format!("writing format-batch scratch file: {e}")))?;
        path_map.insert(scratch_path.to_string_lossy().into_owned(), rel.clone());
        scratch_paths.push(scratch_path);
    }

    let mut command = command_for(scratch, &scratch_paths)?;
    let program = command.get_program().to_string_lossy().into_owned();
    let output = command.output().map_err(|e| {
        BackendError::Other(format!(
            "`{program}` not found on PATH ({e}) — required to generate this \
             backend's output, not only to validate it"
        ))
    })?;

    if !output.status.success() {
        let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        for (scratch_path, real_path) in &path_map {
            stderr = stderr.replace(scratch_path.as_str(), real_path.as_str());
        }
        return Err(BackendError::Other(format!(
            "{program} rejected generated source (this is a codegen bug, not \
             a user error): {stderr}"
        )));
    }

    for (rel, _) in files {
        let scratch_path = scratch.join(rel);
        let formatted = std::fs::read_to_string(&scratch_path)
            .map_err(|e| BackendError::Other(format!("reading formatted output: {e}")))?;
        project.set_content(rel, formatted);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_matching_files_is_a_no_op() {
        let mut project = GeneratedProject::new();
        project.add_file("README.md", "hello");
        format_batch(
            &mut project,
            |p| p.ends_with(".java"),
            |_, _| panic!("command_for should never be called with nothing to format"),
        )
        .expect("no-op succeeds");
        assert_eq!(project.get("README.md"), Some("hello"));
    }

    #[test]
    fn remaps_scratch_paths_in_errors() {
        let mut project = GeneratedProject::new();
        project.add_file("src/Broken.txt", "unchanged");
        let err = format_batch(
            &mut project,
            |p| p.ends_with(".txt"),
            |_, paths| {
                // Echoes the scratch path to stderr, then fails — the
                // real regression this guards is a formatter error
                // naming the scratch directory instead of the real
                // generated path.
                let mut cmd = Command::new("sh");
                cmd.arg("-c")
                    .arg(format!("echo {} >&2; exit 1", paths[0].display()));
                Ok(cmd)
            },
        )
        .expect_err("the command always exits non-zero");
        let message = err.to_string();
        assert!(
            message.contains("src/Broken.txt"),
            "error should name the real path: {message}"
        );
        assert!(
            !message.contains("ciac-fmt-batch"),
            "error should not leak the scratch directory: {message}"
        );
    }
}
