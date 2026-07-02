//! Auth placement rules: authentication must gate the request boundary.

use super::Pass;
use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode};
use ciac_ir::{NodeKind, Step, SystemGraph};

pub struct AuthPlacement;

impl Pass for AuthPlacement {
    fn name(&self) -> &'static str {
        "auth-placement"
    }

    fn run(&self, graph: &SystemGraph, diags: &mut Diagnostics) {
        for pipeline in &graph.pipelines {
            let owner_kind = graph.node(pipeline.owner).component.kind();
            for (idx, step) in pipeline.steps.iter().enumerate() {
                let Step::Auth { .. } = step else { continue };
                let span = pipeline.step_spans.get(idx).copied().flatten();
                if owner_kind == NodeKind::Worker {
                    let mut diag = Diagnostic::new(
                        ErrorCode::InvalidAuthPlacement,
                        format!(
                            "worker pipeline `{}` cannot contain an `Auth` step",
                            pipeline.name
                        ),
                    )
                    .with_help(
                        "workers process queue messages; there is no request to authenticate",
                    );
                    if let Some(span) = span {
                        diag = diag.with_label(span, "`Auth` used in a worker pipeline");
                    }
                    diags.push(diag);
                } else if idx != 0 {
                    let mut diag = Diagnostic::new(
                        ErrorCode::InvalidAuthPlacement,
                        format!(
                            "`Auth` must be the first step of pipeline `{}`",
                            pipeline.name
                        ),
                    )
                    .with_help("no work may happen before the request is authenticated");
                    if let Some(span) = span {
                        diag = diag.with_label(span, format!("`Auth` appears as step {}", idx + 1));
                    }
                    diags.push(diag);
                }
            }
        }
    }
}
