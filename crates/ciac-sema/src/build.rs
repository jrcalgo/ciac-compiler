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
    ApiConfig, AuthScheme, CacheEngine, ChannelConfig, Component, CrudConfig, DbEngine, EdgeKind,
    EmailProvider, EventStream, FieldType, HandlerBody, HttpMethod, JobConfig, LoggingProvider,
    MatchArm, MetricsProvider, NodeId, NodeKind, ObjectStoreProvider, Pipeline, QueueEngine,
    RealtimeProvider, Record, RecordField, RecordId, RecordKind as IrRecordKind, Resource,
    SchedulerProvider, SearchProvider, ServiceId, Step, StepKind, SystemGraph, Table, TableId,
};
use ciac_syntax::ast::{
    ApiDecl, Attr, AttrValue, ChannelDecl, ComponentDecl, CrudDecl, HandlerDecl, Ident, Item,
    JobDecl, PipelineDecl, Program, RecordDecl, RecordKind, ServiceBlock, ServiceItem, StepExpr,
    StreamDecl, TableDecl, TypeExpr, UseEntry, WorkerDecl,
};
use heck::{ToKebabCase, ToSnakeCase};
use std::collections::{BTreeMap, HashMap};

pub fn build_graph(program: &Program, diags: &mut Diagnostics) -> SystemGraph {
    Builder::new(diags).build(program)
}

// `pub(crate)` (struct + the fields `typeck.rs` touches directly): the
// type checker is implemented as a second `impl Builder` block in that
// file so it can reuse `resolve_record`/`resolve_stream`/
// `default_capability`/etc. as-is instead of re-deriving capability and
// type resolution behind a narrower free-function API.
pub(crate) struct Builder<'d> {
    pub(crate) diags: &'d mut Diagnostics,
    pub(crate) graph: SystemGraph,
    /// Declared component names (apis, workers, streams, crud/events
    /// expansions) with their declaration spans, for duplicate detection
    /// and step resolution.
    declared: HashMap<String, (NodeKind, Span)>,
    service_ids: HashMap<String, (ServiceId, Span)>,
    pub(crate) current_service: Option<ServiceId>,
    /// Record names → ids (types live in their own namespace).
    records: HashMap<String, (RecordId, Span)>,
    /// Table names → ids (v0.7 M2; own namespace, resolved after records).
    tables: HashMap<String, (TableId, Span)>,
    /// Handler service nodes created implicitly from pipeline steps.
    handlers: HashMap<String, NodeId>,
    /// Handler declarations by name; missing entries keep legacy implicit bindings.
    handler_bindings: HashMap<String, Vec<BindingSpec>>,
    /// Type-checked signatures for v0.7 handlers (inline body or
    /// `extern`), attached to the node when/if it's created. Absent for
    /// the classic binding-only form, which keeps using
    /// `handler_bindings` above.
    handler_signatures: HashMap<String, HandlerBody>,
    handler_decl_spans: HashMap<String, Span>,
    /// Stream name → node, for `publish`/`on` resolution.
    streams: HashMap<String, NodeId>,
    /// Worker node → stream it consumes (from `on` or the default).
    worker_streams: HashMap<NodeId, NodeId>,
    /// Resolved API paths and stream subjects, for duplicate checks.
    route_paths: BTreeMap<String, Span>,
    channel_paths: BTreeMap<String, Span>,
    stream_subjects: BTreeMap<String, Span>,
    /// Lazily-created default stream backing the legacy `Queue` step.
    default_stream: Option<NodeId>,
}

#[derive(Debug, Clone)]
struct BindingSpec {
    kind: NodeKind,
    instance: String,
    span: Span,
}

impl<'d> Builder<'d> {
    fn new(diags: &'d mut Diagnostics) -> Self {
        Self {
            diags,
            graph: SystemGraph::default(),
            declared: HashMap::new(),
            service_ids: HashMap::new(),
            current_service: None,
            records: HashMap::new(),
            tables: HashMap::new(),
            handlers: HashMap::new(),
            handler_bindings: HashMap::new(),
            handler_signatures: HashMap::new(),
            handler_decl_spans: HashMap::new(),
            streams: HashMap::new(),
            worker_streams: HashMap::new(),
            route_paths: BTreeMap::new(),
            channel_paths: BTreeMap::new(),
            stream_subjects: BTreeMap::new(),
            default_stream: None,
        }
    }

