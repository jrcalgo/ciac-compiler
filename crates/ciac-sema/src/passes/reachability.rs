//! Reachability / liveness warnings: declared components nothing uses.

use super::Pass;
use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode};
use ciac_ir::{Node, NodeKind, SystemGraph};

pub struct Reachability;

impl Pass for Reachability {
    fn name(&self) -> &'static str {
        "reachability"
    }

    fn run(&self, graph: &SystemGraph, diags: &mut Diagnostics) {
        for node in graph.nodes() {
            match node.component.kind() {
                NodeKind::Api => {
                    let has_pipeline = graph.pipeline_of(node.id).is_some();
                    let is_resource = graph.resources.iter().any(|r| r.api == node.id);
                    if !has_pipeline && !is_resource {
                        warn(
                            diags,
                            node,
                            "api has no pipeline",
                            "attach one with `pipeline <Name>: ..;`",
                        );
                    }
                }
                NodeKind::Worker => {
                    let consumes = graph.edges_to(node.id).next().is_some();
                    if !consumes {
                        warn(
                            diags,
                            node,
                            "worker never receives messages",
                            "give it a pipeline (`pipeline <Name>: ..;`) so it consumes from the queue",
                        );
                    }
                }
                NodeKind::Queue => {
                    let used = graph.edges_to(node.id).next().is_some()
                        || graph.edges_from(node.id).next().is_some();
                    if !used {
                        warn(
                            diags,
                            node,
                            "queue is declared but never used",
                            "publish to it with a `Queue` step or consume with a worker",
                        );
                    }
                }
                NodeKind::Database | NodeKind::Cache | NodeKind::Auth => {
                    let used = graph.edges_to(node.id).next().is_some()
                        || graph.edges_from(node.id).next().is_some();
                    if !used {
                        warn(
                            diags,
                            node,
                            "capability is declared but never used",
                            "remove it from the `use { .. }` block or reference it in a pipeline",
                        );
                    }
                }
                // Services are only created by being referenced; logging and
                // metrics apply system-wide.
                NodeKind::Service | NodeKind::Logging | NodeKind::Metrics => {}
            }
        }
    }
}

fn warn(diags: &mut Diagnostics, node: &Node, message: &str, help: &str) {
    let mut diag = Diagnostic::new(
        ErrorCode::UnreachableComponent,
        format!("{}: {message}", node.component.label()),
    )
    .with_help(help);
    if let Some(span) = node.span {
        diag = diag.with_label(span, "declared here");
    }
    diags.push(diag);
}
