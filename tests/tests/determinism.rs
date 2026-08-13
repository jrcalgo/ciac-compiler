//! Compilation must be a pure function of its input: identical source in,
//! byte-identical projects out.
//!
//! `32UpdatePlan.md` M8 item 7 split this file into one `#[test]` fn
//! per backend, on the reasoning that libtest parallelizes across
//! functions, not within one, so a single fn serialized every backend's
//! own examples behind each other regardless of how many cores were
//! free. That measured 4.46% against a >=30% threshold and was kept
//! with the shortfall disclosed, attributed at the time to this
//! machine's core count.
//!
//! `33UpdatePlan.md` corrects that diagnosis: java is 95.3% of this
//! file's cost (52.82s of 55.38s, measured per-backend in isolation),
//! so splitting *by backend* has a hard Amdahl ceiling of
//! `max(52.82, 2.56) / 55.38` ~= 4.6% -- almost exactly the 4.46%
//! measured. The split worked as designed; it was capped by the axis
//! it was split on before a line of it was written. Four concurrent
//! formatter invocations measured 2.55s serial vs 1.13s parallel
//! (2.26x) -- the JVM does not saturate this machine's cores, so the
//! fix is to divide the 95% instead of the 4.7%: parallelize *within*
//! each backend's own test fn, across the 29 examples, rather than
//! only across the five backend fns.
//!
//! `generation_is_byte_deterministic_for` takes a zero-arg backend
//! *factory* (`fn() -> B`) rather than `&dyn Backend`, so each worker
//! thread constructs its own backend -- free, since every backend is a
//! zero-sized unit struct -- instead of sharing one across threads,
//! which would require adding `Sync` to the public `Backend` trait
//! (`ciac-codegen/src/lib.rs`) to speed up a test harness. Worker count
//! is `available_parallelism()` capped at 4: 8-way concurrent formatter
//! invocations measured no better than 4-way on this machine, so the
//! cap costs nothing and bounds oversubscription now that libtest is
//! already running these five fns concurrently on top of it.
//!
//! The double `generate()` per example is unchanged and is not
//! cache-shortcut -- it is the test. Assertions stay inside the worker
//! threads: they are pure `String`/`assert_eq!` comparisons with no
//! shared mutable state, so a failing assertion panics its worker and
//! `std::thread::scope` propagates the panic (with the same
//! example/backend-naming message) when the scope's block ends.
//!
//! `chunk_paths`/`worker_count` live in `ciac_integration_tests` (not
//! here) -- `33UpdatePlan.md` M4 reuses them for `openapi.rs` and
//! `conformance.rs`'s identical shape.
//!
//! Follow-up, tried and reverted: `chunk_paths`' round-robin balances
//! item *count*, not item *cost*, and this corpus' `.ciac` sources
//! span a 34x byte-size range -- so `chunk_by_weight` (LPT scheduling,
//! keyed on source byte length) was tried here on the theory that a
//! worker drawing several of the largest examples was finishing long
//! after one holding only small ones. It balances source bytes across
//! workers almost perfectly (measured: worker totals within 0.7% of
//! each other, vs. round-robin's 74% spread) but moved this file's own
//! measured time by nothing (26.45s vs. 26.16s, within noise) -- source
//! byte size stopped predicting per-example `generate()` cost for java
//! once M6's AppCDS archive made that cost dominated by a near-constant
//! per-call JVM fee (~0.145s) rather than anything proportional to
//! source size. The scheduler is correct; the weight proxy no longer
//! matches what it needs to balance. Reverted rather than kept on
//! theoretical grounds alone -- `chunk_by_weight` itself stays (see
//! `ciac_integration_tests`), used by `conformance.rs`'s own memoized
//! generation pass, where it costs nothing extra since that code was
//! being written from scratch anyway.

use ciac_codegen::manifest::build_manifest;
use ciac_codegen::{Backend, GenOptions};
use ciac_integration_tests::{
    chunk_paths, ciac_files, compile_file, examples_dir, project_dump, worker_count,
};

fn generation_is_byte_deterministic_for<B: Backend>(new_backend: fn() -> B) {
    let chunks = chunk_paths(ciac_files(&examples_dir()), worker_count());

    std::thread::scope(|scope| {
        for chunk in chunks {
            scope.spawn(move || {
                let backend = new_backend();
                for path in &chunk {
                    let ir = compile_file(path);
                    if ciac_codegen::check_support(&backend, &ir).is_err() {
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
                    let first_manifest =
                        build_manifest(&first, "0.6.0", "1.0.0", "source", backend.id());
                    let second_manifest =
                        build_manifest(&second, "0.6.0", "1.0.0", "source", backend.id());
                    assert_eq!(
                        serde_json::to_string_pretty(&first_manifest).expect("manifest serializes"),
                        serde_json::to_string_pretty(&second_manifest)
                            .expect("manifest serializes"),
                        "{} / {} generated differing manifest output across runs",
                        path.display(),
                        backend.id()
                    );
                }
            });
        }
    });
}

#[test]
fn generation_is_byte_deterministic_python() {
    generation_is_byte_deterministic_for(|| ciac_backend_python::PythonBackend);
}

#[test]
fn generation_is_byte_deterministic_rust() {
    generation_is_byte_deterministic_for(|| ciac_backend_rust::RustBackend);
}

#[test]
fn generation_is_byte_deterministic_typescript() {
    generation_is_byte_deterministic_for(|| ciac_backend_ts::TsBackend);
}

#[test]
fn generation_is_byte_deterministic_go() {
    generation_is_byte_deterministic_for(|| ciac_backend_go::GoBackend);
}

#[test]
fn generation_is_byte_deterministic_java() {
    generation_is_byte_deterministic_for(|| ciac_backend_java::JavaBackend);
}
