//! The `HostSyntax` contract's own golden (`22UpdatePlan.md` Pillar 3,
//! Parts 2-3 — "a test-only 'identity' `HostSyntax`... demonstrates...
//! that identity backend's output is itself snapshot-tested so the
//! *contract* has goldens, not just its consumers"). For every typed
//! inline handler body across the 26-example corpus, renders both
//! [`ciac_codegen::lower::IdentitySyntax`] (`Orientation::Expression`)
//! and [`ciac_codegen::lower::IdentitySyntaxStatement`]
//! (`Orientation::Statement`) against the *same* HIR, snapshotting
//! both — proving the shared dispatcher renders every HIR shape the
//! corpus exercises without a panic, from either orientation, not
//! just from the two real backends' own leaf choices.

use ciac_codegen::lower::{
    lower_body_expr, lower_body_stmt, IdentitySyntax, IdentitySyntaxStatement,
};
use ciac_integration_tests::{ciac_files, compile_file, examples_dir};
use ciac_ir::Component;

/// Every typed inline handler (`Component::Service` with a `Some`
/// signature carrying a real body — `extern handler`s have a checked
/// signature but no body to lower) in `ir`, named for stable
/// snapshot keys.
fn typed_inline_handlers(ir: &ciac_ir::NormalizedIr) -> Vec<(String, &ciac_ir::HandlerBody)> {
    let mut handlers: Vec<(String, &ciac_ir::HandlerBody)> = ir
        .nodes()
        .filter_map(|node| match &node.component {
            Component::Service {
                name,
                signature: Some(hir),
            } if hir.body.is_some() => Some((name.clone(), hir)),
            _ => None,
        })
        .collect();
    handlers.sort_by(|a, b| a.0.cmp(&b.0));
    handlers
}

#[test]
fn identity_expression_orientation_renders_every_typed_handler_in_the_corpus() {
    for path in ciac_files(&examples_dir()) {
        let name = path.file_stem().expect("file name").to_string_lossy();
        let ir = compile_file(&path);
        let syntax = IdentitySyntax::new(&ir);
        for (handler_name, hir) in typed_inline_handlers(&ir) {
            let rendered = lower_body_expr(&syntax, &ir, hir);
            insta::assert_snapshot!(format!("identity_expr__{name}__{handler_name}"), rendered);
        }
    }
}

#[test]
fn identity_statement_orientation_renders_every_typed_handler_in_the_corpus() {
    for path in ciac_files(&examples_dir()) {
        let name = path.file_stem().expect("file name").to_string_lossy();
        let ir = compile_file(&path);
        let syntax = IdentitySyntaxStatement::new(&ir);
        for (handler_name, hir) in typed_inline_handlers(&ir) {
            let rendered = lower_body_stmt(&syntax, &ir, hir, "").join("\n");
            insta::assert_snapshot!(format!("identity_stmt__{name}__{handler_name}"), rendered);
        }
    }
}
