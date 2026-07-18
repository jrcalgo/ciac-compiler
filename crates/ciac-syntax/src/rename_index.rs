//! v0.18 M4: a whole-program resolved definition/reference index, built
//! from the module-merged AST **before** `ciac_sema::blueprints::expand`
//! runs — expansion mangles body-declared names into fresh synthetic
//! identifiers (`{name}{type_arg}`) while keeping the *original* span,
//! so an index built afterward would see the wrong name at that span
//! and couldn't tell "one blueprint body declaration" from "each of its
//! expansions" (18UpdatePlan.md Pillar 6). Indexing pre-expansion means
//! a blueprint body rename naturally covers every current and future
//! expansion site with one source edit.
//!
//! This is the resolution core only (18UpdatePlan.md Pillar 6's dry-run
//! steps 1-6): whole-program indexing, position/qualified symbol
//! resolution, and a validated multi-file source edit plan. It
//! deliberately does not know about generated outputs, seeded files, or
//! transactional multi-file writing — that's v0.18 M5.
//!
//! Namespaces mirror `ciac-sema`'s own scoping rules at a lightweight,
//! string-keyed level (record/table/stream are project-global; api/
//! worker/job/channel/crud/events/handler/capability-instance are
//! scoped per enclosing service or blueprint body) rather than reusing
//! `ciac-sema`'s resolver directly, since that resolver only ever runs
//! post-expansion. Two narrowed corners, disclosed rather than silently
//! approximated: chained field access (`a.b.c`) and pipeline-level
//! `match` field/variant resolution are left unresolved — both need
//! real type inference this milestone doesn't build. Direct cases (a
//! handler parameter typed as a record, a record literal, a `where`
//! clause on a `db.query(Table)` call) are resolved.

use crate::ast::*;
use crate::module::SourceOrigin;
use ciac_diagnostics::{Edit, FileId, Fix, Span};
use std::collections::{BTreeMap, HashMap};

pub type SymbolId = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SymbolKind {
    Project,
    Service,
    Record,
    ErrorRecord,
    Field,
    EnumVariant,
    Table,
    Stream,
    Api,
    Worker,
    Job,
    Channel,
    Crud,
    Events,
    Handler,
    CapabilityInstance,
    Blueprint,
    BlueprintTypeParam,
    BlueprintScalarParam,
    HandlerParam,
    LexicalLet,
}

impl SymbolKind {
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Project => "project",
            SymbolKind::Service => "service",
            SymbolKind::Record => "record",
            SymbolKind::ErrorRecord => "error record",
            SymbolKind::Field => "field",
            SymbolKind::EnumVariant => "enum variant",
            SymbolKind::Table => "table",
            SymbolKind::Stream => "stream",
            SymbolKind::Api => "api",
            SymbolKind::Worker => "worker",
            SymbolKind::Job => "job",
            SymbolKind::Channel => "channel",
            SymbolKind::Crud => "crud",
            SymbolKind::Events => "events",
            SymbolKind::Handler => "handler",
            SymbolKind::CapabilityInstance => "capability instance",
            SymbolKind::Blueprint => "blueprint",
            SymbolKind::BlueprintTypeParam => "blueprint type parameter",
            SymbolKind::BlueprintScalarParam => "blueprint parameter",
            SymbolKind::HandlerParam => "handler parameter",
            SymbolKind::LexicalLet => "local",
        }
    }
}

/// Builtin pipeline step names (`ast::StepExpr::Name`'s doc comment) —
/// never a rename target, never indexed as a reference.
const BUILTIN_STEPS: &[&str] = &["Auth", "Queue", "Return"];

#[derive(Debug, Clone)]
pub struct Definition {
    pub id: SymbolId,
    pub kind: SymbolKind,
    pub name: String,
    pub span: Span,
    /// A namespace path uniquely identifying this definition, e.g.
    /// `record/Order` or `service/Billing/handler/Charge/local/total`.
    /// Two definitions never share a key; a rename that would produce a
    /// new key already owned by another definition is a collision.
    pub key: String,
}

#[derive(Debug, Clone, Copy)]
pub struct Reference {
    pub symbol: SymbolId,
    pub span: Span,
}

#[derive(Debug, Default)]
pub struct SourceIndex {
    pub definitions: Vec<Definition>,
    pub references: Vec<Reference>,
}

#[derive(Debug, Clone)]
pub struct ResolvedSymbol {
    pub id: SymbolId,
    pub kind: SymbolKind,
    pub name: String,
    pub key: String,
    pub def_span: Span,
}

impl SourceIndex {
    pub fn definition(&self, id: SymbolId) -> &Definition {
        &self.definitions[id as usize]
    }

    fn resolved(&self, id: SymbolId) -> ResolvedSymbol {
        let d = self.definition(id);
        ResolvedSymbol {
            id: d.id,
            kind: d.kind,
            name: d.name.clone(),
            key: d.key.clone(),
            def_span: d.span,
        }
    }

    /// Every reference recorded against `id`.
    pub fn references_to(&self, id: SymbolId) -> impl Iterator<Item = &Reference> {
        self.references.iter().filter(move |r| r.symbol == id)
    }

    /// Every span (declaration site plus every reference) that a rename
    /// of `id` must rewrite.
    pub fn all_sites(&self, id: SymbolId) -> Vec<Span> {
        let mut sites = vec![self.definition(id).span];
        sites.extend(self.references_to(id).map(|r| r.span));
        sites
    }

    /// Position-based resolution (`--file/--line/--column`): the
    /// definition or reference whose span contains `offset` in `file`.
    /// Definitions win over references at the same position (renaming
    /// from the declaration site is the common case); returns every
    /// match, though in a well-formed program there is at most one,
    /// since spans of different symbols don't overlap.
    pub fn resolve_at(&self, file: FileId, offset: u32) -> Vec<ResolvedSymbol> {
        self.site_at(file, offset)
            .into_iter()
            .map(|(symbol, _span)| symbol)
            .collect()
    }

