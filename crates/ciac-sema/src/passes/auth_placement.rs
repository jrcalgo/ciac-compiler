//! Auth placement rules: authentication must gate the request boundary.

use super::Pass;
use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode};
use ciac_ir::{NodeKind, Step, StepKind, SystemGraph};

pub struct AuthPlacement;

impl Pass for AuthPlacement {
    fn name(&self) -> &'static str {
        "auth-placement"
    }

    fn run(&self, graph: &SystemGraph, diags: &mut Diagnostics) {
        for pipeline in &graph.pipelines {
            let owner_kind = graph.node(pipeline.owner).component.kind();
            check_steps(&pipeline.steps, owner_kind, &pipeline.name, true, diags);
        }
    }
}

fn check_steps(
    steps: &[Step],
    owner_kind: NodeKind,
    pipeline_name: &str,
    top_level: bool,
    diags: &mut Diagnostics,
) {
    for (idx, step) in steps.iter().enumerate() {
        match &step.kind {
            StepKind::Auth { .. } => {
                if owner_kind == NodeKind::Worker {
                    let mut diag = Diagnostic::new(
                        ErrorCode::InvalidAuthPlacement,
                        format!("worker pipeline `{pipeline_name}` cannot contain an `Auth` step"),
                    )
                    .with_help(
                        "workers process queue messages; there is no request to authenticate",
                    );
                    if let Some(span) = step.span {
                        diag = diag.with_label(span, "`Auth` used in a worker pipeline");
                    }
                    diags.push(diag);
                } else if !top_level || idx != 0 {
                    let mut diag = Diagnostic::new(
                        ErrorCode::InvalidAuthPlacement,
                        format!("`Auth` must be the first step of pipeline `{pipeline_name}`"),
                    )
                    .with_help("no work may happen before the request is authenticated");
                    if let Some(span) = step.span {
                        diag = diag.with_label(span, "`Auth` is not the first pipeline step");
                    }
                    diags.push(diag);
                }
            }
            StepKind::Match { arms, .. } => {
                for arm in arms {
                    check_steps(&arm.steps, owner_kind, pipeline_name, false, diags);
                }
            }
            StepKind::Publish { .. } | StepKind::Return | StepKind::Handler { .. } => {}
        }
    }
}
