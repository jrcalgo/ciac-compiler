//! Semantic analysis for CIaC: lowers the AST into the typed system graph,
//! expands higher-level constructs (`crud`, `events`), and runs the
//! validation passes that give CIaC its compile-time architectural
//! guarantees.
//!
//! The entry point is [`analyze`]:
//!
//! ```text
//! AST ──build (resolve names, satisfy capabilities, expand crud/events)──▶ SystemGraph
//!     ──passes (cycles, reachability, auth placement, composition)──────▶ NormalizedIr
//! ```
//!
//! Every problem is reported through [`ciac_diagnostics::Diagnostics`];
//! analysis never panics on user input.

mod build;
pub mod passes;

use ciac_diagnostics::Diagnostics;
use ciac_ir::NormalizedIr;
use ciac_syntax::ast::Program;

/// Runs the full semantic-analysis pipeline.
///
/// Returns `Some(NormalizedIr)` only when the program is architecturally
/// valid (warnings allowed, errors not). On error the graph is withheld so
/// no downstream consumer can generate code from an invalid architecture.
pub fn analyze(program: &Program, diags: &mut Diagnostics) -> Option<NormalizedIr> {
    let graph = build::build_graph(program, diags);
    for pass in passes::default_passes() {
        pass.run(&graph, diags);
    }
    if diags.has_errors() {
        None
    } else {
        Some(NormalizedIr::from_validated(graph))
    }
}