    /// Like [`resolve_at`](Self::resolve_at), but also returns the
    /// exact span that matched at `offset` — the specific reference or
    /// declaration site under the cursor, which may differ from the
    /// symbol's own declaration span. LSP's `prepareRename` needs this
    /// to highlight the right token, not just identify the symbol.
    pub fn site_at(&self, file: FileId, offset: u32) -> Vec<(ResolvedSymbol, Span)> {
        let hits: Vec<(SymbolId, Span)> = self
            .definitions
            .iter()
            .filter(|d| d.span.file == file && d.span.start <= offset && offset < d.span.end)
            .map(|d| (d.id, d.span))
            .collect();
        if !hits.is_empty() {
            return hits
                .into_iter()
                .map(|(id, span)| (self.resolved(id), span))
                .collect();
        }
        self.references
            .iter()
            .filter(|r| r.span.file == file && r.span.start <= offset && offset < r.span.end)
            .map(|r| (self.resolved(r.symbol), r.span))
            .collect()
    }

    /// Convenience qualified-name resolution: `Order` or `Order.total`.
    /// Ambiguous names return every candidate — the caller (CLI/MCP)
    /// reports them and asks for the position form instead of guessing.
    pub fn resolve_qualified(&self, name: &str) -> Vec<ResolvedSymbol> {
        match name.split_once('.') {
            Some((base, member)) => self
                .definitions
                .iter()
                .filter(|d| {
                    d.name == base && matches!(d.kind, SymbolKind::Record | SymbolKind::ErrorRecord)
                })
                .filter_map(|rec| {
                    let field_key = format!("record/{}/field/{member}", rec.name);
                    self.definitions.iter().find(|d| d.key == field_key)
                })
                .map(|d| self.resolved(d.id))
                .collect(),
            None => self
                .definitions
                .iter()
                .filter(|d| d.name == name)
                .map(|d| self.resolved(d.id))
                .collect(),
        }
    }

    /// Validates and computes a whole-program rename: identifier syntax,
    /// reserved words, namespace collisions, and that every affected
    /// site lies in locally-editable source (never `std/`/`registry:`
    /// text). Returns one [`Fix`] per affected file — the multi-file
    /// staging/journal/rollback write path is v0.18 M5; this is the
    /// resolved, validated plan that path will consume.
    pub fn plan_rename(
        &self,
        origins: &HashMap<FileId, SourceOrigin>,
        symbol: SymbolId,
        new_name: &str,
    ) -> Result<RenamePlan, RenameError> {
        if !is_valid_identifier(new_name) {
            return Err(RenameError::InvalidIdentifier(new_name.to_owned()));
        }
        if RESERVED_WORDS.contains(&new_name) {
            return Err(RenameError::ReservedWord(new_name.to_owned()));
        }
        let def = self.definition(symbol).clone();
        if new_name == def.name {
            return Err(RenameError::SameName);
        }
        let new_key = sibling_key(&def.key, new_name);
        if let Some(existing) = self
            .definitions
            .iter()
            .find(|d| d.id != symbol && d.key == new_key)
        {
            return Err(RenameError::Collision {
                key: new_key,
                existing_kind: existing.kind,
            });
        }

        let sites = self.all_sites(symbol);
        for span in &sites {
            if !is_locally_editable(origins.get(&span.file).copied()) {
                return Err(RenameError::NonLocalSource);
            }
        }

        let mut by_file: BTreeMap<FileId, Vec<Edit>> = BTreeMap::new();
        for span in sites {
            by_file.entry(span.file).or_default().push(Edit {
                span,
                replacement: new_name.to_owned(),
            });
        }
        let edits_by_file = by_file
            .into_iter()
            .map(|(file, edits)| {
                (
                    file,
                    Fix {
                        title: format!(
                            "rename {} `{}` to `{new_name}`",
                            def.kind.label(),
                            def.name
                        ),
                        edits,
                    },
                )
            })
            .collect();

        Ok(RenamePlan {
            symbol,
            kind: def.kind,
            old_name: def.name,
            new_name: new_name.to_owned(),
            edits_by_file,
        })
    }
}

fn sibling_key(key: &str, new_name: &str) -> String {
    match key.rfind('/') {
        Some(idx) => format!("{}/{new_name}", &key[..idx]),
        None => new_name.to_owned(),
    }
}

#[derive(Debug)]
pub struct RenamePlan {
    pub symbol: SymbolId,
    pub kind: SymbolKind,
    pub old_name: String,
    pub new_name: String,
    /// One [`Fix`] per affected file — apply each to that file's own
    /// source text via [`Fix::apply`].
    pub edits_by_file: Vec<(FileId, Fix)>,
}

#[derive(Debug)]
pub enum RenameError {
    InvalidIdentifier(String),
    ReservedWord(String),
    SameName,
    Collision {
        key: String,
        existing_kind: SymbolKind,
    },
    NonLocalSource,
}

impl std::fmt::Display for RenameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RenameError::InvalidIdentifier(name) => {
                write!(f, "`{name}` is not a valid CIaC identifier")
            }
            RenameError::ReservedWord(name) => write!(f, "`{name}` is a reserved word"),
            RenameError::SameName => write!(f, "the new name is the same as the current name"),
            RenameError::Collision { existing_kind, .. } => {
                write!(
                    f,
                    "a {} with that name already exists in scope",
                    existing_kind.label()
                )
            }
            RenameError::NonLocalSource => write!(
                f,
                "this symbol has a site in std/registry source, which rename never edits"
            ),
        }
    }
}

impl std::error::Error for RenameError {}

#[derive(Default, Clone, Copy)]
struct Scope<'a> {
    service: Option<&'a str>,
    blueprint: Option<&'a str>,
}

impl Scope<'_> {
    fn prefix(&self) -> String {
        if let Some(bp) = self.blueprint {
            format!("blueprint/{bp}/body")
        } else if let Some(svc) = self.service {
            format!("service/{svc}")
        } else {
            "flat".to_owned()
        }
    }
}

