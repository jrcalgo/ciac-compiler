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
/// is the normative contract). Read from the repo-root `LANGUAGE_VERSION`
/// file so every consumer (`--version`, `describe`, `targets --json`,
/// the generated manifest stamp) shares one source, never a duplicated
/// literal.
pub const LANGUAGE_VERSION: &str = include_str!("../../../LANGUAGE_VERSION");
