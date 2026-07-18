//! v0.8 M1: multi-file programs via `import "path";`, exercised end to
//! end — real files on disk, resolved through `ciac_syntax::load`, fed
//! through the same `ciac_sema::analyze` + backend `generate()` path a
//! single-file program uses. `crates/ciac-syntax/src/module.rs`'s own
//! unit tests cover the resolution algorithm itself (merge order,
//! diamond imports, cycles); this file proves the resolved program is
//! not just structurally correct but compiles to the exact same output
//! as writing it in one file, and that cross-file diagnostics render
//! against the right file.

use ciac_codegen::GenOptions;
use ciac_diagnostics::render::{render_all, PlainRenderer};
use ciac_diagnostics::{Diagnostics, ErrorCode, SourceMap};
use ciac_integration_tests::project_dump;
use std::path::Path;

fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
    path
}

fn tmp(label: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "ciac-modules-test-{}-{}-{label}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs the same front-end `ciac`'s CLI uses (module resolution, then
/// sema) against `entry`, returning the IR and any diagnostics text.
fn compile_entry(entry: &Path) -> (Option<ciac_ir::NormalizedIr>, String) {
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::load(entry, &mut sources, &mut diags).expect("files are readable");
    let ir = ciac_sema::analyze(&program, &mut diags);
    diags.sort();
    let text = render_all(diags, &sources, &PlainRenderer);
    (ir, text)
}

const ONE_FILE: &str = r#"
service Modular;

record Video {
    id: Uuid;
    title: String;
}

handler StoreVideo(v: Video) -> Video {
    return v;
}

api Upload: Video {
    method: POST;
    path: "/videos";
}

pipeline Upload:
    StoreVideo
    -> Return;
"#;

#[test]
fn multi_file_program_generates_identically_to_one_file() {
    let dir = tmp("identical-output");
    write(
        &dir,
        "records.ciac",
        "record Video { id: Uuid; title: String; }\n",
    );
    write(
        &dir,
        "service.ciac",
        "handler StoreVideo(v: Video) -> Video { return v; }\n\n\
         api Upload: Video { method: POST; path: \"/videos\"; }\n\n\
         pipeline Upload:\n    StoreVideo\n    -> Return;\n",
    );
    let entry = write(
        &dir,
        "entry.ciac",
        "service Modular;\nimport \"records.ciac\";\nimport \"service.ciac\";\n",
    );

    let (multi_ir, multi_diags) = compile_entry(&entry);
    assert!(multi_diags.is_empty(), "unexpected: {multi_diags}");
    let multi_ir = multi_ir.expect("multi-file program compiles");

    let mut one_file_sources = SourceMap::new();
    let mut one_file_diags = Diagnostics::new();
    let one_file_id = one_file_sources.add_file("one_file.ciac", ONE_FILE);
    let one_file_program = ciac_syntax::parse(ONE_FILE, one_file_id, &mut one_file_diags);
    let one_file_ir =
        ciac_sema::analyze(&one_file_program, &mut one_file_diags).expect("one-file compiles");

    for backend in ciac_integration_tests::backends() {
        if ciac_codegen::check_support(backend.as_ref(), &multi_ir).is_err() {
            continue;
        }
        let multi_project = backend
            .generate(&multi_ir, &GenOptions::default())
            .expect("multi-file program generates");
        let one_file_project = backend
            .generate(&one_file_ir, &GenOptions::default())
            .expect("one-file program generates");
        assert_eq!(
            project_dump(&multi_project),
            project_dump(&one_file_project),
            "{}: splitting the program across files must not change generated output",
            backend.id()
        );
    }

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn duplicate_name_across_imported_files_names_both_files() {
    let dir = tmp("cross-file-duplicate");
    write(&dir, "other.ciac", "record Video { id: Uuid; }\n");
    let entry = write(
        &dir,
        "entry.ciac",
        "service Dup;\nimport \"other.ciac\";\nrecord Video { id: Uuid; }\n",
    );

    let (ir, diags_text) = compile_entry(&entry);
    assert!(ir.is_none(), "a duplicate declaration must fail sema");
    assert!(
        diags_text.contains(ErrorCode::DuplicateDeclaration.code()),
        "expected {}: {diags_text}",
        ErrorCode::DuplicateDeclaration.code()
    );
    assert!(
        diags_text.contains("other.ciac"),
        "expected the first declaration's own file named in the diagnostic: {diags_text}"
    );
    assert!(
        diags_text.contains("entry.ciac"),
        "expected the duplicate's own file named in the diagnostic: {diags_text}"
    );

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn import_cycle_is_refused() {
    let dir = tmp("cycle");
    write(&dir, "a.ciac", "import \"b.ciac\";\n");
    let entry = write(&dir, "b.ciac", "import \"a.ciac\";\n");

    let (ir, diags_text) = compile_entry(&entry);
    assert!(ir.is_none(), "a cyclic import must fail");
    assert!(
        diags_text.contains(ErrorCode::ImportCycle.code()),
        "expected {}: {diags_text}",
        ErrorCode::ImportCycle.code()
    );

    std::fs::remove_dir_all(&dir).ok();
}