struct Builder {
    index: SourceIndex,
    by_key: HashMap<String, SymbolId>,
    records: HashMap<String, SymbolId>,
    record_fields: HashMap<String, HashMap<String, SymbolId>>,
    enum_variants: HashMap<(String, String), HashMap<String, SymbolId>>,
    tables: HashMap<String, SymbolId>,
    table_record: HashMap<String, String>,
    streams: HashMap<String, SymbolId>,
    services: HashMap<String, SymbolId>,
    components: HashMap<(String, String), SymbolId>,
    handlers: HashMap<(String, String), SymbolId>,
    instances: HashMap<(String, String), SymbolId>,
    blueprints: HashMap<String, SymbolId>,
    blueprint_params: HashMap<(String, String), SymbolId>,
}

/// Builds the whole-program index. `program` must be the module-merged
/// AST **before** blueprint expansion (see module doc comment).
pub fn build_index(program: &Program) -> SourceIndex {
    let mut b = Builder {
        index: SourceIndex::default(),
        by_key: HashMap::new(),
        records: HashMap::new(),
        record_fields: HashMap::new(),
        enum_variants: HashMap::new(),
        tables: HashMap::new(),
        table_record: HashMap::new(),
        streams: HashMap::new(),
        services: HashMap::new(),
        components: HashMap::new(),
        handlers: HashMap::new(),
        instances: HashMap::new(),
        blueprints: HashMap::new(),
        blueprint_params: HashMap::new(),
    };
    let scope = Scope::default();
    b.pass1_items(&program.items, scope);
    b.pass2_items(&program.items, scope);
    b.index
}

impl Builder {
    fn define(&mut self, kind: SymbolKind, ident: &Ident, key: String) -> SymbolId {
        let id = self.index.definitions.len() as SymbolId;
        self.index.definitions.push(Definition {
            id,
            kind,
            name: ident.text.clone(),
            span: ident.span,
            key,
        });
        self.by_key
            .insert(self.index.definitions[id as usize].key.clone(), id);
        id
    }

    fn reference(&mut self, symbol: SymbolId, span: Span) {
        self.index.references.push(Reference { symbol, span });
    }

    // ---------------------------------------------------------------
    // Pass 1: definitions.
    // ---------------------------------------------------------------

    fn pass1_items(&mut self, items: &[Item], scope: Scope<'_>) {
        for item in items {
            self.pass1_item(item, scope);
        }
    }

    fn pass1_item(&mut self, item: &Item, scope: Scope<'_>) {
        match item {
            Item::Import(_) | Item::Expand(_) => {}
            Item::Project(p) => {
                self.define(
                    SymbolKind::Project,
                    &p.name,
                    format!("project/{}", p.name.text),
                );
            }
            Item::Service(s) => {
                let id = self.define(
                    SymbolKind::Service,
                    &s.name,
                    format!("service/{}", s.name.text),
                );
                self.services.insert(s.name.text.clone(), id);
            }
            Item::ServiceBlock(sb) => {
                let id = self.define(
                    SymbolKind::Service,
                    &sb.name,
                    format!("service/{}", sb.name.text),
                );
                self.services.insert(sb.name.text.clone(), id);
                let inner = Scope {
                    service: Some(&sb.name.text),
                    blueprint: None,
                };
                for item in &sb.items {
                    self.pass1_service_item(item, inner);
                }
            }
            Item::Use(ub) => self.pass1_use(ub, scope),
            Item::Record(r) => self.pass1_record(r),
            Item::Stream(s) => {
                let id = self.define(
                    SymbolKind::Stream,
                    &s.name,
                    format!("stream/{}", s.name.text),
                );
                self.streams.insert(s.name.text.clone(), id);
            }
            Item::Table(t) => self.pass1_table(t),
            Item::Api(a) => self.pass1_component(SymbolKind::Api, &a.name, scope),
            Item::Worker(w) => self.pass1_component(SymbolKind::Worker, &w.name, scope),
            Item::Job(j) => self.pass1_component(SymbolKind::Job, &j.name, scope),
            Item::Channel(c) => self.pass1_component(SymbolKind::Channel, &c.name, scope),
            Item::Crud(c) => self.pass1_component(SymbolKind::Crud, &c.name, scope),
            Item::Events(e) => self.pass1_component(SymbolKind::Events, &e.name, scope),
            Item::Handler(h) => self.pass1_handler(h, scope),
            Item::Pipeline(_) => {}
            Item::Blueprint(bp) => self.pass1_blueprint(bp),
        }
    }

    fn pass1_service_item(&mut self, item: &ServiceItem, scope: Scope<'_>) {
        match item {
            ServiceItem::Use(ub) => self.pass1_use(ub, scope),
            ServiceItem::Api(a) => self.pass1_component(SymbolKind::Api, &a.name, scope),
            ServiceItem::Worker(w) => self.pass1_component(SymbolKind::Worker, &w.name, scope),
            ServiceItem::Job(j) => self.pass1_component(SymbolKind::Job, &j.name, scope),
            ServiceItem::Channel(c) => self.pass1_component(SymbolKind::Channel, &c.name, scope),
            ServiceItem::Crud(c) => self.pass1_component(SymbolKind::Crud, &c.name, scope),
            ServiceItem::Events(e) => self.pass1_component(SymbolKind::Events, &e.name, scope),
            ServiceItem::Handler(h) => self.pass1_handler(h, scope),
            ServiceItem::Pipeline(_) => {}
            ServiceItem::Expand(_) => {}
            ServiceItem::Table(t) => self.pass1_table(t),
        }
    }

