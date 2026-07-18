//! Typed system-graph intermediate representation for the CIaC compiler.
//!
//! A CIaC program compiles to a [`SystemGraph`]: architectural components
//! (APIs, services, workers, databases, caches, queues, auth, observability)
//! as nodes, and request/data/message/dependency flow as edges. The graph is
//! target-independent — every code-generation backend consumes the same
//! [`NormalizedIr`] — and fully serializable so it can be inspected
//! (`ciac graph`) or handed to out-of-process tooling.

mod component;
mod graph;
mod hir;
mod record;

pub use component::{
    ApiConfig, AuthScheme, CacheEngine, ChannelConfig, Component, CrudConfig, DbEngine,
    EmailProvider, HttpMethod, JobConfig, LoggingProvider, MetricsProvider, NodeKind,
    ObjectStoreProvider, QueueEngine, RealtimeProvider, SchedulerProvider, SearchProvider,
    TracingProvider, UsersProvider, WorkerConfig,
};
pub use graph::{
    Edge, EdgeId, EdgeKind, EventStream, MatchArm, Node, NodeId, Pipeline, Resource, Service,
    ServiceId, Step, StepKind, SystemGraph,
};
pub use hir::{
    BinOp, Builtin, HandlerBody, HirArm, HirExpr, HirPredTerm, HirPredicate, HirStmt, HirType,
    PredOp, Table, TableId, UnOp, Verb,
};
pub use record::{Cardinality, FieldType, Record, RecordField, RecordId, RecordKind, RefAction};

use serde::Serialize;

/// A [`SystemGraph`] that has passed every semantic-analysis pass.
///
/// This is the contract between the front-end and code-generation backends:
/// backends may rely on all invariants checked by `ciac-sema` (acyclic
/// flows, resolved steps, satisfied capabilities, valid auth placement).
/// Only `ciac-sema` is intended to construct values of this type.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct NormalizedIr(SystemGraph);

impl NormalizedIr {
    /// Wraps a graph **without validating it**.
    ///
    /// Callers must guarantee the graph has passed semantic analysis;
    /// `ciac_sema::analyze` is the only intended caller.
    pub fn from_validated(graph: SystemGraph) -> Self {
        Self(graph)
    }

    pub fn graph(&self) -> &SystemGraph {
        &self.0
    }
}

impl std::ops::Deref for NormalizedIr {
    type Target = SystemGraph;

    fn deref(&self) -> &SystemGraph {
        &self.0
    }
}
