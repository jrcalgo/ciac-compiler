//! v0.15 M7: applying a diagnostic's fix must clear that diagnostic --
//! the correctness bar the plan sets for "structured fixes that apply
//! themselves." Runs over the whole negative-fixture corpus: for
//! every diagnostic that carries at least one offered fix, apply it
//! and re-check that the diagnostic's own code is no longer reported.

use ciac_integration_tests::{ciac_files, compile, ui_dir};

#[test]
fn every_offered_fix_clears_its_own_diagnostic() {
    let mut exercised = 0;
    for path in ciac_files(&ui_dir()) {
        let src = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
        let (_, diags) = compile(&src);
        for diag in diags.iter() {
            for fix in &diag.fixes {
                exercised += 1;
                let patched = fix.apply(&src);
                let (_, after) = compile(&patched);
                assert!(
                    !after.codes().contains(&diag.code),
                    "{}: fix {:?} for {:?} did not clear the diagnostic\n--- patched ---\n{patched}",
                    path.display(),
                    fix.title,
                    diag.code,
                );
            }
        }
    }
    assert!(
        exercised > 0,
        "expected at least one ui/ fixture to exercise a v0.15 M7 fix"
    );
}
