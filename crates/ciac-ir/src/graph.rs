use crate::component::{Component, NodeKind};
use crate::record::{Record, RecordId};
use ciac_diagnostics::Span;
use serde::Serialize;

/// Index of a node in a [`SystemGraph`]. Stable for the life of the graph;
/// nodes are never removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct NodeId(pub u32);

/// Index of an edge in a [`SystemGraph`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct EdgeId(pub u32);

/// What an edge means architecturally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum EdgeKind {
    /// Synchronous request flow (API call, in-process invocation).
    RequestFlow,
    /// Reads/writes of stored data (service ↔ database/cache).
    DataFlow,
    /// Asynchronous messaging through a queue.
    AsyncMessage,
    /// Provisioning/startup dependency without direct data movement.
    DependsOn,
}

#[derive(Debug, Clone, Serialize)]
pub struct Node {
    pub id: NodeId,
    #[serde(flatten)]
    pub component: Component,
    /// Where the component was declared, when it maps to source directly.
    #[serde(skip)]
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Edge {
    pub id: EdgeId,
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
}

/// A resolved pipeline step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "step")]
pub enum Step {
    /// Builtin `Auth`: authenticate the request against the auth node.
    Auth { node: NodeId },
    /// Publish the current payload to a stream node. The surface `Queue`
    /// step lowers to a publish on the service's default stream.
    Publish { stream: NodeId },
    /// Builtin `Return`: respond to the caller. Always terminal.
    Return,
    /// Invoke a service handler node.
    Handler { node: NodeId },
}

/// An ordered execution chain owned by an api (request flow) or a worker
/// (asynchronous processing chain).
#[derive(Debug, Clone, Serialize)]
pub struct Pipeline {
    pub name: String,
    /// The api or worker node this pipeline belongs to.
    pub owner: NodeId,
    /// The payload record every step handles: the api's request type or
    /// the consumed stream's record. `None` = untyped JSON.
    pub payload: Option<RecordId>,
    pub steps: Vec<Step>,
    #[serde(skip)]
    pub span: Option<Span>,
    /// Source spans per step, parallel to `steps` (for diagnostics).
    #[serde(skip)]
    pub step_spans: Vec<Option<Span>>,
}

/// A CRUD resource produced by expanding `crud <Name>;`.
#[derive(Debug, Clone, Serialize)]
pub struct Resource {
    pub name: String,
    pub api: NodeId,
    pub service: NodeId,
    /// Record supplying real columns; `None` keeps the generic
    /// keyed-document model.
    pub record: Option<RecordId>,
}

/// An event stream produced by expanding `events <Name>;`.
#[derive(Debug, Clone, Serialize)]
pub struct EventStream {
    pub name: String,
    /// The stream node the expansion created.
    pub stream: NodeId,
    pub worker: NodeId,
}

/// The typed directed graph a CIaC program compiles to.
///
/// Node and edge iteration follows insertion order, which the builder in
/// `ciac-sema` derives from declaration order — making every downstream
/// consumer (passes, dumps, codegen) deterministic by construction.
#[derive(Debug, Default, Serialize)]
pub struct SystemGraph {
    /// The system name from the `service <Name>;` declaration.
    pub name: String,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    records: Vec<Record>,
    pub pipelines: Vec<Pipeline>,
    pub resources: Vec<Resource>,
    pub event_streams: Vec<EventStream>,
}

impl SystemGraph {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }

