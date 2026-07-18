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

use ciac_diagnostics::{Diagnostic, Diagnostics, Edit, ErrorCode, Fix, Span};
use ciac_ir::{
    ApiConfig, AuthScheme, CacheEngine, Cardinality, ChannelConfig, Component, CrudConfig,
    DbEngine, EdgeKind, EmailProvider, EventStream, FieldType, HandlerBody, HttpMethod, JobConfig,
    LoggingProvider, MatchArm, MetricsProvider, NodeId, NodeKind, ObjectStoreProvider, Pipeline,
    QueueEngine, RealtimeProvider, Record, RecordField, RecordId, RecordKind as IrRecordKind,
    Resource, SchedulerProvider, SearchProvider, ServiceId, Step, StepKind, SystemGraph, Table,
    TableId, TracingProvider, UsersProvider,
};
use ciac_syntax::ast::{
    ApiDecl, Attr, AttrValue, ChannelDecl, ComponentDecl, CrudDecl, HandlerDecl, Ident, Item,
    JobDecl, PipelineDecl, Program, RecordDecl, RecordKind, ServiceBlock, ServiceItem, StepExpr,
    StreamDecl, TableDecl, TypeExpr, UseBlock, UseEntry, WorkerDecl,
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
    /// Record → the (first) table backing it (v0.16 M2) — a
    /// `Reference<T>` field's source record must be backed by typed
    /// storage (`RefRequiresTypedStorage` otherwise), and its owning
    /// table's service/database instance is what `CrossStorageReference`
    /// compares against the target's.
    record_tables: HashMap<RecordId, TableId>,
    /// Every `Reference<T>` field seen during record registration,
    /// deferred until every record and table in the program is
    /// registered (v0.16 M2) — see `resolve_references`.
    pending_references: Vec<PendingReference>,
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
    /// True while processing entries of a `use { .. }` block that also
    /// declares `users Keycloak` (v0.15 M6) — lets the `auth OAuth2`
    /// arm of [`Self::use_entry`] accept a missing `issuer` attribute,
    /// since it defaults to the dev Keycloak container's URL instead.
    current_block_has_users: bool,
    /// Per-service insertion point for "add a capability line to this
    /// service's `use { .. }` block" fixes (v0.15 M7): a zero-width
    /// span right before the first entry (or right before the closing
    /// `}` when the block is empty). Absent when the service has no
    /// `use { .. }` block at all -- `missing_capability_fix` then
    /// offers no fix rather than synthesizing a new block, a disclosed
    /// scope cut (see its doc comment).
    use_block_insertion: HashMap<ServiceId, Span>,
}

#[derive(Debug, Clone)]
struct BindingSpec {
    kind: NodeKind,
    instance: String,
    span: Span,
}

