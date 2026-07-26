//! 27UpdatePlan.md M4 exit checklist: "Rust gate-emptiness test green
//! across the corpus" -- asserts `ciac_backend_rust::
//! unsupported_sim_capabilities` returns empty for every checked-in
//! example the Rust backend can generate at all (mirroring `golden.rs`'s
//! own `check_support`-gated skip for provider combinations Rust
//! doesn't support outright, an orthogonal question from simulation
//! coverage). A regression here means some future verb/capability
//! addition to the language outran its own world-guard.

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
