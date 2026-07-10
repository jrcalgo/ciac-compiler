//! Golden tests: every example program's IR and generated output is
//! snapshotted with `insta`. A diff here means the compiler's observable
//! behavior changed — review it deliberately (`cargo insta review`).

use ciac_codegen::GenOptions;
use ciac_integration_tests::{backends, ciac_files, compile_file, examples_dir, project_dump};

#[test]
fn example_ir_snapshots() {
    for path in ciac_files(&examples_dir()) {
        let name = path.file_stem().expect("file name").to_string_lossy();
        let ir = compile_file(&path);
        let json = serde_json::to_string_pretty(&ir).expect("IR serializes");
        insta::assert_snapshot!(format!("ir__{name}"), json);
    }
}

#[test]
fn example_graph_dot_snapshots() {
    for path in ciac_files(&examples_dir()) {
        let name = path.file_stem().expect("file name").to_string_lossy();
        let ir = compile_file(&path);
        insta::assert_snapshot!(format!("dot__{name}"), ir.to_dot());
    }
}

#[test]
fn example_generated_project_snapshots() {
    for path in ciac_files(&examples_dir()) {
        let name = path.file_stem().expect("file name").to_string_lossy();
        let ir = compile_file(&path);
        for backend in backends() {
            // v0.11: provider support is per-backend (db MySQL and
            // queue Kafka are python-only today) — skip a gated
            // backend/example combination instead of failing it.
            if ciac_codegen::check_support(backend.as_ref(), &ir).is_err() {
                continue;
            }
            let project = backend
                .generate(&ir, &GenOptions::default())
                .expect("examples generate");
            insta::assert_snapshot!(
                format!("gen__{}__{name}", backend.id()),
                project_dump(&project)
            );
        }
    }
}
