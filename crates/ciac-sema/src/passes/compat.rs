//! Composition rules for pipeline steps.

use super::Pass;
use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode, Span};
use ciac_ir::{NodeKind, Pipeline, Step, SystemGraph};

pub struct Composition;

impl Pass for Composition {
    fn name(&self) -> &'static str {
        "composition"
    }

    fn run(&self, graph: &SystemGraph, diags: &mut Diagnostics) {
        for pipeline in &graph.pipelines {
            let owner_kind = graph.node(pipeline.owner).component.kind();
            check_return(pipeline, owner_kind, diags);
            check_queue(pipeline, diags);
        }
    }
}

fn step_span(pipeline: &Pipeline, idx: usize) -> Option<Span> {
    pipeline.step_spans.get(idx).copied().flatten()
}

/// `Return` is only valid as the final step of an api pipeline.
fn check_return(pipeline: &Pipeline, owner_kind: NodeKind, diags: &mut Diagnostics) {
    for (idx, step) in pipeline.steps.iter().enumerate() {
        let Step::Return = step else { continue };
        let last = idx + 1 == pipeline.steps.len();
        if owner_kind == NodeKind::Worker {
            let mut diag = Diagnostic::new(
                ErrorCode::IncompatibleComposition,
                format!(
                    "worker pipeline `{}` cannot contain `Return`",
                    pipeline.name
                ),
            )
            .with_help("workers have no caller to respond to");
            if let Some(span) = step_span(pipeline, idx) {
                diag = diag.with_label(span, "`Return` in a worker pipeline");
            }
            diags.push(diag);
        } else if !last {
            let mut diag = Diagnostic::new(
                ErrorCode::IncompatibleComposition,
                format!(
                    "`Return` must be the final step of pipeline `{}`",
                    pipeline.name
                ),
            )
            .with_help("nothing can run after the response is sent");
            if let Some(span) = step_span(pipeline, idx) {
                diag = diag.with_label(span, "steps follow this `Return`");
            }
            diags.push(diag);
        }
    }
}

/// `Queue` may appear at most once per pipeline.
fn check_queue(pipeline: &Pipeline, diags: &mut Diagnostics) {
    let queue_steps: Vec<usize> = pipeline
        .steps
        .iter()
        .enumerate()
        .filter_map(|(idx, step)| matches!(step, Step::Queue { .. }).then_some(idx))
        .collect();
    if queue_steps.len() > 1 {
        let mut diag = Diagnostic::new(
            ErrorCode::IncompatibleComposition,
            format!(
                "pipeline `{}` publishes to the queue more than once",
                pipeline.name
            ),
        )
        .with_help("a pipeline may contain at most one `Queue` step");
        if let Some(span) = step_span(pipeline, queue_steps[1]) {
            diag = diag.with_label(span, "second `Queue` step here");
        }
        diags.push(diag);
    }
}
