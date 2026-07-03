//! AST → [`SystemGraph`] lowering.
//!
//! This stage resolves names and types, checks that constructs are backed
//! by the capabilities the `use { .. }` block declares, and expands the
//! higher-level `crud`/`events` constructs into primitive components, so
//! the passes in [`crate::passes`] and every backend see only primitives.
//!
//! Streams become graph nodes: publisher → stream → consumer edges make
//! message topology (and cycle detection) per-stream. The v0.1 `Queue`
//! step survives as sugar for publishing to an auto-created default
//! stream (`<service>.events`).

use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode, Span};
use ciac_ir::{
    AuthScheme, CacheEngine, Component, DbEngine, EdgeKind, EventStream, FieldType,
    LoggingProvider, MetricsProvider, NodeId, NodeKind, Pipeline, QueueEngine, Record, RecordField,
    RecordId, Resource, Step, SystemGraph,
};
use ciac_syntax::ast::{
    ApiDecl, ComponentDecl, CrudDecl, Ident, Item, PipelineDecl, Program, RecordDecl, StepExpr,
    StreamDecl, TypeExpr, UseEntry, WorkerDecl,
};
use heck::ToSnakeCase;
use std::collections::HashMap;

pub fn build_graph(program: &Program, diags: &mut Diagnostics) -> SystemGraph {
    Builder::new(diags).build(program)
}

struct Builder<'d> {
    diags: &'d mut Diagnostics,
    graph: SystemGraph,
    /// Declared component names (apis, workers, streams, crud/events
    /// expansions) with their declaration spans, for duplicate detection
    /// and step resolution.
    declared: HashMap<String, (NodeKind, Span)>,
    /// Record names → ids (types live in their own namespace).
    records: HashMap<String, (RecordId, Span)>,
    /// Handler service nodes created implicitly from pipeline steps.
    handlers: HashMap<String, NodeId>,
    /// Stream name → node, for `publish`/`on` resolution.
    streams: HashMap<String, NodeId>,
    /// Worker node → stream it consumes (from `on` or the default).
    worker_streams: HashMap<NodeId, NodeId>,
    /// Lazily-created default stream backing the legacy `Queue` step.
    default_stream: Option<NodeId>,
}

impl<'d> Builder<'d> {
    fn new(diags: &'d mut Diagnostics) -> Self {
        Self {
            diags,
            graph: SystemGraph::default(),
            declared: HashMap::new(),
            records: HashMap::new(),
            handlers: HashMap::new(),
            streams: HashMap::new(),
            worker_streams: HashMap::new(),
            default_stream: None,
        }
    }

    fn build(mut self, program: &Program) -> SystemGraph {
        self.service_name(program);
        // Types first: streams and components reference them.
        for item in &program.items {
            if let Item::Record(decl) = item {
                self.record(decl);
            }
        }
        // Capabilities next: streams and pipelines reference them.
        for item in &program.items {
            if let Item::Use(block) = item {
                for entry in &block.entries {
                    self.use_entry(entry);
                }
            }
        }
        for item in &program.items {
            if let Item::Stream(decl) = item {
                self.stream(decl);
            }
        }
        for item in &program.items {
            match item {
                Item::Api(decl) => self.api(decl),
                Item::Worker(decl) => self.worker(decl),
                Item::Crud(decl) => self.crud(decl),
                Item::Events(decl) => self.events(decl),
                Item::Service(_)
                | Item::Use(_)
                | Item::Record(_)
                | Item::Stream(_)
                | Item::Pipeline(_) => {}
            }
        }
        // Pipelines last: they reference declared components.
        for item in &program.items {
            if let Item::Pipeline(decl) = item {
                self.pipeline(decl);
            }
        }
        self.graph
    }

