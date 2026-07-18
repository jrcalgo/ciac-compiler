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
