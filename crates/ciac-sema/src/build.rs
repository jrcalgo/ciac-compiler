//! AST → [`SystemGraph`] lowering.
//!
//! This stage resolves names, checks that constructs are backed by the
//! capabilities the `use { .. }` block declares, and expands the
//! higher-level `crud`/`events` constructs into primitive components, so
//! the passes in [`crate::passes`] and every backend see only primitives.

use ciac_diagnostics::{Diagnostic, Diagnostics, ErrorCode, Span};
use ciac_ir::{
    AuthScheme, CacheEngine, Component, DbEngine, EdgeKind, EventStream, LoggingProvider,
    MetricsProvider, NodeId, NodeKind, Pipeline, QueueEngine, Resource, Step, SystemGraph,
};
use ciac_syntax::ast::{ComponentDecl, Ident, Item, PipelineDecl, Program, UseEntry};
use heck::ToSnakeCase;
use std::collections::HashMap;

pub fn build_graph(program: &Program, diags: &mut Diagnostics) -> SystemGraph {
    Builder::new(diags).build(program)
}

struct Builder<'d> {
    diags: &'d mut Diagnostics,
    graph: SystemGraph,
    /// Declared component names (apis, workers, crud/events resources) with
    /// their declaration spans, for duplicate detection and step resolution.
    declared: HashMap<String, (NodeKind, Span)>,
    /// Handler service nodes created implicitly from pipeline steps.
    handlers: HashMap<String, NodeId>,
}

impl<'d> Builder<'d> {
    fn new(diags: &'d mut Diagnostics) -> Self {
        Self {
            diags,
            graph: SystemGraph::default(),
            declared: HashMap::new(),
            handlers: HashMap::new(),
        }
    }

    fn build(mut self, program: &Program) -> SystemGraph {
        self.service_name(program);
        // Capabilities first: components and pipelines reference them.
        for item in &program.items {
            if let Item::Use(block) = item {
                for entry in &block.entries {
                    self.use_entry(entry);
                }
            }
        }
        for item in &program.items {
            match item {
                Item::Api(decl) => self.declare_component(decl, NodeKind::Api),
                Item::Worker(decl) => self.declare_component(decl, NodeKind::Worker),
                Item::Crud(decl) => self.crud(decl),
                Item::Events(decl) => self.events(decl),
                Item::Service(_) | Item::Use(_) | Item::Pipeline(_) => {}
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

    fn declare_component(&mut self, decl: &ComponentDecl, kind: NodeKind) {
        if !self.register(&decl.name, kind, decl.span) {
            return;
        }
        let name = decl.name.text.clone();
        let component = match kind {
            NodeKind::Api => Component::Api { name },
            NodeKind::Worker => Component::Worker { name },
            _ => unreachable!("only apis and workers are declared directly"),
        };
        self.graph.add_node(component, Some(decl.span));
    }

    /// Expands `crud <Name>;` into API → (Auth) → Service → Database
    /// (+ Cache) and records the resource for CRUD-aware codegen.
    fn crud(&mut self, decl: &ComponentDecl) {
        if !self.register(&decl.name, NodeKind::Api, decl.span) {
            return;
        }
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
        let api = self
            .graph
            .add_node(Component::Api { name: name.clone() }, Some(decl.span));
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
        self.graph.resources.push(Resource { name, api, service });
    }

    /// Expands `events <Name>;` into Queue → Worker (→ Database) and
    /// records the stream so producers and codegen agree on the subject.
    fn events(&mut self, decl: &ComponentDecl) {
        if !self.register(&decl.name, NodeKind::Worker, decl.span) {
            return;
        }
        let Some(queue) = self.graph.singleton(NodeKind::Queue).map(|n| n.id) else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::MissingCapability,
                    format!("`events {}` requires a queue", decl.name.text),
                )
                .with_label(decl.span, "declared here")
                .with_help("add `queue NATS;` (or `queue Kafka;`) to the `use { .. }` block"),
            );
            return;
        };
        let name = decl.name.text.clone();
        let worker = self.graph.add_node(
            Component::Worker {
                name: format!("{name}Consumer"),
            },
            Some(decl.span),
        );
        self.graph.add_edge(queue, worker, EdgeKind::AsyncMessage);
        if let Some(db) = self.graph.singleton(NodeKind::Database).map(|n| n.id) {
            self.graph.add_edge(worker, db, EdgeKind::DataFlow);
        }
        let subject = name.to_snake_case();
        self.graph.event_streams.push(EventStream {
            name,
            worker,
            subject,
        });
    }

    fn pipeline(&mut self, decl: &PipelineDecl) {
        // The pipeline name must match a declared api or worker.
        let owner = self
            .graph
            .find_named(NodeKind::Api, &decl.name.text)
            .or_else(|| self.graph.find_named(NodeKind::Worker, &decl.name.text))
            .map(|n| (n.id, n.component.kind()));
        let Some((owner, owner_kind)) = owner else {
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

        // A worker pipeline means the worker consumes from the queue.
        if owner_kind == NodeKind::Worker {
            match self.graph.singleton(NodeKind::Queue).map(|n| n.id) {
                Some(queue) => {
                    self.graph.add_edge(queue, owner, EdgeKind::AsyncMessage);
                }
                None => {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::MissingCapability,
                            format!(
                                "worker pipeline `{}` requires a queue to consume from",
                                decl.name.text
                            ),
                        )
                        .with_label(decl.span, "worker pipelines are queue-driven")
                        .with_help(
                            "add `queue NATS;` (or `queue Kafka;`) to the `use { .. }` block",
                        ),
                    );
                }
            }
        }

        let mut steps = Vec::new();
        let mut step_spans = Vec::new();
        let mut prev = owner;
        for ident in &decl.steps {
            let Some(step) = self.resolve_step(ident) else {
                continue;
            };
            match step {
                Step::Auth { node } | Step::Handler { node } => {
                    self.graph.add_edge(prev, node, EdgeKind::RequestFlow);
                    prev = node;
                }
                Step::Queue { node } => {
                    self.graph.add_edge(prev, node, EdgeKind::AsyncMessage);
                    // Later synchronous steps still run in the caller's
                    // context: publishing is fire-and-forget, so `prev`
                    // stays unchanged.
                }
                Step::Return => {}
            }
            steps.push(step);
            step_spans.push(Some(ident.span));
        }
        self.graph.pipelines.push(Pipeline {
            name: decl.name.text.clone(),
            owner,
            steps,
            span: Some(decl.span),
            step_spans,
        });
    }

    fn resolve_step(&mut self, ident: &Ident) -> Option<Step> {
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
            "Queue" => match self.graph.singleton(NodeKind::Queue) {
                Some(node) => Some(Step::Queue { node: node.id }),
                None => {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::MissingCapability,
                            "the `Queue` step requires a queue capability",
                        )
                        .with_label(ident.span, "used here")
                        .with_help(
                            "add `queue NATS;` (or `queue Kafka;`) to the `use { .. }` block",
                        ),
                    );
                    None
                }
            },
            "Return" => Some(Step::Return),
            name => {
                // Referencing a declared api/worker as a step is invalid;
                // any other name declares an implicit service handler.
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
                            "apis receive requests and workers consume queue messages; \
                             pipeline steps are logic handlers, `Auth`, `Queue`, or `Return`",
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
        _ => "component",
    }
}