    fn build(mut self, program: &Program) -> SystemGraph {
        let has_service_blocks = program
            .items
            .iter()
            .any(|item| matches!(item, Item::ServiceBlock(_)));
        self.project_name(program, has_service_blocks);
        // Types first: streams and components reference them. Tables
        // resolve after records (a table's backing record must already be
        // registered).
        for item in &program.items {
            if let Item::Record(decl) = item {
                self.record(decl);
            }
        }
        for item in &program.items {
            if let Item::Table(decl) = item {
                self.table(decl);
            }
        }
        self.register_services(program, has_service_blocks);
        self.graph.multi_service = has_service_blocks;
        if has_service_blocks {
            for item in &program.items {
                if let Item::Stream(decl) = item {
                    self.stream(decl, false);
                }
            }
            for item in &program.items {
                match item {
                    Item::Use(_)
                    | Item::Api(_)
                    | Item::Worker(_)
                    | Item::Job(_)
                    | Item::Channel(_)
                    | Item::Crud(_)
                    | Item::Events(_)
                    | Item::Handler(_)
                    | Item::Pipeline(_) => self.diags.push(
                        Diagnostic::new(
                            ErrorCode::InvalidServiceScope,
                            "service-local declarations must be inside a `service { .. }` block",
                        )
                        .with_label(
                            item_span(item),
                            "move this declaration into a service block",
                        ),
                    ),
                    Item::ServiceBlock(block) => self.build_service_block_decls(block),
                    Item::Import(_)
                    | Item::Project(_)
                    | Item::Service(_)
                    | Item::Record(_)
                    | Item::Stream(_)
                    | Item::Table(_) => {}
                    // `blueprints::expand` (v0.8 M2) always runs before
                    // `build_graph` and eliminates every `Blueprint`/
                    // `Expand` item from its output — unlike `Import`
                    // (resolved by a separate, optional pre-pass), a
                    // caller cannot reach `build_graph` without it.
                    Item::Blueprint(_) | Item::Expand(_) => {
                        unreachable!("blueprints::expand did not eliminate this item")
                    }
                }
            }
            for item in &program.items {
                if let Item::ServiceBlock(block) = item {
                    self.build_service_block_pipelines(block);
                }
            }
        } else {
            self.current_service = self.graph.services().next().map(|service| service.id);
            for item in &program.items {
                if let Item::Use(block) = item {
                    for entry in &block.entries {
                        self.use_entry(entry);
                    }
                }
            }
            for item in &program.items {
                if let Item::Stream(decl) = item {
                    self.stream(decl, true);
                }
            }
            self.build_flat_items(&program.items);
        }
        self.check_scoped_apis();
        self.graph
    }

