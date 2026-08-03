//! Lexer, parser, and AST for the CIaC language.
//!
//! The grammar is documented in `docs/language.md`. Parsing never panics
//! and never aborts: lexical and syntactic problems are reported through
//! [`ciac_diagnostics::Diagnostics`] and the parser recovers at declaration
//! boundaries so a single mistake does not hide the rest of the program.

pub mod ast;
mod lexer;
pub mod module;
mod parser;
mod registry;
pub mod rename_index;

pub use lexer::{lex, Token, TokenKind};
pub use module::load;
pub use parser::parse;

/// The CIaC *language* version (`26UpdatePlan.md` M8) — distinct from
/// `ciac`'s own compiler version (`CARGO_PKG_VERSION`). The language
/// surface froze at v1.0.0; the compiler continues moving on its own
/// number (`docs/language.md`'s `## Stability and versioning` section
/// is the normative contract). Read from `vendor/LANGUAGE_VERSION`, a
/// physical copy checked into this crate's own directory, not the
/// repo-root file directly -- found live via a real `cargo publish`
/// failure: `cargo package`/`publish` never bundles a
/// `../../../LANGUAGE_VERSION` path escaping the crate directory. Run
/// `scripts/sync-vendored-ciac-assets.sh` after `LANGUAGE_VERSION`
/// changes; see this file's own
/// `vendored_language_version_matches_source` test.
pub const LANGUAGE_VERSION: &str = include_str!("../vendor/LANGUAGE_VERSION");

#[cfg(test)]
mod tests {
    /// Guards `vendor/LANGUAGE_VERSION`'s own reason for existing:
    /// runs only inside the workspace, where the repo-root file is
    /// reachable relative to `CARGO_MANIFEST_DIR` -- never true when
    /// building from a published crate's own package tarball, which
    /// contains only the vendored copy this test would have nothing to
    /// compare it against.
    #[test]
    fn vendored_language_version_matches_source() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let source_path = std::path::Path::new(manifest_dir).join("../../LANGUAGE_VERSION");
        if !source_path.is_file() {
            return;
        }
        let source =
            std::fs::read_to_string(&source_path).expect("reading repo-root LANGUAGE_VERSION");
        let vendored = std::fs::read_to_string(
            std::path::Path::new(manifest_dir).join("vendor/LANGUAGE_VERSION"),
        )
        .expect("reading vendor/LANGUAGE_VERSION");
        assert_eq!(
            source, vendored,
            "vendor/LANGUAGE_VERSION has drifted from the repo-root LANGUAGE_VERSION -- \
             run scripts/sync-vendored-ciac-assets.sh"
        );
    }
}
