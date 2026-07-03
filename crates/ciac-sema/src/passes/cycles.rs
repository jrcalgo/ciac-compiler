//! Cycle detection over request, message, and dependency flow.
//!
//! CIaC programs must describe acyclic architectures: a cycle in the flow
//! graph (e.g. a worker publishing back onto the queue it consumes from)
//! would generate a system that feeds itself work forever.

use super::Pass;
use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode};
use ciac_ir::{EdgeKind, NodeId, SystemGraph};

pub struct CycleDetection;

impl Pass for CycleDetection {
    fn name(&self) -> &'static str {
        "cycle-detection"
    }

    fn run(&self, graph: &SystemGraph, diags: &mut Diagnostics) {
        // Iterative DFS with three-color marking over flow edges
        // (DataFlow is excluded: a service both reading and writing a
        // database is not an architectural cycle).
        let node_count = graph.nodes().count();
        let mut state = vec![Color::White; node_count];
        for start in graph.nodes().map(|n| n.id) {
            if state[start.0 as usize] == Color::White {
                if let Some(cycle) = dfs(graph, start, &mut state) {
                    report(graph, &cycle, diags);
                    // One cycle report is enough to act on; avoid a
                    // cascade of overlapping reports.
                    return;
                }
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Color {
    White,
    Gray,
    Black,
}

fn flows(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::RequestFlow
            | EdgeKind::AsyncMessage
            | EdgeKind::ServiceCall
            | EdgeKind::DependsOn
    )
}

/// Returns the nodes forming a cycle, if one is reachable from `start`.
fn dfs(graph: &SystemGraph, start: NodeId, state: &mut [Color]) -> Option<Vec<NodeId>> {
    let mut path: Vec<NodeId> = vec![start];
    // Parallel stack of edge iterators, kept as indices for simplicity.
    let mut edge_stacks: Vec<Vec<NodeId>> = vec![successors(graph, start)];
    state[start.0 as usize] = Color::Gray;

    while let Some(succs) = edge_stacks.last_mut() {
        match succs.pop() {
            Some(next) => match state[next.0 as usize] {
                Color::Gray => {
                    // Found a back edge: the cycle is the path suffix
                    // starting at `next`.
                    let pos = path.iter().position(|&n| n == next).unwrap_or(0);
                    return Some(path[pos..].to_vec());
                }
                Color::White => {
                    state[next.0 as usize] = Color::Gray;
                    path.push(next);
                    edge_stacks.push(successors(graph, next));
                }
                Color::Black => {}
            },
            None => {
                let done = path.pop().expect("path parallels edge_stacks");
                state[done.0 as usize] = Color::Black;
                edge_stacks.pop();
            }
        }
    }
    None
}

fn successors(graph: &SystemGraph, node: NodeId) -> Vec<NodeId> {
    let mut succs: Vec<NodeId> = graph
        .edges_from(node)
        .filter(|e| flows(e.kind))
        .map(|e| e.to)
        .collect();
    // Reverse so popping visits successors in insertion order.
    succs.reverse();
    succs
}

fn report(graph: &SystemGraph, cycle: &[NodeId], diags: &mut Diagnostics) {
    let mut names: Vec<String> = cycle
        .iter()
        .map(|&id| graph.node(id).component.label())
        .collect();
    if let Some(first) = names.first().cloned() {
        names.push(first);
    }
    let mut diag = Diagnostic::new(
        ErrorCode::CyclicDependency,
        format!("cyclic flow: {}", names.join(" -> ")),
    )
    .with_help("break the cycle, e.g. publish to a different stream than the one consumed");
    if let Some(span) = cycle.iter().find_map(|&id| graph.node(id).span) {
        diag = diag.with_label(span, "part of the cycle");
    }
    diags.push(diag);
}
