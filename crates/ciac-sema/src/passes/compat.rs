//! Composition rules for pipeline steps.

use super::Pass;
use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode};
use ciac_ir::{NodeKind, Pipeline, Step, StepKind, SystemGraph};

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
            check_match(pipeline, diags);
        }
    }
}

/// `Return` is only valid as the final step of an api pipeline.
fn check_return(pipeline: &Pipeline, owner_kind: NodeKind, diags: &mut Diagnostics) {
    for (idx, step) in pipeline.steps.iter().enumerate() {
        if !matches!(&step.kind, StepKind::Return) {
            continue;
        }
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
            if let Some(span) = step.span {
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
            if let Some(span) = step.span {
                diag = diag.with_label(span, "steps follow this `Return`");
            }
            diags.push(diag);
        }
    }
}

/// A pipeline may publish to any number of *different* streams, but
/// publishing to the same stream twice is a mistake.
fn check_queue(pipeline: &Pipeline, diags: &mut Diagnostics) {
    let mut seen: Vec<ciac_ir::NodeId> = Vec::new();
    check_queue_steps(&pipeline.steps, &pipeline.name, &mut seen, diags);
}

fn check_queue_steps(
    steps: &[Step],
    pipeline_name: &str,
    seen: &mut Vec<ciac_ir::NodeId>,
    diags: &mut Diagnostics,
) {
    for step in steps {
        let StepKind::Publish { stream } = &step.kind else {
            if let StepKind::Match { arms, .. } = &step.kind {
                for arm in arms {
                    let mut arm_seen = seen.clone();
                    check_queue_steps(&arm.steps, pipeline_name, &mut arm_seen, diags);
                }
            }
            continue;
        };
        if seen.contains(stream) {
            let mut diag = Diagnostic::new(
                ErrorCode::IncompatibleComposition,
                format!(
                    "pipeline `{}` publishes to the same stream more than once",
                    pipeline_name
                ),
            )
            .with_help("each stream may be published to at most once per pipeline");
            if let Some(span) = step.span {
                diag = diag.with_label(span, "second publish to this stream here");
            }
            diags.push(diag);
        } else {
            seen.push(*stream);
        }
    }
}

fn check_match(pipeline: &Pipeline, diags: &mut Diagnostics) {
    for (idx, step) in pipeline.steps.iter().enumerate() {
        let StepKind::Match { arms, .. } = &step.kind else {
            continue;
        };
        if idx + 1 != pipeline.steps.len() {
            let mut diag = Diagnostic::new(
                ErrorCode::InvalidMatch,
                format!(
                    "`match` must be the final step of pipeline `{}`",
                    pipeline.name
                ),
            )
            .with_help("move all post-match work into match arms");
            if let Some(span) = step.span {
                diag = diag.with_label(span, "`match` is followed by another step");
            }
            diags.push(diag);
        }
        for arm in arms {
            if let Some(nested) = first_match(&arm.steps) {
                let mut diag = Diagnostic::new(
                    ErrorCode::InvalidMatch,
                    format!("pipeline `{}` contains a nested `match`", pipeline.name),
                )
                .with_help("v0.3 match arms may contain handlers, publishes, and Return only");
                if let Some(span) = nested.span {
                    diag = diag.with_label(span, "nested match here");
                }
                diags.push(diag);
            }
        }
    }
}

fn first_match(steps: &[Step]) -> Option<&Step> {
    for step in steps {
        if matches!(&step.kind, StepKind::Match { .. }) {
            return Some(step);
        }
    }
    None
}