/// A `Reference<T>` field seen during record registration, deferred
/// until `resolve_references` runs (v0.16 M2) — see
/// `Builder::pending_references`.
#[derive(Debug, Clone)]
struct PendingReference {
    record: RecordId,
    field_index: usize,
    target: Ident,
    attrs: Vec<Attr>,
    field_span: Span,
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
            record_tables: HashMap::new(),
            pending_references: Vec::new(),
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
            current_block_has_users: false,
            use_block_insertion: HashMap::new(),
        }
    }

    fn build(mut self, program: &Program) -> SystemGraph {
        let has_service_blocks = program
            .items
            .iter()
            .any(|item| matches!(item, Item::ServiceBlock(_)));
        self.project_name(program, has_service_blocks);
        // Records first: streams, tables, and `Reference<T>` fields all
        // name them. Tables resolve after records but before `use`
        // blocks/service registration is done per branch below (v0.16
        // M2: a table's owning database instance must be resolved
        // against capability instances that only exist once `use`
        // entries for that service have been processed).
        for item in &program.items {
            if let Item::Record(decl) = item {
                self.record(decl);
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
                    // v0.16 M2: a top-level table doesn't identify a
                    // service/database ownership boundary in a
                    // multi-service project — it must be declared inside
                    // the owning `service { .. }` block.
                    Item::Table(_) => self.diags.push(
                        Diagnostic::new(
                            ErrorCode::InvalidServiceScope,
                            "in a multi-service project, `table` must be declared inside its \
                             owning `service { .. }` block",
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
                    | Item::Stream(_) => {}
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
                    self.current_block_has_users =
                        block.entries.iter().any(|e| e.capability.text == "users");
                    if let Some(sid) = self.current_service {
                        self.use_block_insertion
                            .insert(sid, use_block_insertion_point(block));
                    }
                    for entry in &block.entries {
                        self.use_entry(entry);
                    }
                    self.current_block_has_users = false;
                }
            }
            for item in &program.items {
                if let Item::Stream(decl) = item {
                    self.stream(decl, true);
                }
            }
            // v0.16 M2: after `use` (capability instances now exist) so
            // `{ db: <instance>; }` — or the implicit default — resolves;
            // before `build_flat_items`, since `Reference<T>` resolution
            // (deferred to `resolve_references` below) needs every table.
            for item in &program.items {
                if let Item::Table(decl) = item {
                    self.table(decl);
                }
            }
            self.build_flat_items(&program.items);
        }
        self.check_scoped_apis();
        self.check_scoped_cruds();
        self.resolve_references();
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
                self.current_block_has_users =
                    block.entries.iter().any(|e| e.capability.text == "users");
                if let Some(sid) = self.current_service {
                    self.use_block_insertion
                        .insert(sid, use_block_insertion_point(block));
                }
                for entry in &block.entries {
                    self.use_entry(entry);
                }
                self.current_block_has_users = false;
            }
        }
        // v0.16 M2: after `use` (capability instances exist to resolve
        // `{ db: <instance>; }` — or the implicit default — against) and
        // before handlers/api/etc, which don't depend on table order.
        for item in items {
            if let ServiceItem::Table(decl) = item {
                self.table(decl);
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
                ServiceItem::Use(_)
                | ServiceItem::Handler(_)
                | ServiceItem::Pipeline(_)
                | ServiceItem::Table(_) => {}
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
        // v0.16 M2: a `Reference<T>` field's real type isn't known until
        // every record and table in the program is registered (the
        // target might be declared later in the file, and its owning
        // table's service/database instance must exist). A placeholder
        // `Json` field holds the slot — same position, same name — so
        // field order is preserved; `resolve_references` overwrites
        // `fields[field_index].ty` once resolution runs.
        let mut deferred: Vec<(usize, Ident, Vec<Attr>, Span)> = Vec::new();
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
                TypeExpr::List { span, .. } => {
                    self.diags.push(
                        Diagnostic::new(
                            ErrorCode::UnsupportedFieldType,
                            "list types aren't supported as record field types yet",
                        )
                        .with_label(*span, "not valid here")
                        .with_help(
                            "`[Type]` is valid as a handler parameter/return type, or as the \
                             result of `db.query`/`object_store.list`/`search.query`",
                        ),
                    );
                    continue;
                }
                TypeExpr::Reference { target, span } => {
                    deferred.push((fields.len(), target.clone(), field.attrs.clone(), *span));
                    FieldType::Json
                }
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
        for (field_index, target, attrs, field_span) in deferred {
            self.pending_references.push(PendingReference {
                record: id,
                field_index,
                target,
                attrs,
                field_span,
            });
        }
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
        // v0.16 M2: resolve the table's owning database instance — named
        // (`{ db: primary; }`) or the service's unambiguous default,
        // exactly like any other unqualified capability use. `None` only
        // when the service has no `db` capability at all; existing
        // programs with exactly one instance are unaffected.
        let db_instance = match &decl.db {
            Some(name) => self.resolve_capability(NodeKind::Database, &name.text, name.span),
            None => self.default_capability(
                NodeKind::Database,
                &format!("table `{}`", decl.name.text),
                decl.span,
            ),
        };
        let id = self.graph.add_table(Table {
            name: decl.name.text.clone(),
            record,
            service: self.current_service,
            db_instance,
        });
        self.tables.insert(decl.name.text.clone(), (id, decl.span));
        self.record_tables.entry(record).or_insert(id);
    }

    /// Resolves every `Reference<T>` field deferred by `record()` (v0.16
    /// M2). Runs once, after every record and table in the *entire*
    /// program has been registered — a reference may target a record or
    /// table declared later in the file, and cross-service/instance
    /// checks need every table's ownership to already be known.
    fn resolve_references(&mut self) {
        let pending = std::mem::take(&mut self.pending_references);
        let mut one_edges: Vec<(RecordId, RecordId, Span)> = Vec::new();
        for pending_ref in &pending {
            if let Some((target, cardinality)) = self.resolve_one_reference(pending_ref) {
                if cardinality == Cardinality::One {
                    one_edges.push((pending_ref.record, target, pending_ref.field_span));
                }
            }
        }
        find_reference_cycles(&one_edges, self.diags);
    }

    /// Resolves one deferred `Reference<T>` field. On success, patches
    /// the record's placeholder `Json` field into the real
    /// `FieldType::Reference` and returns `(target, cardinality)` so the
    /// caller can fold it into the cycle-detection edge set. Every
    /// failure path pushes exactly one diagnostic and leaves the
    /// placeholder in place (the field's static type never becomes
    /// visibly wrong; the build simply also reports the error).
    fn resolve_one_reference(
        &mut self,
        pending: &PendingReference,
    ) -> Option<(RecordId, Cardinality)> {
        let Some((target_id, _)) = self.records.get(&pending.target.text).copied() else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::UnknownReferenceTarget,
                    format!("unknown record `{}`", pending.target.text),
                )
                .with_label(pending.target.span, "not a declared record")
                .with_help(format!(
                    "declare it with `record {} {{ .. }}`",
                    pending.target.text
                )),
            );
            return None;
        };

        let parsed = attrs::apply_reference_attrs(&pending.attrs, self.diags);

        let Some(references) = &parsed.references else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidReferenceDefinition,
                    "`Reference<T>` field is missing a `references` attribute",
                )
                .with_label(pending.field_span, "add `references: <Table>;`"),
            );
            return None;
        };
        let Some((table_id, _)) = self.tables.get(&references.text).copied() else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::UnknownReferenceTarget,
                    format!("unknown table `{}`", references.text),
                )
                .with_label(references.span, "not a declared table"),
            );
            return None;
        };
        if self.graph.table(table_id).record != target_id {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::UnknownReferenceTarget,
                    format!(
                        "table `{}` is not backed by record `{}`",
                        references.text, pending.target.text
                    ),
                )
                .with_label(references.span, "backed by a different record"),
            );
            return None;
        }

        let mut missing = Vec::new();
        if parsed.cardinality.is_none() {
            missing.push("cardinality");
        }
        if parsed.on_delete.is_none() {
            missing.push("on_delete");
        }
        if parsed.on_update.is_none() {
            missing.push("on_update");
        }
        if !missing.is_empty() {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidReferenceDefinition,
                    format!(
                        "`Reference<T>` field is missing required attribute(s): {}",
                        missing.join(", ")
                    ),
                )
                .with_label(
                    pending.field_span,
                    "every reference states cardinality and both actions explicitly",
                ),
            );
            return None;
        }
        let cardinality = parsed.cardinality.unwrap();
        let on_delete = parsed.on_delete.unwrap();
        let on_update = parsed.on_update.unwrap();

        if parsed.unique && cardinality == Cardinality::Many {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidReferenceDefinition,
                    "`unique: true` is only valid on a `cardinality: one` reference",
                )
                .with_label(pending.field_span, "not valid on a `many` reference"),
            );
            return None;
        }

        let target_has_id = self
            .graph
            .record(target_id)
            .fields
            .iter()
            .any(|f| f.name == "id" && f.ty == FieldType::Uuid);
        if !target_has_id {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::InvalidReferenceDefinition,
                    format!(
                        "record `{}` has no `id: Uuid` field, which the stores are built on",
                        pending.target.text
                    ),
                )
                .with_label(pending.field_span, "target lacks an `id: Uuid` field"),
            );
            return None;
        }

        let Some(&source_table_id) = self.record_tables.get(&pending.record) else {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::RefRequiresTypedStorage,
                    "a record with a `Reference<T>` field must be backed by an explicit `table`",
                )
                .with_label(pending.field_span, "no `table` backs this record")
                .with_help(
                    "declare `table <Name>: <Record>;` for the record this field belongs to",
                ),
            );
            return None;
        };
        let source_table = self.graph.table(source_table_id);
        let target_table = self.graph.table(table_id);
        if source_table.service != target_table.service
            || source_table.db_instance != target_table.db_instance
        {
            self.diags.push(
                Diagnostic::new(
                    ErrorCode::CrossStorageReference,
                    format!(
                        "`{}` and `{}` belong to different services or database instances",
                        source_table.name, target_table.name
                    ),
                )
                .with_label(
                    pending.field_span,
                    "cannot reference across services/instances",
                ),
            );
            return None;
        }

        self.graph.record_mut(pending.record).fields[pending.field_index].ty =
            FieldType::Reference {
                target: target_id,
                table: table_id,
                cardinality,
                on_delete,
                on_update,
                unique: parsed.unique,
            };
        Some((target_id, cardinality))
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

    /// The "add `line` to the current service's `use { .. }` block"
    /// fix for a missing-capability diagnostic (v0.15 M7). `None` when
    /// this service has no `use { .. }` block to insert into at all --
    /// synthesizing a brand-new block is out of scope (a disclosed cut,
    /// same spirit as the plan's "fixes that can't meet the bar ship as
    /// prose `help`, not a fix").
    fn missing_capability_fix(&self, line: &str) -> Option<Fix> {
        let at = *self.use_block_insertion.get(&self.current_service?)?;
        Some(Fix {
            title: format!("Add `{line}` to the use block"),
            edits: vec![Edit {
                span: at,
                replacement: format!("{line}\n    "),
            }],
        })
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
                let issuer = attr_string(&entry.attrs, "issuer");
                if issuer.is_none() && !self.current_block_has_users {
                    let mut diag = Diagnostic::new(
                        ErrorCode::UnsupportedProviderConfig,
                        "auth OAuth2 requires an `issuer` string attribute",
                    )
                    .with_label(entry.span, "missing `issuer`")
                    .with_help(
                        "add `issuer: \"...\"`, or declare `users Keycloak;` in the \
                         same `use { .. }` block to default to its dev issuer",
                    );
                    // v0.15 M7: only the bare `auth OAuth2;` shape (no
                    // `{ .. }` block at all yet) gets a fix -- rewriting
                    // an existing `{ .. }` block (e.g. one that already
                    // sets `audience`) would need to preserve every
                    // other attribute, which the entry's span alone
                    // doesn't give us a safe way to splice around.
                    if entry.attrs.is_empty() {
                        diag = diag.with_fix(Fix {
                            title: "Add a placeholder `issuer`".to_owned(),
                            edits: vec![Edit {
                                span: entry.span,
                                replacement: format!(
                                    "auth {} {{ issuer: \"http://localhost:8080\"; }}",
                                    provider.unwrap_or("OAuth2")
                                ),
                            }],
                        });
                    }
                    self.diags.push(diag);
                    return;
                }
                Component::Auth {
                    name: name.to_owned(),
                    scheme: AuthScheme::OAuth2,
                    // `None` here means "defaulted from `users Keycloak`"
                    // (v0.15 M6) -- `ciac-codegen::model` resolves the
                    // dev issuer URL, not sema, since the URL is a
                    // deployment-shape fact, not a language fact.
                    issuer,
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
            ("tracing", Some("OpenTelemetry")) => Component::Tracing {
                name: name.to_owned(),
                provider: TracingProvider::OpenTelemetry,
            },
            ("users", Some("Keycloak")) => Component::Users {
                name: name.to_owned(),
                provider: UsersProvider::Keycloak,
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
                let provider_str = provider.unwrap_or("<none>");
                let mut diag = Diagnostic::new(
                    ErrorCode::UnknownProvider,
                    format!("unknown capability `{capability} {provider_str}`"),
                )
                .with_label(entry.span, "not a supported capability/provider pair")
                .with_help(ErrorCode::UnknownProvider.explanation());
                if let Some(provider_ident) = &entry.provider {
                    if let Some((sc, sp)) = nearest_capability_provider(capability, provider_str) {
                        diag = diag.with_fix(Fix {
                            title: format!("Rename to `{sc} {sp}`"),
                            edits: vec![Edit {
                                span: entry.capability.span.to(provider_ident.span),
                                replacement: format!("{sc} {sp}"),
                            }],
                        });
                    }
                }
                self.diags.push(diag);
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
                let mut diag = Diagnostic::new(
                    ErrorCode::MissingCapability,
                    format!("{what} requires a queue capability"),
                )
                .with_label(span, "used here")
                .with_help("add `queue NATS;` (or `queue Kafka;`) to the `use { .. }` block");
                if let Some(fix) = self.missing_capability_fix("queue NATS;") {
                    diag = diag.with_fix(fix);
                }
                self.diags.push(diag);
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
            let mut diag = Diagnostic::new(
                ErrorCode::MissingCapability,
                format!("job `{}` requires a scheduler capability", decl.name.text),
            )
            .with_label(decl.span, "declared here")
            .with_help("add `scheduler Cron;` to the `use { .. }` block");
            if let Some(fix) = self.missing_capability_fix("scheduler Cron;") {
                diag = diag.with_fix(fix);
            }
            self.diags.push(diag);
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
            let mut diag = Diagnostic::new(
                ErrorCode::MissingCapability,
                format!("channel `{}` requires a realtime capability", decl.name.text),
            )
            .with_label(decl.span, "declared here")
            .with_help("add `realtime live WebSocket;` (or `realtime live SSE;`) to the `use { .. }` block");
            if let Some(fix) = self.missing_capability_fix("realtime WebSocket;") {
                diag = diag.with_fix(fix);
            }
            self.diags.push(diag);
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
            let mut diag = Diagnostic::new(
                ErrorCode::MissingCapability,
                format!("`crud {}` requires a database", decl.name.text),
            )
            .with_label(decl.span, "declared here")
            .with_help("add `db Postgres;` to the `use { .. }` block");
            if let Some(fix) = self.missing_capability_fix("db Postgres;") {
                diag = diag.with_fix(fix);
            }
            self.diags.push(diag);
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
                        let mut diag = Diagnostic::new(
                            ErrorCode::MissingCapability,
                            "the `Auth` step requires an auth capability",
                        )
                        .with_label(ident.span, "used here")
                        .with_help("add `auth JWT;` to the `use { .. }` block");
                        if let Some(fix) = self.missing_capability_fix("auth JWT;") {
                            diag = diag.with_fix(fix);
                        }
                        self.diags.push(diag);
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

    /// `crud`'s per-verb `read_scope`/`write_scope` attrs (v0.14 M6)
    /// require *some* auth capability to actually enforce against —
    /// unlike a plain `api`, a `crud` resource has no pipeline of its
    /// own for `check_scoped_apis` to inspect, so this is a separate
    /// check over `graph.resources` rather than an extension of that
    /// one. Same "declared but silently unenforced" failure mode the
    /// plain-`api` check exists to kill.
    fn check_scoped_cruds(&mut self) {
        for resource in &self.graph.resources {
            if resource.auth.is_some() {
                continue;
            }
            for (attr_name, scope) in [
                ("read_scope", &resource.config.read_scope),
                ("write_scope", &resource.config.write_scope),
            ] {
                let Some(scope) = scope else { continue };
                let mut diag = Diagnostic::new(
                    ErrorCode::InvalidAttributeValue,
                    format!(
                        "crud `{}` declares `{attr_name}: \"{scope}\"` but the service has no auth capability",
                        resource.name
                    ),
                )
                .with_help("add `auth JWT;` (or `auth OAuth2;`) to the `use { .. }` block");
                if let Some(span) = self.graph.node(resource.api).span {
                    diag = diag.with_label(span, "scoped crud declared here");
                }
                self.diags.push(diag);
            }
        }
    }
}

/// The zero-width span a "add a capability line here" fix (v0.15 M7)
/// can insert text at: right before the first entry (so the new line
/// inherits its indentation), or -- for an empty `use {}` -- right
/// before the closing brace.
fn use_block_insertion_point(block: &UseBlock) -> Span {
    match block.entries.first() {
        Some(first) => Span::new(first.span.file, first.span.start, first.span.start),
        None => {
            let at = block.span.end.saturating_sub(1);
            Span::new(block.span.file, at, at)
        }
    }
}

/// Every `capability Provider` pair the closed registry accepts
/// (`use_entry`'s big match, above) -- the candidate set a v0.15 M7
/// "did you mean" rename fix searches for the nearest match in.
const KNOWN_CAPABILITY_PROVIDERS: &[(&str, &str)] = &[
    ("auth", "JWT"),
    ("auth", "OAuth2"),
    ("db", "Postgres"),
    ("db", "MySQL"),
    ("db", "SQLite"),
    ("cache", "Redis"),
    ("queue", "NATS"),
    ("queue", "Kafka"),
    ("logging", "Structured"),
    ("metrics", "Prometheus"),
    ("tracing", "OpenTelemetry"),
    ("users", "Keycloak"),
    ("object_store", "S3"),
    ("email", "SES"),
    ("email", "SMTP"),
    ("search", "OpenSearch"),
    ("scheduler", "Cron"),
    ("realtime", "WebSocket"),
    ("realtime", "SSE"),
];

/// The closest known `(capability, provider)` pair to `(capability,
/// provider)`, by Levenshtein distance over the lowercased `"cap
/// provider"` string -- `None` when nothing is close enough to be a
/// plausible typo rather than a coincidence.
fn nearest_capability_provider(
    capability: &str,
    provider: &str,
) -> Option<(&'static str, &'static str)> {
    let query = format!("{capability} {provider}").to_lowercase();
    KNOWN_CAPABILITY_PROVIDERS
        .iter()
        .map(|&(c, p)| {
            (
                c,
                p,
                levenshtein(&query, &format!("{c} {p}").to_lowercase()),
            )
        })
        .min_by_key(|&(_, _, dist)| dist)
        .and_then(|(c, p, dist)| (dist <= 5).then_some((c, p)))
}

/// Classic O(n*m) edit-distance, one row at a time -- these strings
/// are always short (capability/provider names, record field names),
/// so no need for anything smarter. `pub(crate)` for reuse by
/// `typeck.rs`'s unknown-field "did you mean" fix (v0.15 M7).
pub(crate) fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut curr = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        curr[0] = i;
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b.len()]
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
        "tracing" => NodeKind::Tracing,
        "users" => NodeKind::Users,
        _ => return None,
    })
}

