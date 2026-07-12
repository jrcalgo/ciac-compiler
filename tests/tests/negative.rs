//! Negative tests: every file in `ui/` is an intentionally invalid
//! program annotated with the error codes it must produce, e.g.
//!
//! ```text
//! // expect: CIAC0005
//! ```
//!
//! The harness asserts that compilation fails and that every expected
//! code (and no unexpected error code) is reported.

use ciac_diagnostics::Severity;
use ciac_integration_tests::{ciac_files, compile, ui_dir};
use std::collections::BTreeSet;

#[test]
fn invalid_programs_report_expected_codes() {
    let files = ciac_files(&ui_dir());
    assert!(!files.is_empty(), "ui test directory must not be empty");

    for path in files {
        let src = std::fs::read_to_string(&path).expect("readable ui test");
        let expected: BTreeSet<String> = src
            .lines()
            .filter_map(|line| line.trim().strip_prefix("// expect:"))
            .map(|code| code.trim().to_owned())
            .collect();
        assert!(
            !expected.is_empty(),
            "{} must contain at least one `// expect: CIACnnnn` line",
            path.display()
        );

        let (ir, diags) = compile(&src);
        assert!(
            ir.is_none(),
            "{} unexpectedly compiled successfully",
            path.display()
        );
        let actual: BTreeSet<String> = diags
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.code.code().to_owned())
            .collect();
        assert_eq!(
            expected,
            actual,
            "{} reported the wrong error codes",
            path.display()
        );
    }
}
