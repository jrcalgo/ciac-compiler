//! Golden tests: every example program's IR and generated output is
//! snapshotted with `insta`. A diff here means the compiler's observable
//! behavior changed — review it deliberately (`cargo insta review`).
//!
//! `33UpdatePlan.md` M5: `example_generated_project_snapshots` is the
//! expensive one (it's the only fn here that calls a backend, so it's
//! the only one worth touching), but the obvious M3/M4-style rewrite
//! is unsafe here -- `insta::assert_snapshot!` carries thread-local
//! pending-snapshot state and writes `.snap.new` files on mismatch, so
//! asserting from multiple worker threads would make failure ordering
//! nondeterministic. The shape used instead: **phase one, parallel**
//! -- chunked across examples, each worker calls `check_support` +
//! `generate()` + `project_dump()` and collects `(example_idx,
//! backend_idx, snapshot_name, dump)` tuples, which is where all the
//! java cost lives; **phase two, serial, on the main thread** -- sort
//! the collected tuples back into today's exact order (example, then
//! backend) and run every `insta::assert_snapshot!` exactly as before.
//! Failure ordering is therefore identical to today's, not merely
//! equivalent. `example_ir_snapshots`/`example_graph_dot_snapshots`
//! are untouched: neither calls a backend, so neither is expensive.

use ciac_codegen::GenOptions;
use ciac_integration_tests::{
    backends, chunk_paths, ciac_files, compile_file, examples_dir, project_dump, worker_count,
};

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
    let indexed_paths: Vec<(usize, std::path::PathBuf)> = ciac_files(&examples_dir())
        .into_iter()
        .enumerate()
        .collect();
    let chunks = chunk_paths(indexed_paths, worker_count());

    // Phase 1 (parallel): generate every supported (example, backend)
    // pair and collect the snapshot name + dump, tagged with its
    // original position so phase 2 can restore today's exact order.
    let mut results: Vec<(usize, usize, String, String)> = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || {
                    let mut local = Vec::new();
                    for (example_idx, path) in &chunk {
                        let name = path.file_stem().expect("file name").to_string_lossy();
                        let ir = compile_file(path);
                        for (backend_idx, backend) in backends().into_iter().enumerate() {
                            // v0.11: provider support is per-backend (db MySQL
                            // and queue Kafka are python-only today) — skip a
                            // gated backend/example combination instead of
                            // failing it.
                            if ciac_codegen::check_support(backend.as_ref(), &ir).is_err() {
                                continue;
                            }
                            let project = backend
                                .generate(&ir, &GenOptions::default())
                                .expect("examples generate");
                            local.push((
                                *example_idx,
                                backend_idx,
                                format!("gen__{}__{name}", backend.id()),
                                project_dump(&project),
                            ));
                        }
                    }
                    local
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });

    // Phase 2 (serial, main thread): today's exact order is example
    // outer, backend inner -- both indices sort ascending to match.
    results.sort_by_key(|(example_idx, backend_idx, _, _)| (*example_idx, *backend_idx));
    for (_, _, snapshot_name, dump) in results {
        insta::assert_snapshot!(snapshot_name, dump);
    }
}