    fn pass1_blueprint_item(&mut self, item: &BlueprintItem, scope: Scope<'_>) {
        match item {
            BlueprintItem::Use(ub) => self.pass1_use(ub, scope),
            BlueprintItem::Crud(c) => self.pass1_component(SymbolKind::Crud, &c.name, scope),
            BlueprintItem::Stream(s) => {
                let id = self.define(
                    SymbolKind::Stream,
                    &s.name,
                    format!("{}/{}", scope.prefix(), s.name.text),
                );
                self.streams.insert(s.name.text.clone(), id);
            }
            BlueprintItem::Handler(h) => self.pass1_handler(h, scope),
            BlueprintItem::Record(r) => self.pass1_record(r),
            BlueprintItem::Table(t) => self.pass1_table(t),
            BlueprintItem::Api(a) => self.pass1_component(SymbolKind::Api, &a.name, scope),
            BlueprintItem::Worker(w) => self.pass1_component(SymbolKind::Worker, &w.name, scope),
            BlueprintItem::Pipeline(_) => {}
        }
    }

    fn pass1_use(&mut self, ub: &UseBlock, scope: Scope<'_>) {
        for entry in &ub.entries {
            if let Some(name) = &entry.name {
                let key = format!("{}/instance/{}", scope.prefix(), name.text);
                let id = self.define(SymbolKind::CapabilityInstance, name, key.clone());
                self.instances
                    .insert((scope.prefix(), name.text.clone()), id);
            }
        }
    }

    fn pass1_record(&mut self, r: &RecordDecl) {
        let kind = match r.kind {
            RecordKind::Data => SymbolKind::Record,
            RecordKind::Error => SymbolKind::ErrorRecord,
        };
        let id = self.define(kind, &r.name, format!("record/{}", r.name.text));
        self.records.insert(r.name.text.clone(), id);
        let mut fields = HashMap::new();
        for field in &r.fields {
            let fkey = format!("record/{}/field/{}", r.name.text, field.name.text);
            let fid = self.define(SymbolKind::Field, &field.name, fkey);
            fields.insert(field.name.text.clone(), fid);
            if let TypeExpr::Enum { variants, .. } = &field.ty {
                let mut vmap = HashMap::new();
                for variant in variants {
                    let vkey = format!(
                        "record/{}/field/{}/variant/{}",
                        r.name.text, field.name.text, variant.text
                    );
                    let vid = self.define(SymbolKind::EnumVariant, variant, vkey);
                    vmap.insert(variant.text.clone(), vid);
                }
                self.enum_variants
                    .insert((r.name.text.clone(), field.name.text.clone()), vmap);
            }
        }
        self.record_fields.insert(r.name.text.clone(), fields);
    }

    fn pass1_table(&mut self, t: &TableDecl) {
        let id = self.define(SymbolKind::Table, &t.name, format!("table/{}", t.name.text));
        self.tables.insert(t.name.text.clone(), id);
        self.table_record
            .insert(t.name.text.clone(), t.record.text.clone());
    }

    fn pass1_component(&mut self, kind: SymbolKind, name: &Ident, scope: Scope<'_>) {
        let prefix = scope.prefix();
        let key = format!("{prefix}/component/{}", name.text);
        let id = self.define(kind, name, key);
        self.components.insert((prefix, name.text.clone()), id);
    }

    fn pass1_handler(&mut self, h: &HandlerDecl, scope: Scope<'_>) {
        let prefix = scope.prefix();
        let handler_key = format!("{prefix}/handler/{}", h.name.text);
        let id = self.define(SymbolKind::Handler, &h.name, handler_key.clone());
        self.handlers.insert((prefix, h.name.text.clone()), id);
        for param in &h.params {
            self.define(
                SymbolKind::HandlerParam,
                &param.name,
                format!("{handler_key}/local/{}", param.name.text),
            );
        }
        if let Some(body) = &h.body {
            self.pass1_stmts(body, &handler_key);
        }
    }

    fn pass1_stmts(&mut self, stmts: &[Stmt], handler_key: &str) {
        for stmt in stmts {
            match stmt {
                Stmt::Let { name, .. } => {
                    self.define(
                        SymbolKind::LexicalLet,
                        name,
                        format!("{handler_key}/local/{}", name.text),
                    );
                }
                Stmt::Transaction { body, .. } => self.pass1_stmts(body, handler_key),
                Stmt::Expr(expr) => self.pass1_expr_lets(expr, handler_key),
                Stmt::Return { value: Some(v), .. } => self.pass1_expr_lets(v, handler_key),
                Stmt::Return { value: None, .. } => {}
                Stmt::Fail { args, .. } => {
                    for a in args {
                        self.pass1_expr_lets(a, handler_key);
                    }
                }
                Stmt::Publish { value, .. } => self.pass1_expr_lets(value, handler_key),
            }
        }
    }

