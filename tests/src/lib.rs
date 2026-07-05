//! Shared helpers for the workspace-level integration tests.

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

/// The registered backends, mirroring the CLI registry.
pub fn backends() -> Vec<Box<dyn ciac_codegen::Backend>> {
    vec![
        Box::new(ciac_backend_python::PythonBackend),
        Box::new(ciac_backend_rust::RustBackend),
    ]
}