    pub fn add_node(&mut self, component: Component, span: Option<Span>) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        self.nodes.push(Node {
            id,
            component,
            span,
        });
        id
    }

    /// Adds an edge, reusing an existing identical edge if present so
    /// repeated wiring (e.g. two pipelines using the same handler) does not
    /// produce duplicates.
    pub fn add_edge(&mut self, from: NodeId, to: NodeId, kind: EdgeKind) -> EdgeId {
        if let Some(existing) = self
            .edges
            .iter()
            .find(|e| e.from == from && e.to == to && e.kind == kind)
        {
            return existing.id;
        }
        let id = EdgeId(self.edges.len() as u32);
        self.edges.push(Edge { id, from, to, kind });
        id
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id.0 as usize]
    }

    pub fn add_record(&mut self, record: Record) -> RecordId {
        let id = RecordId(self.records.len() as u32);
        self.records.push(record);
        id
    }

    pub fn record(&self, id: RecordId) -> &Record {
        &self.records[id.0 as usize]
    }

    pub fn records(&self) -> impl Iterator<Item = (RecordId, &Record)> {
        self.records
            .iter()
            .enumerate()
            .map(|(i, r)| (RecordId(i as u32), r))
    }

    pub fn find_record(&self, name: &str) -> Option<RecordId> {
        self.records
            .iter()
            .position(|r| r.name == name)
            .map(|i| RecordId(i as u32))
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.iter()
    }

    pub fn edges(&self) -> impl Iterator<Item = &Edge> {
        self.edges.iter()
    }

    pub fn nodes_of_kind(&self, kind: NodeKind) -> impl Iterator<Item = &Node> {
        self.nodes
            .iter()
            .filter(move |n| n.component.kind() == kind)
    }

    /// The single node of an infrastructure kind (database, cache, queue,
    /// auth, ...), if declared.
    pub fn singleton(&self, kind: NodeKind) -> Option<&Node> {
        self.nodes_of_kind(kind).next()
    }

    /// Finds a named component (api/service/worker) by kind and name.
    pub fn find_named(&self, kind: NodeKind, name: &str) -> Option<&Node> {
        self.nodes_of_kind(kind)
            .find(|n| n.component.name() == Some(name))
    }

    pub fn edges_from(&self, node: NodeId) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter(move |e| e.from == node)
    }

    pub fn edges_to(&self, node: NodeId) -> impl Iterator<Item = &Edge> {
        self.edges.iter().filter(move |e| e.to == node)
    }

    /// The pipeline owned by the given api/worker node, if any.
    pub fn pipeline_of(&self, owner: NodeId) -> Option<&Pipeline> {
        self.pipelines.iter().find(|p| p.owner == owner)
    }

    /// Renders the graph in Graphviz DOT format.
    pub fn to_dot(&self) -> String {
        use std::fmt::Write;
        let mut out = String::new();
        let _ = writeln!(out, "digraph {} {{", sanitize_dot_id(&self.name));
        let _ = writeln!(out, "    rankdir=LR;");
        for node in &self.nodes {
            let shape = match node.component.kind() {
                NodeKind::Api => "rarrow",
                NodeKind::Service => "box",
                NodeKind::Worker => "component",
                NodeKind::Database | NodeKind::Cache => "cylinder",
                NodeKind::Queue => "cds",
                NodeKind::Stream => "parallelogram",
                NodeKind::Auth => "hexagon",
                NodeKind::Logging | NodeKind::Metrics => "note",
            };
            let _ = writeln!(
                out,
                "    n{} [label=\"{}\", shape={shape}];",
                node.id.0,
                node.component.label()
            );
        }
        for edge in &self.edges {
            let style = match edge.kind {
                EdgeKind::RequestFlow => "solid",
                EdgeKind::DataFlow => "bold",
                EdgeKind::AsyncMessage => "dashed",
                EdgeKind::DependsOn => "dotted",
            };
            let _ = writeln!(
                out,
                "    n{} -> n{} [label=\"{:?}\", style={style}];",
                edge.from.0, edge.to.0, edge.kind
            );
        }
        out.push_str("}\n");
        out
    }
}

fn sanitize_dot_id(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    if cleaned.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        format!("_{cleaned}")
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::{DbEngine, QueueEngine};

    fn sample() -> SystemGraph {
        let mut g = SystemGraph::new("Test");
        let api = g.add_node(
            Component::Api {
                name: "Upload".into(),
                request: None,
            },
            None,
        );
        let svc = g.add_node(
            Component::Service {
                name: "Store".into(),
            },
            None,
        );
        let db = g.add_node(
            Component::Database {
                engine: DbEngine::Postgres,
            },
            None,
        );
        g.add_edge(api, svc, EdgeKind::RequestFlow);
        g.add_edge(svc, db, EdgeKind::DataFlow);
        g
    }

    #[test]
    fn ids_are_stable_and_lookup_works() {
        let g = sample();
        assert_eq!(g.nodes().count(), 3);
        let api = g.find_named(NodeKind::Api, "Upload").expect("api exists");
        assert_eq!(api.id, NodeId(0));
        assert!(g.singleton(NodeKind::Database).is_some());
        assert!(g.singleton(NodeKind::Queue).is_none());
    }

    #[test]
    fn duplicate_edges_are_deduplicated() {
        let mut g = sample();
        let first = g.add_edge(NodeId(0), NodeId(1), EdgeKind::RequestFlow);
        assert_eq!(g.edges().count(), 2);
        assert_eq!(first, EdgeId(0));
    }

    #[test]
    fn serializes_to_json() {
        let mut g = SystemGraph::new("Test");
        g.add_node(
            Component::Queue {
                engine: QueueEngine::Nats,
            },
            None,
        );
        let json = serde_json::to_value(&g).expect("serializes");
        assert_eq!(json["name"], "Test");
        assert_eq!(json["nodes"][0]["kind"], "Queue");
    }

    #[test]
    fn dot_output_contains_nodes_and_edges() {
        let dot = sample().to_dot();
        assert!(dot.contains("digraph Test"));
        assert!(dot.contains("api Upload"));
        assert!(dot.contains("n1 -> n2"));
    }
}
