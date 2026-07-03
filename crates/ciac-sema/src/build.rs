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
    ApiConfig, AuthScheme, CacheEngine, Component, CrudConfig, DbEngine, EdgeKind, EventStream,
    FieldType, HttpMethod, LoggingProvider, MatchArm, MetricsProvider, NodeId, NodeKind, Pipeline,
    QueueEngine, Record, RecordField, RecordId, Resource, Step, StepKind, SystemGraph,
};
use ciac_syntax::ast::{
    ApiDecl, Attr, AttrValue, ComponentDecl, CrudDecl, Ident, Item, PipelineDecl, Program,
    RecordDecl, StepExpr, StreamDecl, TypeExpr, UseEntry, WorkerDecl,
};
use heck::{ToKebabCase, ToSnakeCase};
use std::collections::{BTreeMap, HashMap};

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
    /// Resolved API paths and stream subjects, for duplicate checks.
    route_paths: BTreeMap<String, Span>,
    stream_subjects: BTreeMap<String, Span>,
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
            route_paths: BTreeMap::new(),
            stream_subjects: BTreeMap::new(),
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
        self.check_scoped_apis();
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
        subject: String,
        queue: NodeId,
        span: Option<Span>,
    ) -> NodeId {
        if let Some(first) = self.stream_subjects.get(&subject) {
            let mut diag = Diagnostic::new(
                ErrorCode::DuplicateDeclaration,
                format!("stream subject `{subject}` is used more than once"),
            )
            .with_label(*first, "first subject used here");
            if let Some(span) = span {
                diag = diag.with_label(span, "duplicate subject here");
            }
            self.diags.push(diag);
        } else if let Some(span) = span {
            self.stream_subjects.insert(subject.clone(), span);
        }
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
        let subject =
            attrs::apply_stream_attrs(&decl.attrs, &self.graph.name, &decl.name.text, self.diags);
        self.add_stream(&decl.name.text, record, subject, queue, Some(decl.span));
    }

    /// The stream backing the legacy `Queue` step and unbound workers:
    /// `<service>.events`, untyped, created on first use.
    fn default_stream(&mut self, queue: NodeId) -> NodeId {
        match self.default_stream {
            Some(node) => node,
            None => {
                let subject = default_subject(&self.graph.name, "Events");
                let node = self.add_stream("Events", None, subject, queue, None);
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
        let config = attrs::apply_api_attrs(&decl.attrs, request.is_some(), self.diags);
        let path = config
            .path
            .clone()
            .unwrap_or_else(|| format!("/{}", decl.name.text.to_kebab_case()));
        if let Some(first) = self.route_paths.get(&path) {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::DuplicateDeclaration,
                    format!("api route path `{path}` is used more than once"),
                )
                .with_label(decl.span, "duplicate route here")
                .with_label(*first, "first route used here"),
            );
        } else {
            self.route_paths.insert(path, decl.span);
        }
        self.graph.add_node(
            Component::Api {
                name: decl.name.text.clone(),
                request,
                config,
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
                config: attrs::apply_worker_attrs(&decl.attrs, self.diags),
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
        let config = attrs::apply_crud_attrs(
            &decl.attrs,
            self.graph.singleton(NodeKind::Cache).is_some(),
            self.diags,
        );
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
                config: ApiConfig::default(),
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
            config,
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
        let subject = default_subject(&self.graph.name, &name);
        let stream = self.add_stream(&name, None, subject, queue, Some(decl.span));
        let worker = self.graph.add_node(
            Component::Worker {
                name: format!("{name}Consumer"),
                config: Default::default(),
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
        for expr in &decl.steps {
            let Some(step) = self.resolve_step(expr, payload) else {
                continue;
            };
            steps.push(step);
        }
        self.wire_steps(owner, &steps);
        self.graph.pipelines.push(Pipeline {
            name: decl.name.text.clone(),
            owner,
            payload,
            steps,
            span: Some(decl.span),
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
                return Some(step(StepKind::Publish { stream }, expr.span()));
            }
            StepExpr::Match { field, arms, .. } => {
                return self.resolve_match(field, arms, payload, expr.span());
            }
            StepExpr::Name(ident) => ident,
        };
        match ident.text.as_str() {
            "Auth" => match self.graph.singleton(NodeKind::Auth) {
                Some(node) => Some(step(StepKind::Auth { node: node.id }, ident.span)),
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
                Some(step(StepKind::Publish { stream }, ident.span))
            }
            "Return" => Some(step(StepKind::Return, ident.span)),
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
                Some(step(StepKind::Handler { node }, ident.span))
            }
        }
    }

    fn resolve_match(
        &mut self,
        field: &Ident,
        arms: &[ciac_syntax::ast::Arm],
        payload: Option<RecordId>,
        span: Span,
    ) -> Option<Step> {
        let Some(payload) = payload else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidMatch,
                    "`match` requires a typed pipeline payload",
                )
                .with_label(field.span, "matched field on an untyped payload"),
            );
            return None;
        };
        let record = self.graph.record(payload);
        let Some(record_field) = record.fields.iter().find(|f| f.name == field.text) else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidMatch,
                    format!("record `{}` has no field `{}`", record.name, field.text),
                )
                .with_label(field.span, "unknown field"),
            );
            return None;
        };
        let FieldType::Enum { variants } = &record_field.ty else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidMatch,
                    format!("field `{}` is not an enum", field.text),
                )
                .with_label(field.span, "match fields must be enum fields"),
            );
            return None;
        };
        let variants = variants.clone();
        let mut seen: BTreeMap<String, Span> = BTreeMap::new();
        let mut saw_wildcard = false;
        let mut ir_arms = Vec::new();
        for (idx, arm) in arms.iter().enumerate() {
            let label = match &arm.label {
                ciac_syntax::ast::ArmLabel::Default(label_span) => {
                    if saw_wildcard {
                        self.diags.push(
                            Diagnostic::new(
                                ErrorCode::DuplicateDeclaration,
                                "match wildcard arm appears more than once",
                            )
                            .with_label(*label_span, "duplicate wildcard here"),
                        );
                    }
                    if idx + 1 != arms.len() {
                        self.diags.push(
                            Diagnostic::new(
                                ErrorCode::InvalidMatch,
                                "match wildcard arm must be last",
                            )
                            .with_label(*label_span, "`_` must be the trailing arm"),
                        );
                    }
                    saw_wildcard = true;
                    None
                }
                ciac_syntax::ast::ArmLabel::Variant(ident) => {
                    if !variants.iter().any(|v| v == &ident.text) {
                        self.diags.push(
                            Diagnostic::new(
                                ErrorCode::InvalidMatch,
                                format!("`{}` is not a variant of `{}`", ident.text, field.text),
                            )
                            .with_label(ident.span, "unknown enum variant"),
                        );
                    }
                    if let Some(first) = seen.get(&ident.text) {
                        self.diags.push(
                            Diagnostic::new(
                                ErrorCode::DuplicateDeclaration,
                                format!("match arm `{}` appears more than once", ident.text),
                            )
                            .with_label(ident.span, "duplicate arm here")
                            .with_label(*first, "first arm here"),
                        );
                    } else {
                        seen.insert(ident.text.clone(), ident.span);
                    }
                    Some(ident.text.clone())
                }
            };
            let steps = arm
                .steps
                .iter()
                .filter_map(|step| self.resolve_step(step, Some(payload)))
                .collect();
            ir_arms.push(MatchArm { label, steps });
        }
        if !saw_wildcard {
            let missing: Vec<&str> = variants
                .iter()
                .map(String::as_str)
                .filter(|variant| !seen.contains_key(*variant))
                .collect();
            if !missing.is_empty() {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::NonExhaustiveMatch,
                        format!(
                            "match on `{}` does not cover {}",
                            field.text,
                            missing.join(", ")
                        ),
                    )
                    .with_label(span, "non-exhaustive match"),
                );
            }
        }
        Some(step(
            StepKind::Match {
                field: field.text.clone(),
                arms: ir_arms,
            },
            span,
        ))
    }

    fn wire_steps(&mut self, start: NodeId, steps: &[Step]) -> NodeId {
        let mut prev = start;
        for step in steps {
            match &step.kind {
                StepKind::Auth { node } | StepKind::Handler { node } => {
                    self.graph.add_edge(prev, *node, EdgeKind::RequestFlow);
                    prev = *node;
                }
                StepKind::Publish { stream } => {
                    self.graph.add_edge(prev, *stream, EdgeKind::AsyncMessage);
                    // Publishing is fire-and-forget: later synchronous
                    // steps still run in the caller's context.
                }
                StepKind::Return => {}
                StepKind::Match { arms, .. } => {
                    for arm in arms {
                        self.wire_steps(prev, &arm.steps);
                    }
                }
            }
        }
        prev
    }

    fn check_scoped_apis(&mut self) {
        for node in self.graph.nodes_of_kind(NodeKind::Api) {
            let Component::Api { config, name, .. } = &node.component else {
                continue;
            };
            let Some(scope) = &config.scope else {
                continue;
            };
            let auth_first = self
                .graph
                .pipeline_of(node.id)
                .and_then(|pipeline| pipeline.steps.first())
                .is_some_and(|step| matches!(&step.kind, StepKind::Auth { .. }));
            if !auth_first {
                let mut diag = Diagnostic::new(
                    ErrorCode::InvalidAttributeValue,
                    format!("api `{name}` declares scope `{scope}` but is not gated by Auth"),
                )
                .with_help("put `Auth` first in the api pipeline or remove the `scope` attribute");
                if let Some(span) = node.span {
                    diag = diag.with_label(span, "scoped api declared here");
                }
                self.diags.push(diag);
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

fn step(kind: StepKind, span: Span) -> Step {
    Step {
        kind,
        span: Some(span),
    }
}

fn default_subject(service: &str, name: &str) -> String {
    format!("{}.{}", service.to_snake_case(), name.to_snake_case())
}

mod attrs {
    use super::*;

    pub fn apply_api_attrs(
        attrs: &[Attr],
        has_typed_body: bool,
        diags: &mut Diagnostics,
    ) -> ApiConfig {
        let mut config = ApiConfig::default();
        let mut seen = BTreeMap::new();
        for attr in attrs {
            if duplicate_attr(attr, &mut seen, diags) {
                continue;
            }
            match attr.name.text.as_str() {
                "method" => match ident_value(attr) {
                    Some("GET") => config.method = HttpMethod::Get,
                    Some("POST") => config.method = HttpMethod::Post,
                    Some("PUT") => config.method = HttpMethod::Put,
                    Some("DELETE") => config.method = HttpMethod::Delete,
                    Some("PATCH") => config.method = HttpMethod::Patch,
                    Some(_) | None => invalid_value(
                        attr,
                        "api `method` must be one of GET, POST, PUT, DELETE, or PATCH",
                        diags,
                    ),
                },
                "path" => match string_value(attr) {
                    Some(path) if path.starts_with('/') => config.path = Some(path.to_owned()),
                    Some(_) | None => {
                        invalid_value(attr, "api `path` must be a string starting with `/`", diags)
                    }
                },
                "scope" => match string_value(attr) {
                    Some(scope) => config.scope = Some(scope.to_owned()),
                    None => invalid_value(attr, "api `scope` must be a string", diags),
                },
                _ => unknown_attr(attr, "api", diags),
            }
        }
        if has_typed_body && matches!(config.method, HttpMethod::Get | HttpMethod::Delete) {
            if let Some(attr) = attrs.iter().find(|a| a.name.text == "method") {
                invalid_value(
                    attr,
                    "GET and DELETE apis cannot declare a typed request body",
                    diags,
                );
            }
        }
        config
    }

    pub fn apply_worker_attrs(attrs: &[Attr], diags: &mut Diagnostics) -> ciac_ir::WorkerConfig {
        let mut config = ciac_ir::WorkerConfig::default();
        let mut seen = BTreeMap::new();
        for attr in attrs {
            if duplicate_attr(attr, &mut seen, diags) {
                continue;
            }
            match attr.name.text.as_str() {
                "concurrency" => match int_value(attr) {
                    Some(value) if value >= 1 => config.concurrency = value,
                    Some(_) | None => {
                        invalid_value(attr, "worker `concurrency` must be an integer >= 1", diags)
                    }
                },
                "max_retries" => match int_value(attr) {
                    Some(value) => config.max_retries = value,
                    None => invalid_value(attr, "worker `max_retries` must be an integer", diags),
                },
                _ => unknown_attr(attr, "worker", diags),
            }
        }
        config
    }

    pub fn apply_stream_attrs(
        attrs: &[Attr],
        service_name: &str,
        stream_name: &str,
        diags: &mut Diagnostics,
    ) -> String {
        let mut subject = super::default_subject(service_name, stream_name);
        let mut seen = BTreeMap::new();
        for attr in attrs {
            if duplicate_attr(attr, &mut seen, diags) {
                continue;
            }
            match attr.name.text.as_str() {
                "subject" => match string_value(attr) {
                    Some(value) => subject = value.to_owned(),
                    None => invalid_value(attr, "stream `subject` must be a string", diags),
                },
                _ => unknown_attr(attr, "stream", diags),
            }
        }
        subject
    }

    pub fn apply_crud_attrs(
        attrs: &[Attr],
        has_cache: bool,
        diags: &mut Diagnostics,
    ) -> CrudConfig {
        let mut config = CrudConfig::default();
        let mut seen = BTreeMap::new();
        for attr in attrs {
            if duplicate_attr(attr, &mut seen, diags) {
                continue;
            }
            match attr.name.text.as_str() {
                "cache_ttl" => match int_value(attr) {
                    Some(value) if value >= 1 => {
                        if has_cache {
                            config.cache_ttl = value;
                        } else {
                            invalid_value(
                                attr,
                                "crud `cache_ttl` requires a cache capability",
                                diags,
                            );
                        }
                    }
                    Some(_) | None => {
                        invalid_value(attr, "crud `cache_ttl` must be an integer >= 1", diags)
                    }
                },
                "page_size" => match int_value(attr) {
                    Some(value) if value >= 1 => config.page_size = value,
                    Some(_) | None => {
                        invalid_value(attr, "crud `page_size` must be an integer >= 1", diags)
                    }
                },
                _ => unknown_attr(attr, "crud", diags),
            }
        }
        config
    }

    fn duplicate_attr(
        attr: &Attr,
        seen: &mut BTreeMap<String, Span>,
        diags: &mut Diagnostics,
    ) -> bool {
        if let Some(first) = seen.get(&attr.name.text) {
            diags.push(
                Diagnostic::new(
                    ErrorCode::DuplicateDeclaration,
                    format!("attribute `{}` appears more than once", attr.name.text),
                )
                .with_label(attr.span, "duplicate attribute here")
                .with_label(*first, "first attribute here"),
            );
            true
        } else {
            seen.insert(attr.name.text.clone(), attr.span);
            false
        }
    }

    fn ident_value(attr: &Attr) -> Option<&str> {
        match &attr.value {
            AttrValue::Ident(ident) => Some(ident.text.as_str()),
            _ => None,
        }
    }

    fn string_value(attr: &Attr) -> Option<&str> {
        match &attr.value {
            AttrValue::Str { value, .. } => Some(value.as_str()),
            _ => None,
        }
    }

    fn int_value(attr: &Attr) -> Option<u32> {
        match &attr.value {
            AttrValue::Number { value, .. } => u32::try_from(*value).ok(),
            _ => None,
        }
    }

    fn unknown_attr(attr: &Attr, target: &str, diags: &mut Diagnostics) {
        diags.push(
            Diagnostic::new(
                ErrorCode::UnknownAttribute,
                format!("unknown {target} attribute `{}`", attr.name.text),
            )
            .with_label(attr.name.span, "not valid for this declaration"),
        );
    }

    fn invalid_value(attr: &Attr, message: &str, diags: &mut Diagnostics) {
        diags.push(
            Diagnostic::new(ErrorCode::InvalidAttributeValue, message)
                .with_label(attr.value.span(), "invalid value"),
        );
    }
}
