//! Compilation must be a pure function of its input: identical source in,
//! byte-identical projects out.

use ciac_codegen::manifest::build_manifest;
use ciac_codegen::GenOptions;
use ciac_integration_tests::{backends, ciac_files, compile_file, examples_dir, project_dump};

#[test]
fn generation_is_byte_deterministic() {
    for path in ciac_files(&examples_dir()) {
        for backend in backends() {
            let first = backend
                .generate(&compile_file(&path), &GenOptions::default())
                .expect("generates");
            let second = backend
                .generate(&compile_file(&path), &GenOptions::default())
                .expect("generates");
            assert_eq!(
                project_dump(&first),
                project_dump(&second),
                "{} / {} generated differing output across runs",
                path.display(),
                backend.id()
            );
            let first_manifest = build_manifest(&first, "0.6.0", "source", backend.id());
            let second_manifest = build_manifest(&second, "0.6.0", "source", backend.id());
            assert_eq!(
                serde_json::to_string_pretty(&first_manifest).expect("manifest serializes"),
                serde_json::to_string_pretty(&second_manifest).expect("manifest serializes"),
                "{} / {} generated differing manifest output across runs",
                path.display(),
                backend.id()
            );
        }
    }
}