    /// `let`s nested inside `if`/`match` branch bodies (those live in
    /// `Expr`, not `Stmt`, per the grammar) still need pass-1 definition
    /// entries before pass 2 can look them up.
    fn pass1_expr_lets(&mut self, expr: &Expr, handler_key: &str) {
        match expr {
            Expr::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.pass1_stmts(then_branch, handler_key);
                if let Some(eb) = else_branch {
                    self.pass1_stmts(eb, handler_key);
                }
            }
            Expr::Match { arms, .. } => {
                for arm in arms {
                    self.pass1_stmts(&arm.body, handler_key);
                }
            }
            _ => {}
        }
    }

    fn pass1_blueprint(&mut self, bp: &BlueprintDecl) {
        let id = self.define(
            SymbolKind::Blueprint,
            &bp.name,
            format!("blueprint/{}", bp.name.text),
        );
        self.blueprints.insert(bp.name.text.clone(), id);
        self.define(
            SymbolKind::BlueprintTypeParam,
            &bp.type_param,
            format!("blueprint/{}/type_param", bp.name.text),
        );
        for param in &bp.params {
            let key = format!("blueprint/{}/param/{}", bp.name.text, param.name.text);
            let pid = self.define(SymbolKind::BlueprintScalarParam, &param.name, key);
            self.blueprint_params
                .insert((bp.name.text.clone(), param.name.text.clone()), pid);
        }
        let inner = Scope {
            service: None,
            blueprint: Some(&bp.name.text),
        };
        for item in &bp.body {
            self.pass1_blueprint_item(item, inner);
        }
    }

    // ---------------------------------------------------------------
    // Pass 2: references.
    // ---------------------------------------------------------------

    fn pass2_items(&mut self, items: &[Item], scope: Scope<'_>) {
        for item in items {
            self.pass2_item(item, scope);
        }
    }

    fn pass2_item(&mut self, item: &Item, scope: Scope<'_>) {
        match item {
            Item::Import(_) => {}
            Item::Project(_) | Item::Service(_) => {}
            Item::ServiceBlock(sb) => {
                let inner = Scope {
                    service: Some(&sb.name.text),
                    blueprint: None,
                };
                for item in &sb.items {
                    self.pass2_service_item(item, inner);
                }
            }
            Item::Use(_) => {}
            Item::Record(r) => self.pass2_record(r),
            Item::Stream(s) => self.reference_record(&s.record),
            Item::Table(t) => self.pass2_table(t, scope),
            Item::Api(a) => {
                if let Some(req) = &a.request {
                    self.reference_record(req);
                }
            }
            Item::Worker(w) => {
                if let Some(s) = &w.stream {
                    self.reference_stream(s);
                }
            }
            Item::Job(_) | Item::Events(_) => {}
            Item::Channel(c) => self.reference_stream(&c.stream),
            Item::Crud(c) => {
                if let Some(r) = &c.record {
                    self.reference_record(r);
                }
            }
            Item::Handler(h) => self.pass2_handler(h, scope),
            Item::Pipeline(p) => self.pass2_pipeline(p, scope),
            Item::Blueprint(bp) => self.pass2_blueprint(bp),
            Item::Expand(ex) => self.pass2_expand(ex),
        }
    }

    fn pass2_service_item(&mut self, item: &ServiceItem, scope: Scope<'_>) {
        match item {
            ServiceItem::Use(_) => {}
            ServiceItem::Api(a) => {
                if let Some(req) = &a.request {
                    self.reference_record(req);
                }
            }
            ServiceItem::Worker(w) => {
                if let Some(s) = &w.stream {
                    self.reference_stream(s);
                }
            }
            ServiceItem::Job(_) | ServiceItem::Events(_) => {}
            ServiceItem::Channel(c) => self.reference_stream(&c.stream),
            ServiceItem::Crud(c) => {
                if let Some(r) = &c.record {
                    self.reference_record(r);
                }
            }
            ServiceItem::Handler(h) => self.pass2_handler(h, scope),
            ServiceItem::Pipeline(p) => self.pass2_pipeline(p, scope),
            ServiceItem::Expand(ex) => self.pass2_expand(ex),
            ServiceItem::Table(t) => self.pass2_table(t, scope),
        }
    }

    fn pass2_blueprint_item(&mut self, item: &BlueprintItem, scope: Scope<'_>) {
        match item {
            BlueprintItem::Use(_) => {}
            BlueprintItem::Crud(c) => {
                if let Some(r) = &c.record {
                    self.reference_record(r);
                }
            }
            BlueprintItem::Stream(s) => self.reference_record(&s.record),
            BlueprintItem::Handler(h) => self.pass2_handler(h, scope),
            BlueprintItem::Record(r) => self.pass2_record(r),
            BlueprintItem::Table(t) => self.pass2_table(t, scope),
            BlueprintItem::Api(a) => {
                if let Some(req) = &a.request {
                    self.reference_record(req);
                }
            }
            BlueprintItem::Worker(w) => {
                if let Some(s) = &w.stream {
                    self.reference_stream(s);
                }
            }
            BlueprintItem::Pipeline(p) => self.pass2_pipeline(p, scope),
        }
    }

    fn reference_record(&mut self, ident: &Ident) {
        if let Some(&id) = self.records.get(&ident.text) {
            self.reference(id, ident.span);
        }
    }

    fn reference_stream(&mut self, ident: &Ident) {
        if let Some(&id) = self.streams.get(&ident.text) {
            self.reference(id, ident.span);
        }
    }

    fn pass2_record(&mut self, r: &RecordDecl) {
        for field in &r.fields {
            self.pass2_type(&field.ty);
        }
    }

    fn pass2_type(&mut self, ty: &TypeExpr) {
        match ty {
            TypeExpr::Named(ident) => self.reference_record(ident),
            TypeExpr::Reference { target, .. } => self.reference_record(target),
            TypeExpr::List { inner, .. } => self.pass2_type(inner),
            TypeExpr::Enum { .. } => {}
        }
    }

    fn pass2_table(&mut self, t: &TableDecl, scope: Scope<'_>) {
        self.reference_record(&t.record);
        if let Some(db) = &t.db {
            self.reference_instance(db, scope);
        }
    }

    fn reference_instance(&mut self, ident: &Ident, scope: Scope<'_>) {
        if let Some(&id) = self.instances.get(&(scope.prefix(), ident.text.clone())) {
            self.reference(id, ident.span);
        }
    }

    fn pass2_handler(&mut self, h: &HandlerDecl, scope: Scope<'_>) {
        for binding in &h.bindings {
            // `binding.capability` is a capability *kind* keyword
            // (`db`, `auth`, ..) — never indexed, per the reject list.
            self.reference_instance(&binding.instance, scope);
        }
        for param in &h.params {
            self.pass2_type(&param.ty);
        }
        if let Some(rt) = &h.return_ty {
            self.pass2_type(rt);
        }
        let prefix = scope.prefix();
        let handler_key = format!("{prefix}/handler/{}", h.name.text);
        if let Some(body) = &h.body {
            let mut locals = HashMap::new();
            let mut local_types = HashMap::new();
            for param in &h.params {
                if let Some(&id) = self
                    .by_key
                    .get(&format!("{handler_key}/local/{}", param.name.text))
                {
                    locals.insert(param.name.text.clone(), id);
                }
                if let Some(record_name) = named_or_reference_target(&param.ty) {
                    local_types.insert(param.name.text.clone(), record_name);
                }
            }
            self.walk_stmts(body, &mut locals, &mut local_types, &handler_key);
        }
    }

    fn pass2_pipeline(&mut self, p: &PipelineDecl, scope: Scope<'_>) {
        let prefix = scope.prefix();
        if let Some(&id) = self.components.get(&(prefix.clone(), p.name.text.clone())) {
            self.reference(id, p.name.span);
        }
        for step in &p.steps {
            self.pass2_step(step, &prefix);
        }
    }

    fn pass2_step(&mut self, step: &StepExpr, prefix: &str) {
        match step {
            StepExpr::Name(ident) => {
                if BUILTIN_STEPS.contains(&ident.text.as_str()) {
                    return;
                }
                if let Some(&id) = self.handlers.get(&(prefix.to_owned(), ident.text.clone())) {
                    self.reference(id, ident.span);
                }
            }
            StepExpr::Publish(ident) => self.reference_stream(ident),
            StepExpr::Call(qualified) => {
                if qualified.segments.len() == 2 {
                    let service = &qualified.segments[0];
                    let api = &qualified.segments[1];
                    if let Some(&sid) = self.services.get(&service.text) {
                        self.reference(sid, service.span);
                    }
                    let svc_prefix = format!("service/{}", service.text);
                    if let Some(&aid) = self.components.get(&(svc_prefix, api.text.clone())) {
                        self.reference(aid, api.span);
                    }
                }
            }
            StepExpr::Match { arms, .. } => {
                // The scrutinee `field` and each `ArmLabel::Variant` need
                // the runtime type of the value flowing through the
                // pipeline at that point, which this milestone's index
                // doesn't track (disclosed limitation, module doc
                // comment) — steps inside each arm are still indexed.
                for arm in arms {
                    for step in &arm.steps {
                        self.pass2_step(step, prefix);
                    }
                }
            }
        }
    }

    fn pass2_expand(&mut self, ex: &ExpandStmt) {
        if let Some(&id) = self.blueprints.get(&ex.blueprint.text) {
            self.reference(id, ex.blueprint.span);
        }
        self.reference_record(&ex.type_arg);
        for attr in &ex.args {
            if let Some(&id) = self
                .blueprint_params
                .get(&(ex.blueprint.text.clone(), attr.name.text.clone()))
            {
                self.reference(id, attr.name.span);
            }
        }
    }

    fn pass2_blueprint(&mut self, bp: &BlueprintDecl) {
        let inner = Scope {
            service: None,
            blueprint: Some(&bp.name.text),
        };
        for item in &bp.body {
            self.pass2_blueprint_item(item, inner);
        }
    }

    // -- handler-body expression walking (best-effort typing) --------

    fn walk_stmts(
        &mut self,
        stmts: &[Stmt],
        locals: &mut HashMap<String, SymbolId>,
        local_types: &mut HashMap<String, String>,
        handler_key: &str,
    ) {
        for stmt in stmts {
            match stmt {
                Stmt::Let { name, value, .. } => {
                    self.walk_expr(value, locals, local_types);
                    let inferred = self.infer_record_type(value, local_types);
                    if let Some(&id) = self
                        .by_key
                        .get(&format!("{handler_key}/local/{}", name.text))
                    {
                        locals.insert(name.text.clone(), id);
                    }
                    if let Some(rn) = inferred {
                        local_types.insert(name.text.clone(), rn);
                    }
                }
                Stmt::Expr(expr) => self.walk_expr(expr, locals, local_types),
                Stmt::Return { value: Some(v), .. } => self.walk_expr(v, locals, local_types),
                Stmt::Return { value: None, .. } => {}
                Stmt::Fail { error, args, .. } => {
                    self.reference_record(error);
                    for a in args {
                        self.walk_expr(a, locals, local_types);
                    }
                }
                Stmt::Publish { stream, value, .. } => {
                    self.reference_stream(stream);
                    self.walk_expr(value, locals, local_types);
                }
                Stmt::Transaction { body, .. } => {
                    self.walk_stmts(body, locals, local_types, handler_key)
                }
            }
        }
    }

    fn walk_expr(
        &mut self,
        expr: &Expr,
        locals: &HashMap<String, SymbolId>,
        local_types: &HashMap<String, String>,
    ) {
        match expr {
            Expr::Ident(ident) => {
                if let Some(&id) = locals.get(&ident.text) {
                    self.reference(id, ident.span);
                }
            }
            Expr::Number { .. } | Expr::Str { .. } | Expr::Bool { .. } => {}
            Expr::FieldAccess { base, field, .. } => {
                self.walk_expr(base, locals, local_types);
                if let Some(record_name) = self.infer_record_type(base, local_types) {
                    if let Some(&fid) = self
                        .record_fields
                        .get(&record_name)
                        .and_then(|m| m.get(&field.text))
                    {
                        self.reference(fid, field.span);
                    }
                }
            }
            Expr::Index { base, index, .. } => {
                self.walk_expr(base, locals, local_types);
                self.walk_expr(index, locals, local_types);
            }
            Expr::Call { callee, args, .. } => {
                self.walk_expr(callee, locals, local_types);
                for (i, arg) in args.iter().enumerate() {
                    self.walk_expr(arg, locals, local_types);
                    if i == 0 {
                        if let Expr::Ident(ident) = arg {
                            if let Some(&tid) = self.tables.get(&ident.text) {
                                self.reference(tid, ident.span);
                            }
                        }
                    }
                }
            }
            Expr::RecordCons { base, fields, .. } => {
                let record_name = match &**base {
                    Expr::Ident(ident) if self.records.contains_key(&ident.text) => {
                        self.reference(self.records[&ident.text], ident.span);
                        Some(ident.text.clone())
                    }
                    _ => {
                        self.walk_expr(base, locals, local_types);
                        self.infer_record_type(base, local_types)
                    }
                };
                for f in fields {
                    if let Some(rn) = &record_name {
                        if let Some(&fid) =
                            self.record_fields.get(rn).and_then(|m| m.get(&f.name.text))
                        {
                            self.reference(fid, f.name.span);
                        }
                    }
                    self.walk_expr(&f.value, locals, local_types);
                }
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.walk_expr(lhs, locals, local_types);
                self.walk_expr(rhs, locals, local_types);
            }
            Expr::Unary { expr, .. } => self.walk_expr(expr, locals, local_types),
            Expr::If {
                cond,
                then_branch,
                else_branch,
                ..
            } => {
                self.walk_expr(cond, locals, local_types);
                let mut then_locals = locals.clone();
                let mut then_types = local_types.clone();
                self.walk_stmts(then_branch, &mut then_locals, &mut then_types, "");
                if let Some(eb) = else_branch {
                    let mut else_locals = locals.clone();
                    let mut else_types = local_types.clone();
                    self.walk_stmts(eb, &mut else_locals, &mut else_types, "");
                }
            }
            Expr::Match {
                scrutinee, arms, ..
            } => {
                self.walk_expr(scrutinee, locals, local_types);
                let enum_ty = self.infer_scrutinee_enum(scrutinee, local_types);
                for arm in arms {
                    if let (ArmLabel::Variant(v), Some((rn, fld))) = (&arm.label, &enum_ty) {
                        if let Some(&vid) = self
                            .enum_variants
                            .get(&(rn.clone(), fld.clone()))
                            .and_then(|m| m.get(&v.text))
                        {
                            self.reference(vid, v.span);
                        }
                    }
                    let mut arm_locals = locals.clone();
                    let mut arm_types = local_types.clone();
                    self.walk_stmts(&arm.body, &mut arm_locals, &mut arm_types, "");
                }
            }
            Expr::Query {
                call, predicate, ..
            } => {
                self.walk_expr(call, locals, local_types);
                let table_record = self.infer_call_table_record(call);
                for term in &predicate.terms {
                    if let Some(rn) = &table_record {
                        if let Some(&fid) = self
                            .record_fields
                            .get(rn)
                            .and_then(|m| m.get(&term.field.text))
                        {
                            self.reference(fid, term.field.span);
                        }
                    }
                    self.walk_expr(&term.value, locals, local_types);
                }
            }
        }
    }

    fn infer_record_type(
        &self,
        expr: &Expr,
        local_types: &HashMap<String, String>,
    ) -> Option<String> {
        match expr {
            Expr::Ident(ident) => local_types.get(&ident.text).cloned(),
            Expr::RecordCons { base, .. } => match &**base {
                Expr::Ident(ident) if self.records.contains_key(&ident.text) => {
                    Some(ident.text.clone())
                }
                other => self.infer_record_type(other, local_types),
            },
            _ => None,
        }
    }

    fn infer_scrutinee_enum(
        &self,
        expr: &Expr,
        local_types: &HashMap<String, String>,
    ) -> Option<(String, String)> {
        if let Expr::FieldAccess { base, field, .. } = expr {
            let record_name = self.infer_record_type(base, local_types)?;
            if self
                .enum_variants
                .contains_key(&(record_name.clone(), field.text.clone()))
            {
                return Some((record_name, field.text.clone()));
            }
        }
        None
    }

    fn infer_call_table_record(&self, call: &Expr) -> Option<String> {
        if let Expr::Call { args, .. } = call {
            if let Some(Expr::Ident(ident)) = args.first() {
                return self.table_record.get(&ident.text).cloned();
            }
        }
        None
    }
}