    fn service_name(&mut self, program: &Program) {
        let mut decls = program.items.iter().filter_map(|item| match item {
            Item::Service(decl) => Some(decl),
            _ => None,
        });
        match decls.next() {
            None => self.diags.push(
                Diagnostic::new(
                    ErrorCode::MissingServiceDeclaration,
                    "program does not declare a service",
                )
                .with_help("start the program with `service <Name>;`"),
            ),
            Some(first) => {
                self.graph.name = first.name.text.clone();
                for extra in decls {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::DuplicateDeclaration,
                            "a program describes exactly one service",
                        )
                        .with_label(extra.span, "second `service` declaration here")
                        .with_label(first.span, "first declared here"),
                    );
                }
            }
        }
    }

    fn record(&mut self, decl: &RecordDecl) {
        if let Some((_, first)) = self.records.get(&decl.name.text) {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::DuplicateDeclaration,
                    format!("record `{}` is declared more than once", decl.name.text),
                )
                .with_label(decl.span, "duplicate declaration here")
                .with_label(*first, "first declared here"),
            );
            return;
        }
        let mut fields: Vec<RecordField> = Vec::new();
        for field in &decl.fields {
            if fields.iter().any(|f| f.name == field.name.text) {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::DuplicateDeclaration,
                        format!(
                            "field `{}` is declared more than once in record `{}`",
                            field.name.text, decl.name.text
                        ),
                    )
                    .with_label(field.span, "duplicate field here"),
                );
                continue;
            }
            let ty = match &field.ty {
                TypeExpr::Named(name) => match FieldType::parse(&name.text) {
                    Some(ty) => ty,
                    None => {
                        self.diags.push(
                            Diagnostic::new(
                                ErrorCode::UnknownType,
                                format!("unknown field type `{}`", name.text),
                            )
                            .with_label(name.span, "not a known type")
                            .with_help(
                                "field types are String, Int, Float, Bool, Uuid, Timestamp, \
                                 Json, or an inline `enum { A, B }`",
                            ),
                        );
                        continue;
                    }
                },
                TypeExpr::Enum { variants, .. } => FieldType::Enum {
                    variants: variants.iter().map(|v| v.text.clone()).collect(),
                },
            };
            fields.push(RecordField {
                name: field.name.text.clone(),
                ty,
            });
        }
        let id = self.graph.add_record(Record {
            name: decl.name.text.clone(),
            fields,
        });
        self.records.insert(decl.name.text.clone(), (id, decl.span));
    }

    /// Resolves an optional record reference, reporting `CIAC0015` when
    /// the name is not declared.
    fn resolve_record(&mut self, name: &Ident) -> Option<RecordId> {
        match self.records.get(&name.text) {
            Some((id, _)) => Some(*id),
            None => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::UnknownType,
                        format!("unknown record `{}`", name.text),
                    )
                    .with_label(name.span, "no record with this name")
                    .with_help(format!("declare it with `record {} {{ .. }}`", name.text)),
                );
                None
            }
        }
    }

    fn use_entry(&mut self, entry: &UseEntry) {
        let capability = entry.capability.text.as_str();
        let provider = entry.provider.text.as_str();
        let component = match (capability, provider) {
            ("auth", "JWT") => Component::Auth {
                scheme: AuthScheme::Jwt,
            },
            ("db", "Postgres") => Component::Database {
                engine: DbEngine::Postgres,
            },
            ("cache", "Redis") => Component::Cache {
                engine: CacheEngine::Redis,
            },
            ("queue", "NATS") => Component::Queue {
                engine: QueueEngine::Nats,
            },
            ("queue", "Kafka") => Component::Queue {
                engine: QueueEngine::Kafka,
            },
            ("logging", "Structured") => Component::Logging {
                provider: LoggingProvider::Structured,
            },
            ("metrics", "Prometheus") => Component::Metrics {
                provider: MetricsProvider::Prometheus,
            },
            _ => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::UnknownProvider,
                        format!("unknown capability `{capability} {provider}`"),
                    )
                    .with_label(entry.span, "not a supported capability/provider pair")
                    .with_help(ErrorCode::UnknownProvider.explanation()),
                );
                return;
            }
        };
        if let Some(existing) = self.graph.singleton(component.kind()) {
            let mut diag = Diagnostic::new(
                ErrorCode::DuplicateCapability,
                format!("capability `{capability}` is declared more than once"),
            )
            .with_label(entry.span, "duplicate declaration here");
            if let Some(span) = existing.span {
                diag = diag.with_label(span, "first declared here");
            }
            self.diags.push(diag);
            return;
        }
        self.graph.add_node(component, Some(entry.span));
    }

    /// Registers a declared name, reporting duplicates. Returns false when
    /// the declaration collides with an earlier one.
    fn register(&mut self, name: &Ident, kind: NodeKind, span: Span) -> bool {
        if let Some((_, first)) = self.declared.get(&name.text) {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::DuplicateDeclaration,
                    format!("`{}` is declared more than once", name.text),
                )
                .with_label(span, "duplicate declaration here")
                .with_label(*first, "first declared here"),
            );
            return false;
        }
        self.declared.insert(name.text.clone(), (kind, span));
        true
    }

    /// Requires the queue (broker) capability, reporting `CIAC0005` with
    /// the given context otherwise.
    fn require_queue(&mut self, what: &str, span: Span) -> Option<NodeId> {
        match self.graph.singleton(NodeKind::Queue).map(|n| n.id) {
            Some(queue) => Some(queue),
            None => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::MissingCapability,
                        format!("{what} requires a queue capability"),
                    )
                    .with_label(span, "used here")
                    .with_help("add `queue NATS;` (or `queue Kafka;`) to the `use { .. }` block"),
                );
                None
            }
        }
    }

    /// Creates a stream node wired to the broker.
    fn add_stream(
        &mut self,
        name: &str,
        record: Option<RecordId>,
        queue: NodeId,
        span: Option<Span>,
    ) -> NodeId {
        let subject = format!(
            "{}.{}",
            self.graph.name.to_snake_case(),
            name.to_snake_case()
        );
        let node = self.graph.add_node(
            Component::Stream {
                name: name.to_owned(),
                subject,
                record,
            },
            span,
        );
        self.graph.add_edge(node, queue, EdgeKind::DependsOn);
        self.streams.insert(name.to_owned(), node);
        node
    }

    fn stream(&mut self, decl: &StreamDecl) {
        if !self.register(&decl.name, NodeKind::Stream, decl.span) {
            return;
        }
        let record = self.resolve_record(&decl.record);
        let Some(queue) = self.require_queue(&format!("stream `{}`", decl.name.text), decl.span)
        else {
            return;
        };
        self.add_stream(&decl.name.text, record, queue, Some(decl.span));
    }

    /// The stream backing the legacy `Queue` step and unbound workers:
    /// `<service>.events`, untyped, created on first use.
    fn default_stream(&mut self, queue: NodeId) -> NodeId {
        match self.default_stream {
            Some(node) => node,
            None => {
                let node = self.add_stream("Events", None, queue, None);
                self.default_stream = Some(node);
                node
            }
        }
    }

    /// Resolves a stream reference from `publish X` / `on X`.
    fn resolve_stream(&mut self, name: &Ident) -> Option<NodeId> {
        match self.streams.get(&name.text) {
            Some(node) => Some(*node),
            None => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::UnknownStream,
                        format!("unknown stream `{}`", name.text),
                    )
                    .with_label(name.span, "no stream with this name")
                    .with_help(format!("declare it with `stream {}: <Record>;`", name.text)),
                );
                None
            }
        }
    }

    fn api(&mut self, decl: &ApiDecl) {
        if !self.register(&decl.name, NodeKind::Api, decl.span) {
            return;
        }
        let request = decl.request.as_ref().and_then(|r| self.resolve_record(r));
        self.graph.add_node(
            Component::Api {
                name: decl.name.text.clone(),
                request,
            },
            Some(decl.span),
        );
    }

    fn worker(&mut self, decl: &WorkerDecl) {
        if !self.register(&decl.name, NodeKind::Worker, decl.span) {
            return;
        }
        let node = self.graph.add_node(
            Component::Worker {
                name: decl.name.text.clone(),
            },
            Some(decl.span),
        );
        if let Some(stream_name) = &decl.stream {
            let Some(stream) = self.resolve_stream(stream_name) else {
                return;
            };
            self.graph.add_edge(stream, node, EdgeKind::AsyncMessage);
            self.worker_streams.insert(node, stream);
        }
    }

    /// Expands `crud <Name>;` into API → (Auth) → Service → Database
    /// (+ Cache) and records the resource for CRUD-aware codegen.
    fn crud(&mut self, decl: &CrudDecl) {
        if !self.register(&decl.name, NodeKind::Api, decl.span) {
            return;
        }
        let record = decl.record.as_ref().and_then(|r| self.resolve_record(r));
        let Some(db) = self.graph.singleton(NodeKind::Database).map(|n| n.id) else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::MissingCapability,
                    format!("`crud {}` requires a database", decl.name.text),
                )
                .with_label(decl.span, "declared here")
                .with_help("add `db Postgres;` to the `use { .. }` block"),
            );
            return;
        };
        let name = decl.name.text.clone();
        let api = self.graph.add_node(
            Component::Api {
                name: name.clone(),
                request: record,
            },
            Some(decl.span),
        );
        let service = self.graph.add_node(
            Component::Service {
                name: format!("{name}Store"),
            },
            Some(decl.span),
        );
        match self.graph.singleton(NodeKind::Auth).map(|n| n.id) {
            Some(auth) => {
                self.graph.add_edge(api, auth, EdgeKind::RequestFlow);
                self.graph.add_edge(auth, service, EdgeKind::RequestFlow);
            }
            None => {
                self.graph.add_edge(api, service, EdgeKind::RequestFlow);
            }
        }
        self.graph.add_edge(service, db, EdgeKind::DataFlow);
        if let Some(cache) = self.graph.singleton(NodeKind::Cache).map(|n| n.id) {
            self.graph.add_edge(service, cache, EdgeKind::DataFlow);
        }
        self.graph.resources.push(Resource {
            name,
            api,
            service,
            record,
        });
    }

    /// Expands `events <Name>;` into Stream → Worker (→ Database).
    fn events(&mut self, decl: &ComponentDecl) {
        if !self.register(&decl.name, NodeKind::Stream, decl.span) {
            return;
        }
        let Some(queue) = self.require_queue(&format!("`events {}`", decl.name.text), decl.span)
        else {
            return;
        };
        let name = decl.name.text.clone();
        let stream = self.add_stream(&name, None, queue, Some(decl.span));
        let worker = self.graph.add_node(
            Component::Worker {
                name: format!("{name}Consumer"),
            },
            Some(decl.span),
        );
        self.graph.add_edge(stream, worker, EdgeKind::AsyncMessage);
        self.worker_streams.insert(worker, stream);
        if let Some(db) = self.graph.singleton(NodeKind::Database).map(|n| n.id) {
            self.graph.add_edge(worker, db, EdgeKind::DataFlow);
        }
        self.graph.event_streams.push(EventStream {
            name,
            stream,
            worker,
        });
    }

    fn pipeline(&mut self, decl: &PipelineDecl) {
        // The pipeline name must match a declared api or worker.
        let owner = self
            .graph
            .find_named(NodeKind::Api, &decl.name.text)
            .or_else(|| self.graph.find_named(NodeKind::Worker, &decl.name.text))
            .map(|n| (n.id, n.component.clone()));
        let Some((owner, owner_component)) = owner else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::UnknownPipelineTarget,
                    format!(
                        "pipeline `{}` does not match any declared api or worker",
                        decl.name.text
                    ),
                )
                .with_label(decl.name.span, "no api or worker with this name")
                .with_help(format!(
                    "declare `api {};` or `worker {};`",
                    decl.name.text, decl.name.text
                )),
            );
            return;
        };
        if self.graph.pipeline_of(owner).is_some() {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::DuplicateDeclaration,
                    format!("`{}` already has a pipeline", decl.name.text),
                )
                .with_label(decl.span, "second pipeline declared here"),
            );
            return;
        }
        if decl.steps.is_empty() {
            self.diags.push(
                Diagnostic::new(ErrorCode::EmptyPipeline, "pipeline has no steps")
                    .with_label(decl.span, "expected at least one step"),
            );
            return;
        }

        // The payload type every step of this pipeline handles.
        let payload = match &owner_component {
            Component::Api { request, .. } => *request,
            Component::Worker { .. } => {
                // A worker pipeline consumes a stream: the bound one, or
                // the service's default stream (v0.1 behavior).
                let stream = match self.worker_streams.get(&owner) {
                    Some(stream) => Some(*stream),
                    None => self
                        .require_queue(&format!("worker pipeline `{}`", decl.name.text), decl.span)
                        .map(|queue| {
                            let stream = self.default_stream(queue);
                            self.graph.add_edge(stream, owner, EdgeKind::AsyncMessage);
                            self.worker_streams.insert(owner, stream);
                            stream
                        }),
                };
                stream.and_then(|s| match &self.graph.node(s).component {
                    Component::Stream { record, .. } => *record,
                    _ => None,
                })
            }
            _ => None,
        };

        let mut steps = Vec::new();
        let mut step_spans = Vec::new();
        let mut prev = owner;
        for expr in &decl.steps {
            let Some(step) = self.resolve_step(expr, payload) else {
                continue;
            };
            match step {
                Step::Auth { node } | Step::Handler { node } => {
                    self.graph.add_edge(prev, node, EdgeKind::RequestFlow);
                    prev = node;
                }
                Step::Publish { stream } => {
                    self.graph.add_edge(prev, stream, EdgeKind::AsyncMessage);
                    // Publishing is fire-and-forget: later synchronous
                    // steps still run in the caller's context.
                }
                Step::Return => {}
            }
            steps.push(step);
            step_spans.push(Some(expr.span()));
        }
        self.graph.pipelines.push(Pipeline {
            name: decl.name.text.clone(),
            owner,
            payload,
            steps,
            span: Some(decl.span),
            step_spans,
        });
    }

    /// Checks that publishing the pipeline payload onto `stream` is
    /// type-correct (`CIAC0016`). Untyped streams accept any payload.
    fn check_publish_type(&mut self, stream: NodeId, payload: Option<RecordId>, span: Span) {
        let Component::Stream {
            record: Some(expected),
            name,
            ..
        } = &self.graph.node(stream).component
        else {
            return;
        };
        if payload == Some(*expected) {
            return;
        }
        let expected_name = self.graph.record(*expected).name.clone();
        let found = match payload {
            Some(id) => format!("`{}`", self.graph.record(id).name),
            None => "untyped JSON".to_owned(),
        };
        let stream_name = name.clone();
        self.diags.push(
            Diagnostic::new(
                ErrorCode::TypeMismatch,
                format!(
                    "stream `{stream_name}` carries `{expected_name}` but the pipeline's \
                     payload is {found}"
                ),
            )
            .with_label(span, format!("publishes {found} here"))
            .with_help(format!(
                "type the pipeline's owner with `: {expected_name}` or publish to a \
                 stream carrying {found}"
            )),
        );
    }

    fn resolve_step(&mut self, expr: &StepExpr, payload: Option<RecordId>) -> Option<Step> {
        let ident = match expr {
            StepExpr::Publish(stream_name) => {
                let stream = self.resolve_stream(stream_name)?;
                self.check_publish_type(stream, payload, stream_name.span);
                return Some(Step::Publish { stream });
            }
            StepExpr::Name(ident) => ident,
        };
        match ident.text.as_str() {
            "Auth" => match self.graph.singleton(NodeKind::Auth) {
                Some(node) => Some(Step::Auth { node: node.id }),
                None => {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::MissingCapability,
                            "the `Auth` step requires an auth capability",
                        )
                        .with_label(ident.span, "used here")
                        .with_help("add `auth JWT;` to the `use { .. }` block"),
                    );
                    None
                }
            },
            "Queue" => {
                let queue = self.require_queue("the `Queue` step", ident.span)?;
                let stream = self.default_stream(queue);
                self.check_publish_type(stream, payload, ident.span);
                Some(Step::Publish { stream })
            }
            "Return" => Some(Step::Return),
            name => {
                // Referencing a declared api/worker/stream as a step is
                // invalid; any other name declares an implicit handler.
                if let Some((kind, _)) = self.declared.get(name) {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::IncompatibleComposition,
                            format!("`{name}` cannot be invoked as a pipeline step"),
                        )
                        .with_label(
                            ident.span,
                            format!("`{name}` is declared as a {}", kind_noun(*kind)),
                        )
                        .with_help(
                            "pipeline steps are logic handlers, `Auth`, `publish <Stream>`, \
                             or `Return`; streams are published to, not invoked",
                        ),
                    );
                    return None;
                }
                if name == self.graph.name {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::IncompatibleComposition,
                            format!("`{name}` is the service itself and cannot be a step"),
                        )
                        .with_label(ident.span, "refers to the enclosing service"),
                    );
                    return None;
                }
                let node = match self.handlers.get(name) {
                    Some(id) => *id,
                    None => {
                        let id = self.graph.add_node(
                            Component::Service {
                                name: name.to_owned(),
                            },
                            Some(ident.span),
                        );
                        // Handlers are provisioned with the declared storage
                        // capabilities (injected by codegen).
                        for kind in [NodeKind::Database, NodeKind::Cache] {
                            if let Some(target) = self.graph.singleton(kind).map(|n| n.id) {
                                self.graph.add_edge(id, target, EdgeKind::DataFlow);
                            }
                        }
                        self.handlers.insert(name.to_owned(), id);
                        id
                    }
                };
                Some(Step::Handler { node })
            }
        }
    }
}

fn kind_noun(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Api => "api",
        NodeKind::Worker => "worker",
        NodeKind::Service => "service",
        NodeKind::Stream => "stream",
        _ => "component",
    }
}
