//! v0.15 M1: every generated project's `openapi.json` is well-formed and
//! internally consistent — not just byte-stable (the golden snapshots
//! already catch drift), but a document a real client/generator could
//! consume: every `$ref` resolves, every path has at least one
//! operation, and single-service programs never emit the multi-service
//! index shape by accident.
//!
//! `33UpdatePlan.md` M4: same shape as `determinism.rs`'s own M3
//! rewrite -- see that file's module doc for the full derivation of
//! why splitting by example (not only by backend) is what pays. Each
//! `#[test]` fn's own example corpus is chunked across worker threads
//! via a backend factory, so no `Sync` bound is added to the public
//! `Backend` trait to speed up this harness.

use ciac_codegen::{Backend, GenOptions};
use ciac_integration_tests::{chunk_paths, ciac_files, compile_file, examples_dir, worker_count};
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

fn every_example_openapi_doc_is_well_formed_for<B: Backend>(new_backend: fn() -> B) {
    let chunks = chunk_paths(ciac_files(&examples_dir()), worker_count());

    std::thread::scope(|scope| {
        for chunk in chunks {
            scope.spawn(move || {
                let backend = new_backend();
                for path in &chunk {
                    let name = path.file_stem().expect("file name").to_string_lossy();
                    let ir = compile_file(path);
                    if ciac_codegen::check_support(&backend, &ir).is_err() {
                        continue;
                    }
                    let project = backend
                        .generate(&ir, &GenOptions::default())
                        .expect("examples generate");
                    let raw = project.get("openapi.json").unwrap_or_else(|| {
                        panic!("{name}/{}: no openapi.json emitted", backend.id())
                    });
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
                    let paths = doc["paths"].as_object().unwrap_or_else(|| {
                        panic!("{name}/{}: paths is not an object", backend.id())
                    });
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
                        let target = r.strip_prefix("#/components/schemas/").unwrap_or_else(|| {
                            panic!("{name}/{}: unexpected $ref shape: {r}", backend.id())
                        });
                        assert!(
                            schemas.contains_key(target),
                            "{name}/{}: dangling $ref {r}",
                            backend.id()
                        );
                    }
                }
            });
        }
    });
}

#[test]
fn every_example_openapi_doc_is_well_formed_python() {
    every_example_openapi_doc_is_well_formed_for(|| ciac_backend_python::PythonBackend);
}

#[test]
fn every_example_openapi_doc_is_well_formed_rust() {
    every_example_openapi_doc_is_well_formed_for(|| ciac_backend_rust::RustBackend);
}

#[test]
fn every_example_openapi_doc_is_well_formed_typescript() {
    every_example_openapi_doc_is_well_formed_for(|| ciac_backend_ts::TsBackend);
}

#[test]
fn every_example_openapi_doc_is_well_formed_go() {
    every_example_openapi_doc_is_well_formed_for(|| ciac_backend_go::GoBackend);
}

#[test]
fn every_example_openapi_doc_is_well_formed_java() {
    every_example_openapi_doc_is_well_formed_for(|| ciac_backend_java::JavaBackend);
}
