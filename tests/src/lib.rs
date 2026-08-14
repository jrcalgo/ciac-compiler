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

/// Splits `items` into up to `worker_count` round-robin buckets, sizes
/// differing by at most one -- `33UpdatePlan.md` M3's own fix, shared
/// here so M4's `openapi.rs`/`conformance.rs` and M5's `golden.rs`
/// don't each redefine it. Contiguous fixed-size chunks (`.chunks(len
/// / n)`) leave the last worker with fewer items and idle for the
/// run's tail once the busier workers still have theirs; round-robin
/// keeps every worker's item count within one of every other's.
/// Failure messages that use this already carry the full example path
/// and backend id, so round-robin costs no attribution -- there is no
/// per-worker grouping anyone reads.
///
/// Generic over the item type (not just `PathBuf`, despite the name
/// kept for call-site continuity with M3/M4): M5's `golden.rs` chunks
/// `(usize, PathBuf)` pairs so each worker's output can be sorted back
/// into original example order afterward.
pub fn chunk_paths<T>(items: Vec<T>, worker_count: usize) -> Vec<Vec<T>> {
    let worker_count = worker_count.max(1).min(items.len().max(1));
    let mut chunks: Vec<Vec<T>> = (0..worker_count).map(|_| Vec::new()).collect();
    for (i, item) in items.into_iter().enumerate() {
        chunks[i % worker_count].push(item);
    }
    chunks
}

/// Longest-Processing-Time-first scheduling: sorts `items` descending by
/// `weight`, then greedily assigns each one to whichever worker bucket
/// currently carries the least total weight. Follow-up to `chunk_paths`'
/// own round-robin, which balances item *count*, not item *cost* --
/// measured to matter on this corpus specifically: example source files
/// span a 34x size range (191 bytes to 6,550 bytes), so a round-robin
/// split can hand one worker several of the largest examples while
/// another gets the smallest, leaving it idle for the run's tail.
/// `weight` is the caller's cost proxy -- file byte size for `.ciac`
/// sources, cheap to obtain and a reasonable zeroth-order stand-in for
/// generation cost without requiring a compile first.
pub fn chunk_by_weight<T>(mut items: Vec<(T, u64)>, worker_count: usize) -> Vec<Vec<T>> {
    let worker_count = worker_count.max(1).min(items.len().max(1));
    items.sort_by(|a, b| b.1.cmp(&a.1));
    let mut chunks: Vec<Vec<T>> = (0..worker_count).map(|_| Vec::new()).collect();
    let mut totals = vec![0u64; worker_count];
    for (item, weight) in items {
        let (lightest, total) = totals
            .iter_mut()
            .enumerate()
            .min_by_key(|(_, total)| **total)
            .expect("worker_count clamped to at least 1");
        chunks[lightest].push(item);
        *total += weight;
    }
    chunks
}

/// A `.ciac` source file's byte length, used as `chunk_by_weight`'s cost
/// proxy. `1` (never `0`) on a read failure so a missing/unreadable file
/// still gets scheduled rather than panicking a worker-assignment pass
/// over something a later `compile_file` call will report properly.
pub fn file_weight(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(1).max(1)
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
