//! Shared helpers for the workspace-level integration tests.

pub mod bench;

use ciac_diagnostics::{Diagnostics, SourceMap};
use ciac_ir::NormalizedIr;
use std::path::{Path, PathBuf};

/// Runs the full front-end on source text.
pub fn compile(src: &str) -> (Option<NormalizedIr>, Diagnostics) {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ciac", src);
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::parse(src, file, &mut diags);
    let ir = ciac_sema::analyze(&program, &mut diags);
    diags.sort();
    (ir, diags)
}

/// Compiles a `.ciac` file, panicking on invalid programs — for tests
/// operating on the known-good examples. Resolves `import "path";`
/// (v0.8 M1) the same way `ciac`'s CLI does, so an entry file's
/// imported fragments are part of the compiled program, not silently
/// dropped as unresolved `Item::Import`s.
pub fn compile_file(path: &Path) -> NormalizedIr {
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::load(path, &mut sources, &mut diags)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    let ir = ciac_sema::analyze(&program, &mut diags);
    diags.sort();
    ir.unwrap_or_else(|| panic!("{} failed to compile: {:?}", path.display(), diags.codes()))
}

/// Repository-level `examples/` directory.
pub fn examples_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples")
}

/// This crate's `ui/` directory of intentionally-invalid programs.
pub fn ui_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("ui")
}

/// All `.ciac` files in a directory, sorted for determinism.
pub fn ciac_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", dir.display()))
        .filter_map(|entry| {
            let path = entry.expect("readable dir entry").path();
            (path.extension().is_some_and(|e| e == "ciac")).then_some(path)
        })
        .collect();
    files.sort();
    files
}

/// Splits `paths` into up to `worker_count` round-robin buckets, sizes
/// differing by at most one -- `33UpdatePlan.md` M3's own fix, shared
/// here so M4's `openapi.rs`/`conformance.rs` and M5's `golden.rs`
/// don't each redefine it. Contiguous fixed-size chunks (`.chunks(len
/// / n)`) leave the last worker with fewer items and idle for the
/// run's tail once the busier workers still have theirs; round-robin
/// keeps every worker's item count within one of every other's.
/// Failure messages that use this already carry the full example path
/// and backend id, so round-robin costs no attribution -- there is no
/// per-worker grouping anyone reads.
pub fn chunk_paths(paths: Vec<PathBuf>, worker_count: usize) -> Vec<Vec<PathBuf>> {
    let worker_count = worker_count.max(1).min(paths.len().max(1));
    let mut chunks: Vec<Vec<PathBuf>> = (0..worker_count).map(|_| Vec::new()).collect();
    for (i, path) in paths.into_iter().enumerate() {
        chunks[i % worker_count].push(path);
    }
    chunks
}

/// This process's usable worker count for test-suite-internal
/// parallelism (`33UpdatePlan.md` M3-M5): `available_parallelism()`
/// capped at 4, since 8-way concurrent formatter invocations measured
/// no better than 4-way on the reference machine, so a low cap costs
/// nothing and bounds oversubscription against libtest's own
/// function-level concurrency running on top of it.
pub fn worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(4)
}

/// Renders a generated project as one stable string for snapshotting.
pub fn project_dump(project: &ciac_codegen::GeneratedProject) -> String {
    let mut out = String::new();
    for (path, content) in project.files() {
        out.push_str("============================================================\n");
        out.push_str(path);
        out.push('\n');
        out.push_str("============================================================\n");
        out.push_str(content);
        if !content.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// The registered backends, mirroring the CLI registry — every target,
/// at whatever `supports()` gate its own milestone plan has reached.
/// Registry-driven suites (`conformance.rs`, `golden.rs`,
/// `targets_cli.rs`) want this: they assert properties that hold
/// automatically for whatever's registered, gate or no gate.
pub fn backends() -> Vec<Box<dyn ciac_codegen::Backend>> {
    vec![
        Box::new(ciac_backend_python::PythonBackend),
        Box::new(ciac_backend_rust::RustBackend),
        Box::new(ciac_backend_ts::TsBackend),
        Box::new(ciac_backend_go::GoBackend),
        Box::new(ciac_backend_java::JavaBackend),
    ]
}

/// The two backends `gating.rs`'s "both backends" suite means by that
/// name: Python and Rust reached full ontology parity across v0.11-
/// v0.17, before any third target existed, and those tests assert a
/// historical claim about exactly these two, not "every backend the
/// registry happens to contain today" — a fresh target (TypeScript,
/// Go, Java) is *expected* to fail most of them until its own
/// milestone plan un-gates the construct in question; that is what
/// `supports()`'s narrow-then-widen discipline means; `backends()`'s
/// growing registry must not make this suite flaky as new targets
/// land mid-arc.
pub fn full_parity_backends() -> Vec<Box<dyn ciac_codegen::Backend>> {
    vec![
        Box::new(ciac_backend_python::PythonBackend),
        Box::new(ciac_backend_rust::RustBackend),
    ]
}