fn attr_string(attrs: &[Attr], name: &str) -> Option<String> {
    attrs.iter().find_map(|attr| {
        (attr.name.text == name).then(|| match &attr.value {
            AttrValue::Str { value, .. } => Some(value.clone()),
            AttrValue::Ident(ident) => Some(ident.text.clone()),
            AttrValue::Number { .. } | AttrValue::Float { .. } => None,
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

/// Detects cycles in the directed graph of `cardinality: one`
/// `Reference<T>` edges (v0.16 M2) — a `many` reference creates a
/// separate link table and never requires the target to exist first, so
/// it never participates in a cycle. A required-reference cycle (direct
/// self-reference included) is uninsertable: no row in the cycle could
/// ever be written first. Colored DFS; reports the back-edge that closes
/// each cycle found, at that field's own span.
fn find_reference_cycles(edges: &[(RecordId, RecordId, Span)], diags: &mut Diagnostics) {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    fn visit(
        node: RecordId,
        adj: &HashMap<RecordId, Vec<(RecordId, Span)>>,
        color: &mut HashMap<RecordId, Color>,
        diags: &mut Diagnostics,
    ) {
        color.insert(node, Color::Gray);
        if let Some(targets) = adj.get(&node) {
            for &(next, span) in targets {
                match color.get(&next).copied().unwrap_or(Color::White) {
                    Color::White => visit(next, adj, color, diags),
                    Color::Gray => {
                        diags.push(
                            Diagnostic::new(
                                ErrorCode::CyclicReferenceGraph,
                                "this reference is part of a cycle of required references",
                            )
                            .with_label(span, "closes the cycle here"),
                        );
                    }
                    Color::Black => {}
                }
            }
        }
        color.insert(node, Color::Black);
    }

    let mut adj: HashMap<RecordId, Vec<(RecordId, Span)>> = HashMap::new();
    for &(from, to, span) in edges {
        adj.entry(from).or_default().push((to, span));
    }
    let mut color: HashMap<RecordId, Color> = HashMap::new();
    let nodes: Vec<RecordId> = adj.keys().copied().collect();
    for start in nodes {
        if color.get(&start).copied().unwrap_or(Color::White) == Color::White {
            visit(start, &adj, &mut color, diags);
        }
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
                "read_scope" => match string_value(attr) {
                    Some(scope) => config.read_scope = Some(scope.to_owned()),
                    None => invalid_value(attr, "crud `read_scope` must be a string", diags),
                },
                "write_scope" => match string_value(attr) {
                    Some(scope) => config.write_scope = Some(scope.to_owned()),
                    None => invalid_value(attr, "crud `write_scope` must be a string", diags),
                },
                _ => unknown_attr(attr, "crud", diags),
            }
        }
        config
    }

    /// Parsed `Reference<T>` field attributes (v0.16 M2). All four of
    /// `references`/`cardinality`/`on_delete`/`on_update` are `None`
    /// when missing or invalid — the caller (`Builder::resolve_one_reference`)
    /// reports `InvalidReferenceDefinition` naming exactly which are
    /// absent, rather than this function guessing a default.
    #[derive(Default)]
    pub struct ReferenceAttrs {
        pub references: Option<Ident>,
        pub cardinality: Option<ciac_ir::Cardinality>,
        pub on_delete: Option<ciac_ir::RefAction>,
        pub on_update: Option<ciac_ir::RefAction>,
        pub unique: bool,
    }

    pub fn apply_reference_attrs(attrs: &[Attr], diags: &mut Diagnostics) -> ReferenceAttrs {
        let mut result = ReferenceAttrs::default();
        let mut seen = BTreeMap::new();
        for attr in attrs {
            if duplicate_attr(attr, &mut seen, diags) {
                continue;
            }
            match attr.name.text.as_str() {
                "references" => match &attr.value {
                    AttrValue::Ident(ident) => result.references = Some(ident.clone()),
                    _ => invalid_ref_value(attr, "`references` must name a declared table", diags),
                },
                "cardinality" => match ident_value(attr) {
                    Some("one") => result.cardinality = Some(ciac_ir::Cardinality::One),
                    Some("many") => result.cardinality = Some(ciac_ir::Cardinality::Many),
                    _ => invalid_ref_value(attr, "`cardinality` must be `one` or `many`", diags),
                },
                "on_delete" => match ident_value(attr) {
                    Some("restrict") => result.on_delete = Some(ciac_ir::RefAction::Restrict),
                    Some("cascade") => result.on_delete = Some(ciac_ir::RefAction::Cascade),
                    _ => invalid_ref_value(
                        attr,
                        "`on_delete` must be `restrict` or `cascade`",
                        diags,
                    ),
                },
                "on_update" => match ident_value(attr) {
                    Some("restrict") => result.on_update = Some(ciac_ir::RefAction::Restrict),
                    Some("cascade") => result.on_update = Some(ciac_ir::RefAction::Cascade),
                    _ => invalid_ref_value(
                        attr,
                        "`on_update` must be `restrict` or `cascade`",
                        diags,
                    ),
                },
                "unique" => match bool_value(attr) {
                    Some(value) => result.unique = value,
                    None => invalid_ref_value(attr, "`unique` must be `true` or `false`", diags),
                },
                // Storage-attribute surface shared with scalar fields
                // (Pillar 2, v0.16 M4): recognized here so a reference
                // field carrying it doesn't spuriously fail as an
                // "unknown attribute" before that milestone lands.
                "index" => {
                    if bool_value(attr).is_none() {
                        invalid_ref_value(attr, "`index` must be `true` or `false`", diags);
                    }
                }
                _ => unknown_attr(attr, "reference", diags),
            }
        }
        result
    }

    fn invalid_ref_value(attr: &Attr, message: &str, diags: &mut Diagnostics) {
        diags.push(
            Diagnostic::new(ErrorCode::InvalidReferenceDefinition, message)
                .with_label(attr.value.span(), "invalid value"),
        );
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

#[cfg(test)]
mod fix_tests {
    use super::*;

    #[test]
    fn levenshtein_is_zero_for_equal_strings_and_counts_substitutions() {
        assert_eq!(levenshtein("mysql", "mysql"), 0);
        assert_eq!(levenshtein("mongo", "mysql"), 4);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn nearest_capability_provider_finds_a_close_typo() {
        assert_eq!(
            nearest_capability_provider("db", "Mongo"),
            Some(("db", "MySQL"))
        );
        assert_eq!(
            nearest_capability_provider("qeue", "NATS"),
            Some(("queue", "NATS"))
        );
        assert_eq!(nearest_capability_provider("wat", "ThisIsNotReal"), None);
    }
}
