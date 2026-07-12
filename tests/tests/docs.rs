//! Documentation must track the code: every registered error code (and
//! its exact title) appears in docs/errors.md.

use ciac_diagnostics::ErrorCode;

#[test]
fn error_docs_cover_every_code() {
    let doc = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/errors.md"),
    )
    .expect("docs/errors.md exists");
    for code in ErrorCode::ALL {
        assert!(
            doc.contains(code.code()),
            "docs/errors.md is missing {}",
            code.code()
        );
        assert!(
            doc.contains(code.title()),
            "docs/errors.md is missing the title of {} ({:?})",
            code.code(),
            code.title()
        );
    }
}
