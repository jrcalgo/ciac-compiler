//! Compilation must be a pure function of its input: identical source in,
//! byte-identical projects out.
//!
//! `32UpdatePlan.md` M8 item 7: one `#[test]` fn per backend rather than
//! a single loop over every backend -- libtest parallelizes across
//! functions, not within one, so a single fn serialized every backend's
//! own examples behind each other regardless of how many cores were
//! free. Splitting by backend (not by example) isolates java's own
//! `generate()` cost -- which pays the vendored `google-java-format`
//! JVM's startup for every `.java`-emitting example -- onto its own
//! thread instead of interleaving it with the other four backends'
//! much cheaper runs.

use ciac_codegen::manifest::build_manifest;
use ciac_codegen::{Backend, GenOptions};
use ciac_integration_tests::{ciac_files, compile_file, examples_dir, project_dump};

fn generation_is_byte_deterministic_for(backend: &dyn Backend) {
    for path in ciac_files(&examples_dir()) {
        let ir = compile_file(&path);
        if ciac_codegen::check_support(backend, &ir).is_err() {
            continue;
        }
        let first = backend
            .generate(&ir, &GenOptions::default())
            .expect("generates");
        let second = backend
            .generate(&ir, &GenOptions::default())
            .expect("generates");
        assert_eq!(
            project_dump(&first),
            project_dump(&second),
            "{} / {} generated differing output across runs",
            path.display(),
            backend.id()
        );
        let first_manifest = build_manifest(&first, "0.6.0", "1.0.0", "source", backend.id());
        let second_manifest = build_manifest(&second, "0.6.0", "1.0.0", "source", backend.id());
        assert_eq!(
            serde_json::to_string_pretty(&first_manifest).expect("manifest serializes"),
            serde_json::to_string_pretty(&second_manifest).expect("manifest serializes"),
            "{} / {} generated differing manifest output across runs",
            path.display(),
            backend.id()
        );
    }
}

#[test]
fn generation_is_byte_deterministic_python() {
    generation_is_byte_deterministic_for(&ciac_backend_python::PythonBackend);
}

#[test]
fn generation_is_byte_deterministic_rust() {
    generation_is_byte_deterministic_for(&ciac_backend_rust::RustBackend);
}

#[test]
fn generation_is_byte_deterministic_typescript() {
    generation_is_byte_deterministic_for(&ciac_backend_ts::TsBackend);
}

#[test]
fn generation_is_byte_deterministic_go() {
    generation_is_byte_deterministic_for(&ciac_backend_go::GoBackend);
}

#[test]
fn generation_is_byte_deterministic_java() {
    generation_is_byte_deterministic_for(&ciac_backend_java::JavaBackend);
}
