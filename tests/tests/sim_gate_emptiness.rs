//! 27UpdatePlan.md M4/M6 exit checklist: "gate-emptiness test green
//! across the corpus" -- asserts `ciac_backend_rust::
//! unsupported_sim_capabilities` (M4) and `ciac_backend_ts::
//! unsupported_sim_capabilities` (M6) return empty for every checked-in
//! example each backend can generate at all (mirroring `golden.rs`'s
//! own `check_support`-gated skip for provider combinations a backend
//! doesn't support outright, an orthogonal question from simulation
//! coverage). A regression here means some future verb/capability
//! addition to the language outran its own world-guard. Go (M7) and
//! Java (M8) join this test in their own milestones.

use ciac_integration_tests::{backends, ciac_files, compile_file, examples_dir};

#[test]
fn rust_gate_is_empty_for_the_whole_corpus() {
    let rust = backends()
        .into_iter()
        .find(|b| b.id() == "rust")
        .expect("rust backend registered");
    for path in ciac_files(&examples_dir()) {
        let name = path.file_stem().expect("file name").to_string_lossy();
        let ir = compile_file(&path);
        if ciac_codegen::check_support(rust.as_ref(), &ir).is_err() {
            continue;
        }
        let reasons = ciac_backend_rust::unsupported_sim_capabilities(&ir);
        assert!(
            reasons.is_empty(),
            "{name}: expected an empty simulation-gate, found {reasons:?}"
        );
    }
}

#[test]
fn typescript_gate_is_empty_for_the_whole_corpus() {
    let ts = backends()
        .into_iter()
        .find(|b| b.id() == "typescript")
        .expect("typescript backend registered");
    for path in ciac_files(&examples_dir()) {
        let name = path.file_stem().expect("file name").to_string_lossy();
        let ir = compile_file(&path);
        if ciac_codegen::check_support(ts.as_ref(), &ir).is_err() {
            continue;
        }
        let reasons = ciac_backend_ts::unsupported_sim_capabilities(&ir);
        assert!(
            reasons.is_empty(),
            "{name}: expected an empty simulation-gate, found {reasons:?}"
        );
    }
}