fn named_or_reference_target(ty: &TypeExpr) -> Option<String> {
    match ty {
        TypeExpr::Named(ident) => Some(ident.text.clone()),
        TypeExpr::Reference { target, .. } => Some(target.text.clone()),
        _ => None,
    }
}

/// The reserved words a new identifier must avoid — every `#[token(..)]`
/// text in `crate::lexer::TokenKind` that's a bare word (not
/// punctuation), kept in sync by hand since `logos` doesn't expose its
/// token table for reuse at runtime.
pub const RESERVED_WORDS: &[&str] = &[
    "import",
    "blueprint",
    "expand",
    "params",
    "project",
    "service",
    "use",
    "api",
    "worker",
    "job",
    "channel",
    "pipeline",
    "crud",
    "events",
    "record",
    "stream",
    "handler",
    "on",
    "publish",
    "call",
    "enum",
    "match",
    "let",
    "true",
    "false",
    "if",
    "else",
    "table",
    "error",
    "extern",
    "return",
    "fail",
    "where",
    "contains",
    "Reference",
    "transaction",
];

/// Whether `name` is a syntactically valid CIaC identifier: `[A-Za-z_][A-Za-z0-9_]*`.
pub fn is_valid_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `origin` is text the rename engine may write back to.
/// `std/`/`registry:` sources are read-only, no matter what the
/// resolved symbol or reference is (18UpdatePlan.md Pillar 6).
pub fn is_locally_editable(origin: Option<SourceOrigin>) -> bool {
    matches!(origin, Some(SourceOrigin::Local) | None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciac_diagnostics::{Diagnostics, SourceMap};

    fn index_of(src: &str) -> (SourceIndex, SourceMap, ciac_diagnostics::FileId) {
        let mut sources = SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = Diagnostics::new();
        let program = crate::parser::parse(src, file, &mut diags);
        assert!(diags.is_empty(), "must parse cleanly: {:?}", diags.codes());
        (build_index(&program), sources, file)
    }

    #[test]
    fn indexes_record_and_field_and_resolves_qualified_name() {
        let (index, _sources, _file) = index_of(
            "service Ping;\nrecord Video { id: Uuid; title: String; }\napi Echo: Video { method: POST; path: \"/echo\"; }\npipeline Echo: Return;\n",
        );
        let hits = index.resolve_qualified("Video");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, SymbolKind::Record);

        let field_hits = index.resolve_qualified("Video.title");
        assert_eq!(field_hits.len(), 1);
        assert_eq!(field_hits[0].kind, SymbolKind::Field);
        assert_eq!(field_hits[0].name, "title");
    }

    #[test]
    fn record_reference_in_api_request_is_indexed() {
        let (index, _sources, _file) = index_of(
            "service Ping;\nrecord Video { id: Uuid; }\napi Echo: Video { method: POST; path: \"/echo\"; }\npipeline Echo: Return;\n",
        );
        let record = index.resolve_qualified("Video");
        assert_eq!(record.len(), 1);
        let refs: Vec<_> = index.references_to(record[0].id).collect();
        // One reference: `Video` named as Echo's request record.
        assert_eq!(refs.len(), 1);
    }

    #[test]
    fn position_resolution_finds_the_declaration_at_its_span() {
        let src = "service Ping;\nrecord Video { id: Uuid; }\n";
        let (index, sources, file) = index_of(src);
        let offset = src.find("Video").unwrap() as u32;
        let hits = index.resolve_at(file, offset);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, SymbolKind::Record);
        assert_eq!(hits[0].name, "Video");
        let _ = sources.file(file); // sanity: file is registered
    }

    #[test]
    fn handler_param_and_field_access_resolve() {
        let src = r#"
service Ping;
record Video { id: Uuid; title: String; }
api Echo: Video { method: POST; path: "/echo"; }
handler Echo(v: Video) -> Video {
    return v { title: v.title };
}
pipeline Echo: Echo -> Return;
"#;
        let (index, _sources, _file) = index_of(src);
        let field = index.resolve_qualified("Video.title");
        assert_eq!(field.len(), 1);
        // v.title (read) + { title: .. } (functional update) = 2 refs,
        // plus the field's own declaration is not a reference.
        let refs: Vec<_> = index.references_to(field[0].id).collect();
        assert_eq!(refs.len(), 2, "expected two field references, got {refs:?}");
    }

    #[test]
    fn builtin_step_names_are_never_indexed_as_references() {
        let src = "service Ping;\nrecord Msg { id: Uuid; }\napi Echo: Msg { method: POST; path: \"/echo\"; }\npipeline Echo: Return;\n";
        let (index, _sources, _file) = index_of(src);
        assert!(
            index.definitions.iter().all(|d| d.name != "Return"),
            "Return must never become a definition"
        );
    }

    #[test]
    fn reserved_words_and_identifier_syntax_are_checked() {
        assert!(is_valid_identifier("PurchaseOrder"));
        assert!(is_valid_identifier("_private"));
        assert!(!is_valid_identifier("2Fast"));
        assert!(!is_valid_identifier("has-dash"));
        assert!(RESERVED_WORDS.contains(&"record"));
        assert!(!RESERVED_WORDS.contains(&"PurchaseOrder"));
    }

    #[test]
    fn stream_and_worker_and_publish_reference_resolve() {
        let src = r#"
service Ping;
record Video { id: Uuid; }
stream Uploaded: Video;
worker Notify on Uploaded;
pipeline Notify: Return;
"#;
        let (index, _sources, _file) = index_of(src);
        let stream = index.resolve_qualified("Uploaded");
        assert_eq!(stream.len(), 1);
        assert_eq!(stream[0].kind, SymbolKind::Stream);
        let refs: Vec<_> = index.references_to(stream[0].id).collect();
        assert_eq!(
            refs.len(),
            1,
            "worker's `on Uploaded` should be the one reference"
        );
    }

    fn local_origins(index: &SourceIndex) -> HashMap<FileId, SourceOrigin> {
        let mut origins = HashMap::new();
        for d in &index.definitions {
            origins.insert(d.span.file, SourceOrigin::Local);
        }
        origins
    }

    #[test]
    fn plan_rename_produces_edits_for_every_site() {
        let src = "service Ping;\nrecord Video { id: Uuid; }\napi Echo: Video { method: POST; path: \"/echo\"; }\npipeline Echo: Return;\n";
        let (index, _sources, _file) = index_of(src);
        let record = index.resolve_qualified("Video");
        assert_eq!(record.len(), 1);
        let origins = local_origins(&index);
        let plan = index
            .plan_rename(&origins, record[0].id, "Clip")
            .expect("valid rename");
        assert_eq!(plan.old_name, "Video");
        assert_eq!(plan.new_name, "Clip");
        assert_eq!(plan.edits_by_file.len(), 1);
        let (_, fix) = &plan.edits_by_file[0];
        // Declaration + the api's request record = 2 sites.
        assert_eq!(fix.edits.len(), 2);
        let rewritten = fix.apply(src);
        assert!(rewritten.contains("record Clip"));
        assert!(rewritten.contains("api Echo: Clip"));
        assert!(!rewritten.contains("Video"));
    }

    #[test]
    fn plan_rename_rejects_reserved_word_and_bad_syntax_and_collision() {
        let src = "service Ping;\nrecord Video { id: Uuid; }\nrecord Clip { id: Uuid; }\n";
        let (index, _sources, _file) = index_of(src);
        let origins = local_origins(&index);
        let video = index.resolve_qualified("Video")[0].id;

        assert!(matches!(
            index.plan_rename(&origins, video, "record"),
            Err(RenameError::ReservedWord(_))
        ));
        assert!(matches!(
            index.plan_rename(&origins, video, "2Bad"),
            Err(RenameError::InvalidIdentifier(_))
        ));
        assert!(matches!(
            index.plan_rename(&origins, video, "Clip"),
            Err(RenameError::Collision { .. })
        ));
        assert!(matches!(
            index.plan_rename(&origins, video, "Video"),
            Err(RenameError::SameName)
        ));
    }

    #[test]
    fn plan_rename_refuses_non_local_source() {
        let src = "service Ping;\nrecord Video { id: Uuid; }\n";
        let (index, _sources, _file) = index_of(src);
        let video = index.resolve_qualified("Video")[0].id;
        // Every site's file is marked EmbeddedStd -- as if this record
        // had come from a `std/` blueprint rather than local source.
        let mut origins = HashMap::new();
        for d in &index.definitions {
            origins.insert(d.span.file, SourceOrigin::EmbeddedStd);
        }
        assert!(matches!(
            index.plan_rename(&origins, video, "Clip"),
            Err(RenameError::NonLocalSource)
        ));
    }
}
