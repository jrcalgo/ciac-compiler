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
/// is the normative contract). Read from this crate's `LANGUAGE_VERSION`
/// file (packaged for crates.io) so every consumer (`--version`,
/// `describe`, `targets --json`, the generated manifest stamp) shares
/// one source, never a duplicated literal. The repo-root
/// `LANGUAGE_VERSION` is a CI/reviewer mirror and must stay identical.
pub const LANGUAGE_VERSION: &str = include_str!("../LANGUAGE_VERSION");

#[cfg(test)]
mod language_version_tests {
    use super::LANGUAGE_VERSION;
    use std::path::Path;

    #[test]
    fn root_mirror_matches_crate_file() {
        // Workspace builds have the repo-root mirror; the crates.io
        // package tarball does not. Gate so `cargo package` verify still
        // compiles and runs tests.
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../LANGUAGE_VERSION");
        if !root.exists() {
            return;
        }
        let mirror = std::fs::read_to_string(&root).expect("read root LANGUAGE_VERSION");
        assert_eq!(
            mirror, LANGUAGE_VERSION,
            "repo-root LANGUAGE_VERSION must match crates/ciac-syntax/LANGUAGE_VERSION"
        );
    }
}