    fn project_name(&mut self, program: &Program, has_service_blocks: bool) {
        let project = program
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Project(decl) => Some(decl),
                _ => None,
            })
            .next();
        if let Some(project) = project {
            self.graph.name = project.name.text.clone();
            return;
        }
        let mut decls = program.items.iter().filter_map(|item| match item {
            Item::Service(decl) => Some(decl),
            _ => None,
        });
        if has_service_blocks {
            self.graph.name = program
                .items
                .iter()
                .find_map(|item| match item {
                    Item::ServiceBlock(block) => Some(block.name.text.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "Project".to_owned());
            return;
        }
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

    fn register_services(&mut self, program: &Program, has_service_blocks: bool) {
        if has_service_blocks {
            for item in &program.items {
                if let Item::ServiceBlock(block) = item {
                    self.add_service_decl(&block.name, block.span);
                }
            }
        } else if let Some(decl) = program.items.iter().find_map(|item| match item {
            Item::Service(decl) => Some(decl),
            _ => None,
        }) {
            self.add_service_decl(&decl.name, decl.span);
        }
    }

    fn add_service_decl(&mut self, name: &Ident, span: Span) -> Option<ServiceId> {
        if let Some((_, first)) = self.service_ids.get(&name.text) {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::DuplicateService,
                    format!("service `{}` is declared more than once", name.text),
                )
                .with_label(span, "duplicate service here")
                .with_label(*first, "first declared here"),
            );
            return None;
        }
        let id = self.graph.add_service(name.text.clone(), Some(span));
        self.service_ids.insert(name.text.clone(), (id, span));
        Some(id)
    }

    fn enter_service(&mut self, block: &ServiceBlock) -> Option<Option<ServiceId>> {
        let (service, _) = self.service_ids.get(&block.name.text).copied()?;
        let previous = self.current_service;
        self.current_service = Some(service);
        Some(previous)
    }

    fn build_service_block_decls(&mut self, block: &ServiceBlock) {
        let Some(previous) = self.enter_service(block) else {
            return;
        };
        self.build_service_items(&block.items);
        self.current_service = previous;
    }

    fn build_service_block_pipelines(&mut self, block: &ServiceBlock) {
        let Some(previous) = self.enter_service(block) else {
            return;
        };
        for item in &block.items {
            if let ServiceItem::Pipeline(decl) = item {
                self.pipeline(decl);
            }
        }
        self.current_service = previous;
    }

    fn build_flat_items(&mut self, items: &[Item]) {
        for item in items {
            if let Item::Handler(decl) = item {
                self.handler_decl(decl);
            }
        }
        for item in items {
            match item {
                Item::Api(decl) => self.api(decl),
                Item::Worker(decl) => self.worker(decl),
                Item::Job(decl) => self.job(decl),
                Item::Channel(decl) => self.channel(decl),
                Item::Crud(decl) => self.crud(decl),
                Item::Events(decl) => self.events(decl),
                _ => {}
            }
        }
        for item in items {
            if let Item::Pipeline(decl) = item {
                self.pipeline(decl);
            }
        }
    }

    fn build_service_items(&mut self, items: &[ServiceItem]) {
        for item in items {
            if let ServiceItem::Use(block) = item {
                for entry in &block.entries {
                    self.use_entry(entry);
                }
            }
        }
        for item in items {
            if let ServiceItem::Handler(decl) = item {
                self.handler_decl(decl);
            }
        }
        for item in items {
            match item {
                ServiceItem::Api(decl) => self.api(decl),
                ServiceItem::Worker(decl) => self.worker(decl),
                ServiceItem::Job(decl) => self.job(decl),
                ServiceItem::Channel(decl) => self.channel(decl),
                ServiceItem::Crud(decl) => self.crud(decl),
                ServiceItem::Events(decl) => self.events(decl),
                ServiceItem::Use(_) | ServiceItem::Handler(_) | ServiceItem::Pipeline(_) => {}
                ServiceItem::Expand(_) => {
                    unreachable!("blueprints::expand did not eliminate this item")
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
        let kind = match decl.kind {
            RecordKind::Data => IrRecordKind::Data,
            RecordKind::Error => IrRecordKind::Error,
        };
        let id = self.graph.add_record(Record {
            name: decl.name.text.clone(),
            fields,
            kind,
        });
        self.records.insert(decl.name.text.clone(), (id, decl.span));
    }

    /// Resolves an optional record reference, reporting `CIAC0015` when
    /// the name is not declared.
    pub(crate) fn resolve_record(&mut self, name: &Ident) -> Option<RecordId> {
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

    /// `table <Name>: <Record>;` (v0.7). Mirrors `record`/`stream`: tables
    /// have their own namespace and resolve after records (a table's
    /// backing record must already be registered).
    fn table(&mut self, decl: &TableDecl) {
        if let Some((_, first)) = self.tables.get(&decl.name.text) {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::DuplicateDeclaration,
                    format!("table `{}` is declared more than once", decl.name.text),
                )
                .with_label(decl.span, "duplicate declaration here")
                .with_label(*first, "first declared here"),
            );
            return;
        }
        let Some(record) = self.resolve_record(&decl.record) else {
            return;
        };
        let id = self.graph.add_table(Table {
            name: decl.name.text.clone(),
            record,
        });
        self.tables.insert(decl.name.text.clone(), (id, decl.span));
    }

    /// Resolves a bare table name (e.g. the first argument of a `db.*`
    /// verb call) reported as `UnknownTable` when absent.
    pub(crate) fn resolve_table(&mut self, name: &Ident) -> Option<TableId> {
        match self.tables.get(&name.text) {
            Some((id, _)) => Some(*id),
            None => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::UnknownTable,
                        format!("unknown table `{}`", name.text),
                    )
                    .with_label(name.span, "no table with this name")
                    .with_help(format!("declare it with `table {}: <Record>;`", name.text)),
                );
                None
            }
        }
    }

    fn use_entry(&mut self, entry: &UseEntry) {
        let capability = entry.capability.text.as_str();
        let provider = entry.provider.as_ref().map(|p| p.text.as_str());
        let name = entry
            .name
            .as_ref()
            .map(|n| n.text.as_str())
            .unwrap_or("default");
        let component = match (capability, provider) {
            ("auth", Some("JWT")) => Component::Auth {
                name: name.to_owned(),
                scheme: AuthScheme::Jwt,
                issuer: None,
                audience: None,
            },
            ("auth", Some("OAuth2")) => {
                let Some(issuer) = attr_string(&entry.attrs, "issuer") else {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::UnsupportedProviderConfig,
                            "auth OAuth2 requires an `issuer` string attribute",
                        )
                        .with_label(entry.span, "missing `issuer`"),
                    );
                    return;
                };
                Component::Auth {
                    name: name.to_owned(),
                    scheme: AuthScheme::OAuth2,
                    issuer: Some(issuer),
                    audience: attr_string(&entry.attrs, "audience"),
                }
            }
            ("db", Some("Postgres")) => Component::Database {
                name: name.to_owned(),
                engine: DbEngine::Postgres,
            },
            ("db", Some("MySQL")) => Component::Database {
                name: name.to_owned(),
                engine: DbEngine::MySql,
            },
            ("db", Some("SQLite")) => Component::Database {
                name: name.to_owned(),
                engine: DbEngine::Sqlite,
            },
            ("cache", Some("Redis")) => Component::Cache {
                name: name.to_owned(),
                engine: CacheEngine::Redis,
            },
            ("queue", Some("NATS")) => Component::Queue {
                name: name.to_owned(),
                engine: QueueEngine::Nats,
            },
            ("queue", Some("Kafka")) => Component::Queue {
                name: name.to_owned(),
                engine: QueueEngine::Kafka,
            },
            ("logging", Some("Structured")) => Component::Logging {
                name: name.to_owned(),
                provider: LoggingProvider::Structured,
            },
            ("metrics", Some("Prometheus")) => Component::Metrics {
                name: name.to_owned(),
                provider: MetricsProvider::Prometheus,
            },
            ("object_store", Some("S3")) => Component::ObjectStore {
                name: name.to_owned(),
                provider: ObjectStoreProvider::S3,
                bucket: attr_string(&entry.attrs, "bucket"),
            },
            ("email", Some("SES")) => Component::Email {
                name: name.to_owned(),
                provider: EmailProvider::Ses,
            },
            ("email", Some("SMTP")) => Component::Email {
                name: name.to_owned(),
                provider: EmailProvider::Smtp,
            },
            ("search", Some("OpenSearch")) => Component::Search {
                name: name.to_owned(),
                provider: SearchProvider::OpenSearch,
            },
            ("external_http", None) => {
                let Some(base_url) = attr_string(&entry.attrs, "base_url") else {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::UnsupportedProviderConfig,
                            "external_http requires a `base_url` string attribute",
                        )
                        .with_label(entry.span, "missing `base_url`"),
                    );
                    return;
                };
                Component::ExternalHttp {
                    name: name.to_owned(),
                    base_url,
                }
            }
            ("scheduler", Some("Cron")) | ("scheduler", None) => Component::Scheduler {
                name: name.to_owned(),
                provider: SchedulerProvider::Cron,
            },
            ("realtime", Some("WebSocket")) => Component::Realtime {
                name: name.to_owned(),
                provider: RealtimeProvider::WebSocket,
            },
            ("realtime", Some("SSE")) | ("realtime", None) => Component::Realtime {
                name: name.to_owned(),
                provider: RealtimeProvider::Sse,
            },
            _ => {
                let provider = provider.unwrap_or("<none>");
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
        if let Some(existing) = self.find_capability(component.kind(), name) {
            let mut diag = Diagnostic::new(
                ErrorCode::DuplicateCapability,
                format!("capability `{capability} {name}` is declared more than once"),
            )
            .with_label(entry.span, "duplicate declaration here");
            if let Some(span) = existing.span {
                diag = diag.with_label(span, "first declared here");
            }
            self.diags.push(diag);
            return;
        }
        self.add_node(component, Some(entry.span));
    }

    fn handler_decl(&mut self, decl: &HandlerDecl) {
        let key = self.scoped_key(&decl.name.text);
        if let Some(first) = self.handler_decl_spans.get(&key) {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::DuplicateDeclaration,
                    format!("handler `{}` is declared more than once", decl.name.text),
                )
                .with_label(decl.span, "duplicate handler declaration here")
                .with_label(*first, "first declared here"),
            );
            return;
        }
        if decl.body.is_some() || decl.is_extern {
            // v0.7 M2: type-check the signature/body into HIR. Capability
            // dependencies come from verb resolution inside the body, not
            // declared bindings, so this handler skips the binding
            // bookkeeping below entirely. No backend implements either
            // form yet (`Backend::supports` gates it at build time), so a
            // handler that type-checks here still fails `ciac build` with
            // CIAC0011 — the same pattern Kafka already uses.
            if let Some(hir) = self.check_handler_body(decl) {
                self.handler_decl_spans.insert(key.clone(), decl.span);
                self.handler_signatures.insert(key, hir);
            }
            return;
        }
        let mut bindings = Vec::new();
        let mut seen = BTreeMap::new();
        for binding in &decl.bindings {
            let Some(kind) = binding_kind(&binding.capability.text) else {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::InvalidHandlerBinding,
                        format!("unknown handler binding `{}`", binding.capability.text),
                    )
                    .with_label(binding.capability.span, "not a bindable capability kind"),
                );
                continue;
            };
            if let Some(first) = seen.get(&kind) {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::DuplicateDeclaration,
                        format!(
                            "handler `{}` binds `{}` more than once",
                            decl.name.text, binding.capability.text
                        ),
                    )
                    .with_label(binding.span, "duplicate binding here")
                    .with_label(*first, "first binding here"),
                );
                continue;
            }
            seen.insert(kind, binding.span);
            bindings.push(BindingSpec {
                kind,
                instance: binding.instance.text.clone(),
                span: binding.span,
            });
        }
        self.handler_decl_spans.insert(key.clone(), decl.span);
        self.handler_bindings.insert(key, bindings);
    }

    /// Registers a declared name, reporting duplicates. Returns false when
    /// the declaration collides with an earlier one.
    fn register(&mut self, name: &Ident, kind: NodeKind, span: Span) -> bool {
        let key = self.scoped_key(&name.text);
        if let Some((_, first)) = self.declared.get(&key) {
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
        self.declared.insert(key, (kind, span));
        true
    }

    pub(crate) fn scoped_key(&self, name: &str) -> String {
        match self.current_service {
            Some(service) => format!("{}::{name}", service.0),
            None => name.to_owned(),
        }
    }

    pub(crate) fn add_node(&mut self, component: Component, span: Option<Span>) -> NodeId {
        self.graph
            .add_node_owned(self.current_service, component, span)
    }

    /// Requires the queue (broker) capability, reporting `CIAC0005` with
    /// the given context otherwise.
    pub(crate) fn require_queue(&mut self, what: &str, span: Span) -> Option<NodeId> {
        match self.default_capability(NodeKind::Queue, what, span) {
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

    pub(crate) fn default_capability(
        &mut self,
        kind: NodeKind,
        what: &str,
        span: Span,
    ) -> Option<NodeId> {
        let nodes: Vec<_> = self
            .graph
            .nodes_of_kind(kind)
            .filter(|node| node.service == self.current_service)
            .collect();
        if nodes.is_empty() {
            return None;
        }
        if let Some(default) = nodes
            .iter()
            .copied()
            .find(|node| node.component.name() == Some("default"))
        {
            return Some(default.id);
        }
        if nodes.len() == 1 {
            return Some(nodes[0].id);
        }
        self.diags.push(
            Diagnostic::new(
                ErrorCode::AmbiguousCapabilityBinding,
                format!("{what} needs a {kind:?} capability but multiple instances exist"),
            )
            .with_label(span, "ambiguous capability use")
            .with_help("name a `default` instance or add an explicit handler/resource binding"),
        );
        None
    }

    fn resolve_capability(&mut self, kind: NodeKind, name: &str, span: Span) -> Option<NodeId> {
        match self.find_capability(kind, name) {
            Some(node) => Some(node.id),
            None => {
                self.diags.push(
                    Diagnostic::new(
                        ErrorCode::UnknownCapabilityInstance,
                        format!("unknown {kind:?} capability instance `{name}`"),
                    )
                    .with_label(span, "no capability instance with this name"),
                );
                None
            }
        }
    }

    fn find_capability(&self, kind: NodeKind, name: &str) -> Option<&ciac_ir::Node> {
        self.graph.nodes_of_kind(kind).find(|node| {
            node.service == self.current_service && node.component.name() == Some(name)
        })
    }

    /// Creates a stream node wired to the broker.
    fn add_stream(
        &mut self,
        name: &str,
        record: Option<RecordId>,
        subject: String,
        queue: Option<NodeId>,
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
        let node = self.graph.add_node_owned(
            None,
            Component::Stream {
                name: name.to_owned(),
                subject,
                record,
            },
            span,
        );
        if let Some(queue) = queue {
            self.graph.add_edge(node, queue, EdgeKind::DependsOn);
        }
        self.streams.insert(name.to_owned(), node);
        node
    }

    fn stream(&mut self, decl: &StreamDecl, require_queue: bool) {
        if !self.register(&decl.name, NodeKind::Stream, decl.span) {
            return;
        }
        let record = self.resolve_record(&decl.record);
        let queue = if require_queue {
            let Some(queue) =
                self.require_queue(&format!("stream `{}`", decl.name.text), decl.span)
            else {
                return;
            };
            Some(queue)
        } else {
            None
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
                let node = self.add_stream("Events", None, subject, Some(queue), None);
                self.default_stream = Some(node);
                node
            }
        }
    }

    /// Resolves a stream reference from `publish X` / `on X`.
    pub(crate) fn resolve_stream(&mut self, name: &Ident) -> Option<NodeId> {
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
        self.add_node(
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
        let config = attrs::apply_worker_attrs(&decl.attrs, self.diags);
        let node = self.add_node(
            Component::Worker {
                name: decl.name.text.clone(),
                config,
            },
            Some(decl.span),
        );
        if let Some(stream_name) = &decl.stream {
            let Some(stream) = self.resolve_stream(stream_name) else {
                return;
            };
            if let Some(queue) =
                self.require_queue(&format!("worker `{}`", decl.name.text), stream_name.span)
            {
                self.graph.add_edge(stream, queue, EdgeKind::DependsOn);
            }
            self.graph.add_edge(stream, node, EdgeKind::AsyncMessage);
            self.worker_streams.insert(node, stream);
        }
    }

    fn job(&mut self, decl: &JobDecl) {
        if !self.register(&decl.name, NodeKind::Job, decl.span) {
            return;
        }
        let Some(config) = attrs::apply_job_attrs(&decl.attrs, self.diags) else {
            return;
        };
        let node = self.add_node(
            Component::Job {
                name: decl.name.text.clone(),
                config,
            },
            Some(decl.span),
        );
        let Some(scheduler) = self.default_capability(
            NodeKind::Scheduler,
            &format!("job `{}`", decl.name.text),
            decl.span,
        ) else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::MissingCapability,
                    format!("job `{}` requires a scheduler capability", decl.name.text),
                )
                .with_label(decl.span, "declared here")
                .with_help("add `scheduler Cron;` to the `use { .. }` block"),
            );
            return;
        };
        self.graph.add_edge(node, scheduler, EdgeKind::DependsOn);
    }

    fn channel(&mut self, decl: &ChannelDecl) {
        if !self.register(&decl.name, NodeKind::Channel, decl.span) {
            return;
        }
        let Some(stream) = self.resolve_stream(&decl.stream) else {
            return;
        };
        let path = attrs::apply_channel_attrs(&decl.attrs, &decl.name.text, self.diags);
        if let Some(first) = self.channel_paths.get(&path) {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::DuplicateDeclaration,
                    format!("channel path `{path}` is used more than once"),
                )
                .with_label(decl.span, "duplicate channel path here")
                .with_label(*first, "first channel path used here"),
            );
        } else {
            self.channel_paths.insert(path.clone(), decl.span);
        }
        let node = self.add_node(
            Component::Channel {
                name: decl.name.text.clone(),
                config: ChannelConfig { path },
            },
            Some(decl.span),
        );
        let Some(realtime) = self.default_capability(
            NodeKind::Realtime,
            &format!("channel `{}`", decl.name.text),
            decl.span,
        ) else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::MissingCapability,
                    format!("channel `{}` requires a realtime capability", decl.name.text),
                )
                .with_label(decl.span, "declared here")
                .with_help("add `realtime live WebSocket;` (or `realtime live SSE;`) to the `use { .. }` block"),
            );
            return;
        };
        self.graph.add_edge(stream, node, EdgeKind::AsyncMessage);
        self.graph.add_edge(node, realtime, EdgeKind::DependsOn);
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
            self.graph.nodes_of_kind(NodeKind::Cache).next().is_some(),
            self.diags,
        );
        let Some(db) = self.default_capability(
            NodeKind::Database,
            &format!("`crud {}`", decl.name.text),
            decl.span,
        ) else {
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
        let api = self.add_node(
            Component::Api {
                name: name.clone(),
                request: record,
                config: ApiConfig::default(),
            },
            Some(decl.span),
        );
        let service = self.add_node(
            Component::Service {
                name: format!("{name}Store"),
                signature: None,
            },
            Some(decl.span),
        );
        let auth = self.default_capability(
            NodeKind::Auth,
            &format!("`crud {}`", decl.name.text),
            decl.span,
        );
        match auth {
            Some(auth) => {
                self.graph.add_edge(api, auth, EdgeKind::RequestFlow);
                self.graph.add_edge(auth, service, EdgeKind::RequestFlow);
            }
            None => {
                self.graph.add_edge(api, service, EdgeKind::RequestFlow);
            }
        }
        self.graph.add_edge(service, db, EdgeKind::DataFlow);
        let cache = self.default_capability(
            NodeKind::Cache,
            &format!("`crud {}`", decl.name.text),
            decl.span,
        );
        if let Some(cache) = cache {
            self.graph.add_edge(service, cache, EdgeKind::DataFlow);
        }
        self.graph.resources.push(Resource {
            name,
            service_owner: self.current_service,
            api,
            service,
            database: db,
            cache,
            auth,
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
        let stream = self.add_stream(&name, None, subject, Some(queue), Some(decl.span));
        let worker = self.add_node(
            Component::Worker {
                name: format!("{name}Consumer"),
                config: Default::default(),
            },
            Some(decl.span),
        );
        self.graph.add_edge(stream, worker, EdgeKind::AsyncMessage);
        self.worker_streams.insert(worker, stream);
        if let Some(db) = self.default_capability(
            NodeKind::Database,
            &format!("`events {}`", decl.name.text),
            decl.span,
        ) {
            self.graph.add_edge(worker, db, EdgeKind::DataFlow);
        }
        self.graph.event_streams.push(EventStream {
            name,
            service_owner: self.current_service,
            stream,
            worker,
        });
    }

    fn pipeline(&mut self, decl: &PipelineDecl) {
        // The pipeline name must match a declared api, worker, or job.
        let owner = match self.current_service {
            Some(service) => self
                .graph
                .find_named_in_service(service, NodeKind::Api, &decl.name.text)
                .or_else(|| {
                    self.graph
                        .find_named_in_service(service, NodeKind::Worker, &decl.name.text)
                })
                .or_else(|| {
                    self.graph
                        .find_named_in_service(service, NodeKind::Job, &decl.name.text)
                })
                .map(|n| (n.id, n.component.clone())),
            None => self
                .graph
                .find_named(NodeKind::Api, &decl.name.text)
                .or_else(|| self.graph.find_named(NodeKind::Worker, &decl.name.text))
                .or_else(|| self.graph.find_named(NodeKind::Job, &decl.name.text))
                .map(|n| (n.id, n.component.clone())),
        };
        let Some((owner, owner_component)) = owner else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::UnknownPipelineTarget,
                    format!(
                        "pipeline `{}` does not match any declared api, worker, or job",
                        decl.name.text
                    ),
                )
                .with_label(decl.name.span, "no api, worker, or job with this name")
                .with_help(format!(
                    "declare `api {};`, `worker {};`, or `job {};`",
                    decl.name.text, decl.name.text, decl.name.text
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
            Component::Job { .. } => None,
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
            service: self.current_service,
            owner,
            payload,
            steps,
            span: Some(decl.span),
        });
    }

    /// Checks that publishing the pipeline payload onto `stream` is
    /// type-correct (`CIAC0016`). Untyped streams accept any payload.
    pub(crate) fn check_publish_type(
        &mut self,
        stream: NodeId,
        payload: Option<RecordId>,
        span: Span,
    ) {
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
                if let Some(queue) =
                    self.require_queue(&format!("publish `{}`", stream_name.text), stream_name.span)
                {
                    self.graph.add_edge(stream, queue, EdgeKind::DependsOn);
                }
                self.check_publish_type(stream, payload, stream_name.span);
                return Some(step(StepKind::Publish { stream }, expr.span()));
            }
            StepExpr::Call(target) => {
                return self.resolve_call(target, payload);
            }
            StepExpr::Match { field, arms, .. } => {
                return self.resolve_match(field, arms, payload, expr.span());
            }
            StepExpr::Name(ident) => ident,
        };
        match ident.text.as_str() {
            "Auth" => {
                match self.default_capability(NodeKind::Auth, "the `Auth` step", ident.span) {
                    Some(node) => Some(step(StepKind::Auth { node }, ident.span)),
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
                }
            }
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
                if let Some((kind, _)) = self.declared.get(&self.scoped_key(name)) {
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
                let handler_key = self.scoped_key(name);
                let node = match self.handlers.get(&handler_key) {
                    Some(id) => *id,
                    None => {
                        // v0.7 M2: a type-checked signature takes its
                        // capability dependencies from verb resolution,
                        // not declared bindings, so it skips
                        // `wire_handler_bindings` entirely — instead, it
                        // gets the same `DataFlow` edges classic bindings
                        // get, wired below from the HIR's own record of
                        // which capability instances it touched (there's
                        // no node to attach an edge to at type-check
                        // time, since the handler node is only created
                        // here, on first pipeline reference).
                        let signature = self.handler_signatures.get(&handler_key).cloned();
                        let id = self.add_node(
                            Component::Service {
                                name: name.to_owned(),
                                signature: signature.clone(),
                            },
                            Some(ident.span),
                        );
                        match &signature {
                            None => self.wire_handler_bindings(name, id, ident.span),
                            Some(hir) => {
                                for capability in hir.capability_nodes() {
                                    self.graph.add_edge(id, capability, EdgeKind::DataFlow);
                                }
                            }
                        }
                        self.handlers.insert(handler_key, id);
                        id
                    }
                };
                Some(step(StepKind::Handler { node }, ident.span))
            }
        }
    }

    fn resolve_call(
        &mut self,
        target: &ciac_syntax::ast::QualifiedIdent,
        payload: Option<RecordId>,
    ) -> Option<Step> {
        if target.segments.len() != 2 {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidCall,
                    "`call` targets must be written as `Service.Api`",
                )
                .with_label(target.span, "invalid call target"),
            );
            return None;
        }
        let service_name = &target.segments[0];
        let api_name = &target.segments[1];
        let Some((service, _)) = self.service_ids.get(&service_name.text).copied() else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::UnknownService,
                    format!("unknown service `{}`", service_name.text),
                )
                .with_label(service_name.span, "no service with this name"),
            );
            return None;
        };
        let Some(api) = self
            .graph
            .find_named_in_service(service, NodeKind::Api, &api_name.text)
            .map(|node| (node.id, node.component.clone()))
        else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::UnknownServiceMember,
                    format!(
                        "service `{}` has no api `{}`",
                        service_name.text, api_name.text
                    ),
                )
                .with_label(api_name.span, "unknown api in target service"),
            );
            return None;
        };
        let Component::Api { request, .. } = api.1 else {
            unreachable!("find_named_in_service requested an api");
        };
        if request != payload {
            let expected = request
                .map(|id| format!("`{}`", self.graph.record(id).name))
                .unwrap_or_else(|| "untyped JSON".to_owned());
            let found = payload
                .map(|id| format!("`{}`", self.graph.record(id).name))
                .unwrap_or_else(|| "untyped JSON".to_owned());
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::CrossServiceTypeMismatch,
                    format!(
                        "call to `{}.{}` expects {expected} but caller payload is {found}",
                        service_name.text, api_name.text
                    ),
                )
                .with_label(target.span, "payload type mismatch at call site"),
            );
        }
        Some(step(StepKind::Call { target: api.0 }, target.span))
    }

    fn wire_handler_bindings(&mut self, name: &str, handler: NodeId, span: Span) {
        if let Some(bindings) = self.handler_bindings.get(&self.scoped_key(name)).cloned() {
            for binding in bindings {
                if let Some(target) =
                    self.resolve_capability(binding.kind, &binding.instance, binding.span)
                {
                    self.graph.add_edge(handler, target, EdgeKind::DataFlow);
                }
            }
            return;
        }

        for kind in [NodeKind::Database, NodeKind::Cache] {
            if self.graph.nodes_of_kind(kind).next().is_some() {
                if let Some(target) =
                    self.default_capability(kind, &format!("handler `{name}`"), span)
                {
                    self.graph.add_edge(handler, target, EdgeKind::DataFlow);
                }
            }
        }
    }

    /// Validates match arm labels against `variants` — duplicate arms, a
    /// wildcard (`_`) out of trailing position, unknown variants, and
    /// (unless a wildcard covers the rest) missing variants
    /// (`NonExhaustiveMatch`, CIAC0021). Returns one resolved label per
    /// arm in input order (`None` = wildcard) for the caller to zip with
    /// its own arm bodies. Shared by pipeline-level `match` steps
    /// (`resolve_match`, above) and (v0.7) expression-level `match`
    /// (`typeck.rs`) — 07UpdatePlan.md calls for exactly this reuse.
    pub(crate) fn check_match_labels(
        &mut self,
        variants: &[String],
        labels: &[&ciac_syntax::ast::ArmLabel],
        scrutinee_desc: &str,
        match_span: Span,
    ) -> Vec<Option<String>> {
        let mut seen: BTreeMap<String, Span> = BTreeMap::new();
        let mut saw_wildcard = false;
        let mut resolved = Vec::with_capacity(labels.len());
        for (idx, label) in labels.iter().enumerate() {
            resolved.push(match label {
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
                    if idx + 1 != labels.len() {
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
                                format!(
                                    "`{}` is not a variant of `{}`",
                                    ident.text, scrutinee_desc
                                ),
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
            });
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
                            scrutinee_desc,
                            missing.join(", ")
                        ),
                    )
                    .with_label(match_span, "non-exhaustive match"),
                );
            }
        }
        resolved
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
        let labels: Vec<&ciac_syntax::ast::ArmLabel> = arms.iter().map(|arm| &arm.label).collect();
        let resolved_labels = self.check_match_labels(&variants, &labels, &field.text, span);
        let mut ir_arms = Vec::new();
        for (arm, label) in arms.iter().zip(resolved_labels) {
            let steps = arm
                .steps
                .iter()
                .filter_map(|step| self.resolve_step(step, Some(payload)))
                .collect();
            ir_arms.push(MatchArm { label, steps });
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
                StepKind::Call { target } => {
                    self.graph.add_edge(prev, *target, EdgeKind::ServiceCall);
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
        NodeKind::Job => "job",
        NodeKind::Channel => "channel",
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

pub(crate) fn binding_kind(capability: &str) -> Option<NodeKind> {
    Some(match capability {
        "auth" => NodeKind::Auth,
        "db" => NodeKind::Database,
        "cache" => NodeKind::Cache,
        "queue" => NodeKind::Queue,
        "object_store" => NodeKind::ObjectStore,
        "email" => NodeKind::Email,
        "search" => NodeKind::Search,
        "external_http" => NodeKind::ExternalHttp,
        "scheduler" => NodeKind::Scheduler,
        "realtime" => NodeKind::Realtime,
        "logging" => NodeKind::Logging,
        "metrics" => NodeKind::Metrics,
        _ => return None,
    })
}

fn attr_string(attrs: &[Attr], name: &str) -> Option<String> {
    attrs.iter().find_map(|attr| {
        (attr.name.text == name).then(|| match &attr.value {
            AttrValue::Str { value, .. } => Some(value.clone()),
            AttrValue::Ident(ident) => Some(ident.text.clone()),
            AttrValue::Number { .. } => None,
        })?
    })
}

fn item_span(item: &Item) -> Span {
    match item {
        Item::Import(decl) => decl.span,
        Item::Blueprint(decl) => decl.span,
        Item::Expand(decl) => decl.span,
        Item::Project(decl) => decl.span,
        Item::Service(decl) => decl.span,
        Item::ServiceBlock(decl) => decl.span,
        Item::Use(decl) => decl.span,
        Item::Record(decl) => decl.span,
        Item::Stream(decl) => decl.span,
        Item::Table(decl) => decl.span,
        Item::Api(decl) => decl.span,
        Item::Worker(decl) => decl.span,
        Item::Job(decl) => decl.span,
        Item::Channel(decl) => decl.span,
        Item::Crud(decl) => decl.span,
        Item::Events(decl) => decl.span,
        Item::Handler(decl) => decl.span,
        Item::Pipeline(decl) => decl.span,
    }
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

    pub fn apply_job_attrs(attrs: &[Attr], diags: &mut Diagnostics) -> Option<JobConfig> {
        let mut schedule = None;
        let mut catch_up = false;
        let mut seen = BTreeMap::new();
        for attr in attrs {
            if duplicate_attr(attr, &mut seen, diags) {
                continue;
            }
            match attr.name.text.as_str() {
                "schedule" => match string_value(attr) {
                    Some(value) if valid_cron(value) => schedule = Some(value.to_owned()),
                    Some(_) => diags.push(
                        Diagnostic::new(
                            ErrorCode::InvalidCron,
                            "job `schedule` must be a valid five-field cron expression",
                        )
                        .with_label(attr.value.span(), "invalid cron expression"),
                    ),
                    None => invalid_value(attr, "job `schedule` must be a string", diags),
                },
                "catch_up" => match bool_value(attr) {
                    Some(value) => catch_up = value,
                    None => invalid_value(attr, "job `catch_up` must be `true` or `false`", diags),
                },
                _ => unknown_attr(attr, "job", diags),
            }
        }
        match schedule {
            Some(schedule) => Some(JobConfig { schedule, catch_up }),
            None => {
                diags.push(
                    Diagnostic::new(
                        ErrorCode::InvalidAttributeValue,
                        "job requires a `schedule` attribute",
                    )
                    .with_help("write `job Name { schedule: \"0 3 * * *\"; }`"),
                );
                None
            }
        }
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

    pub fn apply_channel_attrs(
        attrs: &[Attr],
        channel_name: &str,
        diags: &mut Diagnostics,
    ) -> String {
        let mut path = format!("/channels/{}", channel_name.to_kebab_case());
        let mut seen = BTreeMap::new();
        for attr in attrs {
            if duplicate_attr(attr, &mut seen, diags) {
                continue;
            }
            match attr.name.text.as_str() {
                "path" => match string_value(attr) {
                    Some(value) if value.starts_with('/') => path = value.to_owned(),
                    Some(_) | None => invalid_value(
                        attr,
                        "channel `path` must be a string starting with `/`",
                        diags,
                    ),
                },
                _ => unknown_attr(attr, "channel", diags),
            }
        }
        path
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

    fn bool_value(attr: &Attr) -> Option<bool> {
        match &attr.value {
            AttrValue::Ident(ident) if ident.text == "true" => Some(true),
            AttrValue::Ident(ident) if ident.text == "false" => Some(false),
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

    fn valid_cron(expr: &str) -> bool {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return false;
        }
        fields
            .iter()
            .enumerate()
            .all(|(idx, field)| valid_cron_field(field, idx))
    }

    fn valid_cron_field(field: &str, idx: usize) -> bool {
        let (min, max) = match idx {
            0 => (0, 59),
            1 => (0, 23),
            2 => (1, 31),
            3 => (1, 12),
            4 => (0, 7),
            _ => return false,
        };
        field.split(',').all(|part| valid_cron_part(part, min, max))
    }

    fn valid_cron_part(part: &str, min: u32, max: u32) -> bool {
        let base = match part.split_once('/') {
            Some((base, step)) => {
                let Ok(step) = step.parse::<u32>() else {
                    return false;
                };
                if step == 0 {
                    return false;
                }
                base
            }
            None => part,
        };
        let base_valid = if base == "*" {
            true
        } else if let Some((start, end)) = base.split_once('-') {
            match (start.parse::<u32>(), end.parse::<u32>()) {
                (Ok(start), Ok(end)) => min <= start && start <= end && end <= max,
                _ => false,
            }
        } else {
            match base.parse::<u32>() {
                Ok(value) => min <= value && value <= max,
                Err(_) => false,
            }
        };
        base_valid
    }
}
