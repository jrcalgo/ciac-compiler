//! Validation passes over the system graph.
//!
//! Each pass checks one architectural property and reports violations as
//! diagnostics. Passes are read-only, independent, and run in a fixed
//! registered order so diagnostics are deterministic.

mod auth_placement;
mod compat;
mod cycles;
mod reachability;

use ciac_diagnostics::Diagnostics;
use ciac_ir::SystemGraph;

pub trait Pass {
    /// Stable pass name, used in logs and docs.
    fn name(&self) -> &'static str;

    fn run(&self, graph: &SystemGraph, diags: &mut Diagnostics);
}

/// The standard validation pipeline, in execution order.
pub fn default_passes() -> Vec<Box<dyn Pass>> {
    vec![
        Box::new(cycles::CycleDetection),
        Box::new(reachability::Reachability),
        Box::new(auth_placement::AuthPlacement),
        Box::new(compat::Composition),
    ]
}
