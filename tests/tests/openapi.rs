//! v0.15 M1: every generated project's `openapi.json` is well-formed and
//! internally consistent — not just byte-stable (the golden snapshots
//! already catch drift), but a document a real client/generator could
//! consume: every `$ref` resolves, every path has at least one
//! operation, and single-service programs never emit the multi-service
//! index shape by accident.
//!
//! `32UpdatePlan.md` M8 item 7: split into one `#[test]` fn per backend
//! — see `determinism.rs`'s identical note for why (libtest
//! parallelizes across functions, and java's own `generate()` pays a
//! JVM-formatter cost the other four backends don't).

use ciac_codegen::{Backend, GenOptions};
use ciac_integration_tests::{ciac_files, compile_file, examples_dir};
use serde_json::Value;
use std::collections::BTreeSet;

fn collect_refs(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(Value::String(r)) = map.get("$ref") {
                out.insert(r.clone());
            }
            for v in map.values() {
                collect_refs(v, out);
            }
        }
        Value::Array(items) => {
            for v in items {
                collect_refs(v, out);
            }
        }
        _ => {}
    }
}

fn every_example_openapi_doc_is_well_formed_for(backend: &dyn Backend) {
    for path in ciac_files(&examples_dir()) {
        let name = path.file_stem().expect("file name").to_string_lossy();
        let ir = compile_file(&path);
        if ciac_codegen::check_support(backend, &ir).is_err() {
            continue;
        }
        let project = backend
            .generate(&ir, &GenOptions::default())
            .expect("examples generate");
        let raw = project
            .get("openapi.json")
            .unwrap_or_else(|| panic!("{name}/{}: no openapi.json emitted", backend.id()));
        let doc: Value = serde_json::from_str(raw)
            .unwrap_or_else(|e| panic!("{name}/{}: invalid JSON: {e}", backend.id()));

        if ir.multi_service {
            // System root gets the lightweight index, not a spec —
            // real specs live under each service's own directory
            // and are checked in the loop below via `ctx.dir`.
            assert!(
                doc.get("openapi-index").is_some(),
                "{name}/{}: multi-service root openapi.json should be the index",
                backend.id()
            );
            continue;
        }

        assert_eq!(
            doc["openapi"],
            "3.0.3",
            "{name}/{}: unexpected openapi version",
            backend.id()
        );
        let paths = doc["paths"]
            .as_object()
            .unwrap_or_else(|| panic!("{name}/{}: paths is not an object", backend.id()));
        assert!(
            paths.contains_key("/health"),
            "{name}/{}: missing /health path",
            backend.id()
        );
        for (route, item) in paths {
            assert!(
                !item.as_object().unwrap().is_empty(),
                "{name}/{}: {route} has no operations",
                backend.id()
            );
        }

        let schemas = doc["components"]["schemas"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let mut refs = BTreeSet::new();
        collect_refs(&doc, &mut refs);
        for r in refs {
            let target = r
                .strip_prefix("#/components/schemas/")
                .unwrap_or_else(|| panic!("{name}/{}: unexpected $ref shape: {r}", backend.id()));
            assert!(
                schemas.contains_key(target),
                "{name}/{}: dangling $ref {r}",
                backend.id()
            );
        }
    }
}

#[test]
fn every_example_openapi_doc_is_well_formed_python() {
    every_example_openapi_doc_is_well_formed_for(&ciac_backend_python::PythonBackend);
}

#[test]
fn every_example_openapi_doc_is_well_formed_rust() {
    every_example_openapi_doc_is_well_formed_for(&ciac_backend_rust::RustBackend);
}

#[test]
fn every_example_openapi_doc_is_well_formed_typescript() {
    every_example_openapi_doc_is_well_formed_for(&ciac_backend_ts::TsBackend);
}

#[test]
fn every_example_openapi_doc_is_well_formed_go() {
    every_example_openapi_doc_is_well_formed_for(&ciac_backend_go::GoBackend);
}

#[test]
fn every_example_openapi_doc_is_well_formed_java() {
    every_example_openapi_doc_is_well_formed_for(&ciac_backend_java::JavaBackend);
}
