//! Language-neutral template model built from the validated IR.
//!
//! Everything backend templates need is precomputed here as plain
//! serializable data: casing variants, per-pipeline step lists, per-field
//! type mappings, and which capabilities each generated unit must have
//! injected. Backends share this model so targets stay structurally
//! comparable; templates stay purely presentational.

use crate::GenOptions;
use ciac_ir::{
    Component, EdgeKind, FieldType, HttpMethod, NodeId, NodeKind, NormalizedIr, QueueEngine,
    RealtimeProvider, RecordId, ServiceId, Step, StepKind,
};
use heck::{ToKebabCase, ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// The generated system: one deployable project per declared service, or
/// a single unprefixed project for single-service programs.
#[derive(Debug, Serialize)]
pub struct SystemModel {
    /// System/project name, e.g. `media-system`.
    pub project_name: String,
    /// True when the program declares `service { .. }` blocks; the
    /// backends then emit each service under `<dir>/` plus root
    /// docker-compose/README files for the whole system.
    pub multi: bool,
    pub services: Vec<Ctx>,
    /// Any service publishes/consumes streams (the system compose then
    /// runs one shared broker every service's `nats_url` points at).
    pub has_queue: bool,
    /// Any service declares databases (drives the compose volume list).
    pub has_db: bool,
    pub has_object_store: bool,
}

/// Root template context for one deployable project.
#[derive(Debug, Serialize)]
pub struct Ctx {
    /// Original service name, e.g. `VideoPlatform`.
    pub service_name: String,
    /// Package name, e.g. `video-platform`.
    pub package: String,
    /// Module/crate prefix, e.g. `video_platform`.
    pub module: String,
    /// Output subdirectory (kebab service name) in multi-service
    /// systems; empty for single-service programs.
    pub dir: String,
    /// Host port the system docker-compose maps to the app's port 8000.
    pub host_port: u16,
    pub has_auth: bool,
    pub has_db: bool,
    pub has_cache: bool,
    pub has_queue: bool,
    pub db_instances: Vec<InstanceCtx>,
    pub cache_instances: Vec<InstanceCtx>,
    pub object_store_instances: Vec<OntologyInstanceCtx>,
    pub email_instances: Vec<OntologyInstanceCtx>,
    pub search_instances: Vec<OntologyInstanceCtx>,
    pub external_http_instances: Vec<OntologyInstanceCtx>,
    pub has_object_store: bool,
    pub has_email: bool,
    pub has_search: bool,
    pub has_external_http: bool,
    /// Sessionmaker key args for instances that back CRUD resources
    /// (schema creation targets), e.g. [""] or ["\"main\""].
    pub schema_key_args: Vec<String>,
    /// Rust `AppState` fields for the same schema targets, e.g. ["db"].
    pub schema_state_fields: Vec<String>,
    pub has_logging: bool,
    pub has_metrics: bool,
    pub queue_engine: Option<String>,
    /// Subject of the default stream backing the legacy `Queue` step.
    pub events_subject: String,
    pub records: Vec<RecordCtx>,
    /// Any record uses a `Timestamp` field (drives datetime imports/deps).
    pub records_use_datetime: bool,
    /// Any record uses a `Json` field (drives `Any` imports).
    pub records_use_json: bool,
    /// Any record uses an inline enum (drives `Literal`/enum declarations).
    pub records_use_enum: bool,
    pub apis: Vec<ApiCtx>,
    pub channels: Vec<ChannelCtx>,
    pub workers: Vec<WorkerCtx>,
    pub jobs: Vec<JobCtx>,
    pub consumers: Vec<ConsumerCtx>,
    pub services: Vec<ServiceCtx>,
    pub resources: Vec<ResourceCtx>,
    /// `table <Name>: <Record>;` declarations: one SQLAlchemy model per
    /// entry, registered on the same `Base` as CRUD resources so
    /// `create_schema()` picks it up with no migration-specific work.
    pub tables: Vec<TableCtx>,
    /// v0.7 typed handlers (inline body or `extern`) owned by this
    /// service, identified by node id — see the comment at their build
    /// site for why they aren't a `ServiceCtx`.
    pub typed_handlers: Vec<NodeId>,
    /// Downstream services invoked via `call`, one typed client each.
    pub call_targets: Vec<CallTargetCtx>,
}

/// A resolved `table <Name>: <Record>;` declaration, ready for a
/// SQLAlchemy model: same field data as [`RecordCtx`], named after the
/// table (not the record) so a record reused under several table names
/// doesn't collide.
#[derive(Debug, Serialize)]
pub struct TableCtx {
    pub class_name: String,
    pub snake: String,
    pub record: RecordCtx,
}

/// A resolved record type.
#[derive(Debug, Serialize)]
pub struct RecordCtx {
    /// Type name in both targets, e.g. `Video`.
    pub name: String,
    pub snake: String,
    /// Whether the record declares its own `id` field. Typed CRUD always
    /// stores a server-generated TEXT `id` primary key; when the record
    /// has one it doubles as that key, otherwise one is synthesized.
    pub has_id: bool,
    /// `error` (v0.7) vs. plain `record`: an error record generates a
    /// raisable exception type instead of a `BaseModel`.
    pub is_error: bool,
    /// Precomputed SQL fragments for typed CRUD, e.g.
    /// `id, title, status` / `$1, $2, $3` / `title = $2, status = $3`.
    pub select_cols: String,
    pub insert_placeholders: String,
    pub update_assignments: String,
    pub fields: Vec<FieldCtx>,
    /// Rust needs named enum types; one per inline-enum field.
    pub enums: Vec<EnumCtx>,
}

#[derive(Debug, Serialize)]
pub struct EnumCtx {
    /// e.g. `VideoStatus` for `record Video { status: enum { .. } }`.
    pub name: String,
    pub variants: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct FieldCtx {
    pub name: String,
    /// Python annotation, e.g. `str`, `datetime`, `Literal["A", "B"]`.
    pub py_type: String,
    /// Python annotation on the read path; enums come back from storage
    /// as their text form, so `Literal[..]` widens to `str`.
    pub py_out_type: String,
    /// Rust type, e.g. `String`, `chrono::DateTime<chrono::Utc>`, `VideoStatus`.
    pub rust_type: String,
    /// Rust type as stored in the database (enums are TEXT → `String`).
    pub db_rust_type: String,
    /// Postgres column type, e.g. `TEXT`, `BIGINT`, `JSONB`.
    pub sql_type: String,
    pub is_json: bool,
    pub is_enum: bool,
}

/// The payload type a pipeline (and its handlers) carries.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PayloadRef {
    /// Record/class name, e.g. `Video`, identical in both targets.
    pub class_name: String,
}

/// One named capability instance (db/cache), with every naming variant
/// codegen needs. The legacy/`default` instance keeps the unsuffixed
/// names so pre-v0.4 output is unchanged.
#[derive(Debug, Clone, Serialize)]
pub struct InstanceCtx {
    /// Instance name as declared, e.g. `default`, `main`.
    pub name: String,
    pub snake: String,
    pub is_default: bool,
    /// Settings field, e.g. `database_url` / `database_url_main`.
    pub url_field: String,
    /// Environment variable, e.g. `DATABASE_URL_MAIN`.
    pub env_var: String,
    /// Compose container name, e.g. `db` / `db-main`.
    pub container: String,
    /// Redis database index for cache instances (0, 1, ..).
    pub redis_db: u32,
    /// Rust `AppState` field, e.g. `db` / `db_main`.
    pub state_field: String,
    /// Postgres database name (db instances only).
    pub db_name: String,
    /// FastAPI session dependency (db instances only),
    /// e.g. `get_session` / `get_session_main`.
    pub session_dep: String,
    /// Route parameter name (db instances only), e.g. `session_main`.
    pub session_param: String,
    /// Argument for `get_sessionmaker(..)` / `get_cache(..)`:
    /// empty for the default instance, else `"name"` (quoted).
    pub key_arg: String,
}

/// One typed configuration field of an ontology capability instance.
#[derive(Debug, Clone, Serialize)]
pub struct CfgFieldCtx {
    /// Settings field name, e.g. `s3_endpoint_media`.
    pub field: String,
    /// Environment variable, e.g. `S3_ENDPOINT_MEDIA`.
    pub env: String,
    /// Python annotation: `str`, `int`, or `bool`.
    pub py_ann: String,
    /// Python default literal, e.g. `"http://localhost:9000"`, `1025`.
    pub py_default: String,
    /// Rust default literal (all ontology config is `String` in Rust;
    /// numeric/bool values are parsed at the use site).
    pub rust_default: String,
    /// Value wired into docker-compose (container DNS), when it differs
    /// from the development default.
    pub compose_value: Option<String>,
}

/// One instance of an ontology capability (object_store/email/search/
/// external_http) with its generated-client wiring.
#[derive(Debug, Clone, Serialize)]
pub struct OntologyInstanceCtx {
    /// Instance name as declared, e.g. `default`, `media`.
    pub name: String,
    pub snake: String,
    pub is_default: bool,
    /// Capability kind: `object_store` | `email` | `search` | `external_http`.
    pub kind: String,
    /// Provider, e.g. `S3`, `SES`, `SMTP`, `OpenSearch`.
    pub provider: Option<String>,
    /// Compose container name, when a local-dev container exists.
    pub container: Option<String>,
    /// Rust `AppState` field, e.g. `object_store_media`.
    pub state_field: String,
    /// Python getter argument: empty for default, else `"media"` (quoted).
    pub key_arg: String,
    /// Bucket name (object stores only).
    pub bucket: Option<String>,
    /// Host port docker-compose maps for the instance's inspection UI
    /// (email only: the Mailpit web interface).
    pub host_port: Option<u16>,
    pub cfg: Vec<CfgFieldCtx>,
}

/// A capability client a handler receives beyond db/cache.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtraDepCtx {
    /// Capability kind, e.g. `object_store`.
    pub kind: String,
    /// Constructor parameter name, e.g. `object_store`, `email`, `http`.
    pub param: String,
    /// Python parameter type, e.g. `ObjectStore`.
    pub py_type: String,
    /// Python module under `app.` providing the getter.
    pub py_module: String,
    /// Python getter expression, e.g. `get_object_store("media")`.
    pub py_expr: String,
    /// Python getter function name (for imports).
    pub py_getter: String,
    /// Rust type, e.g. `ObjectStore`.
    pub rust_type: String,
    /// Rust module providing the type, e.g. `object_store`.
    pub rust_module: String,
    /// Rust `AppState` field, e.g. `object_store_media`.
    pub rust_state_field: String,
}

/// A database session a route/worker needs for its handlers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionCtx {
    /// Parameter/variable name, e.g. `session` / `session_main`.
    pub param: String,
    /// FastAPI dependency function, e.g. `get_session_main`.
    pub dep: String,
    /// `get_sessionmaker(..)` argument (empty or `"main"` quoted).
    pub key_arg: String,
}

/// An api with a request pipeline.
#[derive(Debug, Serialize)]
pub struct ApiCtx {
    pub name: String,
    pub snake: String,
    /// Route path, e.g. `/upload`.
    pub route: String,
    pub method_upper: String,
    pub method_lower: String,
    pub scope: Option<String>,
    pub has_body: bool,
    /// Typed request payload; `None` = untyped JSON body.
    pub payload: Option<PayloadRef>,
    pub steps: Vec<StepCtx>,
    pub has_auth_step: bool,
    pub has_publish_step: bool,
    /// Whether any invoked handler needs a database session / cache client.
    pub needs_db: bool,
    pub needs_cache: bool,
    /// Distinct database sessions the route must inject, in first-use order.
    pub db_sessions: Vec<SessionCtx>,
    /// Comma-joined dependency names for the `from app.db import ..` line.
    pub db_imports: String,
    /// Deduplicated ontology getters the route imports, per module.
    pub extra_imports: Vec<ExtraImportCtx>,
    /// Deduplicated service clients the route invokes, for imports.
    pub call_imports: Vec<CallCtx>,
    /// Deduplicated handlers, in invocation order, for imports.
    pub handlers: Vec<HandlerRef>,
}

/// A realtime route exposing a stream over WebSocket or SSE.
#[derive(Debug, Serialize)]
pub struct ChannelCtx {
    pub name: String,
    pub snake: String,
    pub path: String,
    pub subject: String,
    pub provider: String,
    pub payload: Option<PayloadRef>,
}

/// One `from app.<module> import <getter>` line a route/worker needs.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtraImportCtx {
    pub py_module: String,
    pub py_getter: String,
}

#[derive(Debug, Serialize)]
pub struct StepCtx {
    /// One of `auth`, `handler`, `publish`, `return`, `match`.
    pub kind: &'static str,
    pub handler: Option<HandlerRef>,
    /// Subject of the published stream, for `publish` steps.
    pub subject: Option<String>,
    pub call: Option<CallCtx>,
    pub field: Option<String>,
    pub arms: Vec<ArmCtx>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallCtx {
    pub service: String,
    pub api: String,
    /// Client module under `app/clients` / `src/clients`, e.g. `billing`.
    pub module: String,
    /// Client class/struct name, e.g. `BillingClient`.
    pub class_name: String,
    /// Client method, e.g. `charge`.
    pub method: String,
}

/// A downstream service this service invokes via `call <Service>.<Api>`:
/// the compiler generates one typed HTTP client per target.
#[derive(Debug, Serialize)]
pub struct CallTargetCtx {
    /// Target service name, e.g. `Billing`.
    pub service: String,
    /// Client module, e.g. `billing`.
    pub module: String,
    /// Compose DNS name of the target's app container, e.g. `billing`.
    pub kebab: String,
    /// Client class/struct, e.g. `BillingClient`.
    pub class_name: String,
    /// Settings/Config field holding the base URL, e.g. `billing_url`.
    pub url_field: String,
    /// Environment variable, e.g. `BILLING_URL`.
    pub env_var: String,
    /// Development default: the target's host port from the system
    /// compose mapping, e.g. `http://localhost:8000`.
    pub default_url: String,
    /// Record class names the client file imports.
    pub schema_imports: Vec<String>,
    /// Any called api lacks a typed payload (drives dict/Any signatures).
    pub needs_any: bool,
    pub apis: Vec<CallApiCtx>,
}

/// One api of a call target, generated as a client method.
#[derive(Debug, Serialize)]
pub struct CallApiCtx {
    /// Api name, e.g. `Charge`.
    pub name: String,
    /// Client method name, e.g. `charge`.
    pub method: String,
    /// HTTP method for the request, e.g. `post`.
    pub http_method_lower: String,
    /// Route path, e.g. `/charge`.
    pub path: String,
    pub has_body: bool,
    pub payload: Option<PayloadRef>,
}

#[derive(Debug, Serialize)]
pub struct ArmCtx {
    pub label: Option<String>,
    pub rust_variant: Option<String>,
    pub steps: Vec<StepCtx>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HandlerRef {
    /// Class name, e.g. `StoreVideo`.
    pub class_name: String,
    /// Module name, e.g. `store_video`.
    pub module: String,
    /// Package the module lives under: `services` for classic/`extern`
    /// handlers (seeded, user-owned), `logic` for v0.7 inline typed
    /// handlers (compiler-owned, lowered from the HIR).
    pub py_package: &'static str,
    pub needs_db: bool,
    pub needs_cache: bool,
    pub bindings: Vec<BindingCtx>,
    /// Precomputed Python constructor arguments for invoking this handler
    /// from a route/worker, e.g. `session=session_main, cache=get_cache("hot")`.
    /// Keeps templates free of whitespace gymnastics.
    pub py_args: String,
    /// The database session this handler consumes, when bound.
    pub db_session: Option<SessionCtx>,
    /// Rust `AppState` field for the bound database, e.g. `db_main`.
    pub rust_db_field: Option<String>,
    /// Rust `AppState` field for the bound cache, e.g. `cache_hot`.
    pub rust_cache_field: Option<String>,
    /// Ontology clients this handler receives (object stores, email,
    /// search, external HTTP), resolved to their bound instances.
    pub extras: Vec<ExtraDepCtx>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BindingCtx {
    pub kind: String,
    pub name: String,
    pub snake: String,
    pub py_attr: String,
    pub rust_field: String,
}

/// A declared worker with (or without) a processing pipeline.
#[derive(Debug, Serialize)]
pub struct WorkerCtx {
    pub name: String,
    pub snake: String,
    /// Subject the worker consumes.
    pub subject: String,
    /// Queue group so replicas of this worker load-balance.
    pub queue_group: String,
    /// Typed message payload; `None` = untyped JSON.
    pub payload: Option<PayloadRef>,
    pub concurrency: u32,
    pub max_retries: u32,
    pub has_publish_step: bool,
    pub steps: Vec<StepCtx>,
    pub handlers: Vec<HandlerRef>,
    pub needs_db: bool,
    pub needs_cache: bool,
    /// Distinct database sessions the worker opens per message.
    pub db_sessions: Vec<SessionCtx>,
    /// Pre-joined `async with` items, e.g.
    /// `get_sessionmaker()() as session, get_sessionmaker("main")() as session_main`.
    pub session_with: String,
    /// Deduplicated ontology getters the worker imports, per module.
    pub extra_imports: Vec<ExtraImportCtx>,
    /// Deduplicated service clients the worker invokes, for imports.
    pub call_imports: Vec<CallCtx>,
}

/// A scheduled job with (or without) a processing pipeline.
#[derive(Debug, Serialize)]
pub struct JobCtx {
    pub name: String,
    pub snake: String,
    pub schedule: String,
    /// `schedule` translated to the `cron` crate's expectations: a leading
    /// seconds field, and weekday `0` (CIaC's Sunday, matching POSIX cron)
    /// rewritten as `7` (the crate only accepts 1-7). Ranges, lists, and
    /// steps in the weekday field are expanded to an explicit list so the
    /// rewrite stays correct across the 0-6/1-7 numbering gap. Rust-only;
    /// other backends parse `schedule` with a library that already accepts
    /// the 0-7 convention.
    pub cron_crate_schedule: String,
    pub catch_up: bool,
    pub has_publish_step: bool,
    pub steps: Vec<StepCtx>,
    pub handlers: Vec<HandlerRef>,
    pub needs_db: bool,
    pub needs_cache: bool,
    /// Distinct database sessions the job opens per tick.
    pub db_sessions: Vec<SessionCtx>,
    /// Pre-joined `async with` items, e.g.
    /// `get_sessionmaker()() as session, get_sessionmaker("main")() as session_main`.
    pub session_with: String,
    /// Deduplicated ontology getters the job imports, per module.
    pub extra_imports: Vec<ExtraImportCtx>,
    /// Deduplicated service clients the job invokes, for imports.
    pub call_imports: Vec<CallCtx>,
}

/// A consumer generated from `events <Name>;`.
#[derive(Debug, Serialize)]
pub struct ConsumerCtx {
    pub name: String,
    pub snake: String,
    pub subject: String,
    pub queue_group: String,
    pub needs_db: bool,
}

/// An implicit service handler module.
#[derive(Debug, Serialize)]
pub struct ServiceCtx {
    pub class_name: String,
    pub module: String,
    /// Payload type when every pipeline using this handler agrees on one;
    /// `None` falls back to untyped JSON.
    pub payload: Option<PayloadRef>,
    pub needs_db: bool,
    pub needs_cache: bool,
    pub bindings: Vec<BindingCtx>,
    /// Rust `AppState` fields for the bound db/cache instances.
    pub rust_db_field: Option<String>,
    pub rust_cache_field: Option<String>,
    /// Ontology clients this handler receives.
    pub extras: Vec<ExtraDepCtx>,
}

/// A CRUD resource from `crud <Name>;`.
#[derive(Debug, Serialize)]
pub struct ResourceCtx {
    /// e.g. `Note`.
    pub name: String,
    /// e.g. `note`.
    pub snake: String,
    /// Route base and table name, e.g. `notes`.
    pub plural: String,
    /// Store class/module, e.g. `NoteStore` / `note_store`.
    pub store_class: String,
    pub store_module: String,
    /// Typed columns from `crud <Name>: <Record>;`; `None` keeps the
    /// generic keyed-document model.
    pub record: Option<RecordCtx>,
    pub has_auth: bool,
    pub has_cache: bool,
    pub cache_ttl: u32,
    pub page_size: u32,
    /// Session dependency backing this resource's store.
    pub db_session: SessionCtx,
    /// `get_cache(..)` expression when caching is enabled.
    pub cache_expr: String,
    /// Rust `AppState` fields for the bound instances.
    pub rust_db_field: String,
    pub rust_cache_field: Option<String>,
}

/// Builds the whole system: one [`Ctx`] per declared service, or a single
/// unscoped [`Ctx`] for single-service programs (unchanged output).
pub fn build_system(ir: &NormalizedIr, opts: &GenOptions) -> SystemModel {
    let project_name = crate::project_name(ir, opts);
    let mut services = if ir.multi_service {
        let declared: Vec<ciac_ir::Service> = ir.services().cloned().collect();
        declared
            .iter()
            .enumerate()
            .map(|(index, service)| build_scoped(ir, opts, Some(service), 8000 + index as u16))
            .collect()
    } else {
        vec![build_scoped(ir, opts, None, 8000)]
    };
    // Mailpit UI host ports must be unique across the whole system.
    let mut mail_port = 8025;
    for ctx in &mut services {
        for inst in &mut ctx.email_instances {
            inst.host_port = Some(mail_port);
            mail_port += 1;
        }
    }
    SystemModel {
        project_name,
        multi: ir.multi_service,
        has_queue: services.iter().any(|c| c.has_queue),
        has_db: services.iter().any(|c| c.has_db),
        has_object_store: services.iter().any(|c| c.has_object_store),
        services,
    }
}

fn build_scoped(
    ir: &NormalizedIr,
    opts: &GenOptions,
    scope: Option<&ciac_ir::Service>,
    host_port: u16,
) -> Ctx {
    let sid = scope.map(|s| s.id);
    let base_name = scope.map_or_else(|| ir.name.clone(), |s| s.name.clone());
    let dir = scope.map_or_else(String::new, |s| s.name.to_kebab_case());
    let container_prefix = if dir.is_empty() {
        String::new()
    } else {
        format!("{dir}-")
    };
    let package = if scope.is_some() {
        dir.clone()
    } else {
        crate::project_name(ir, opts)
    };
    let module = base_name.to_snake_case();
    let capability = |kind: NodeKind| {
        ir.nodes_of_kind(kind)
            .find(|node| infra_in_scope(node.service, sid))
    };
    let has_db = capability(NodeKind::Database).is_some();
    let has_cache = capability(NodeKind::Cache).is_some();

    let resource_services: BTreeSet<NodeId> = ir.resources.iter().map(|r| r.service).collect();
    let consumer_workers: BTreeSet<NodeId> = ir.event_streams.iter().map(|s| s.worker).collect();

    let record_ctx = |id: RecordId| build_record(ir, id);
    let payload_ref = |record: Option<RecordId>| -> Option<PayloadRef> {
        record.map(|id| PayloadRef {
            class_name: ir.record(id).name.clone(),
        })
    };

    // Payload type per handler node: the single payload every pipeline
    // using it agrees on, else untyped.
    let mut handler_payloads: BTreeMap<NodeId, Option<RecordId>> = BTreeMap::new();
    for pipeline in ir.pipelines.iter().filter(|p| owned_by(p.service, sid)) {
        for node in handler_nodes(&pipeline.steps) {
            handler_payloads
                .entry(node)
                .and_modify(|existing| {
                    if *existing != pipeline.payload {
                        *existing = None;
                    }
                })
                .or_insert(pipeline.payload);
        }
    }

    let apis = ir
        .nodes_of_kind(NodeKind::Api)
        .filter(|api| owned_by(api.service, sid) && ir.pipeline_of(api.id).is_some())
        .map(|api| {
            let name = api.component.name().unwrap_or_default().to_owned();
            let pipeline = ir
                .pipeline_of(api.id)
                .expect("filtered to apis with pipelines");
            let (steps, handlers) = steps_of(ir, api.id);
            let payload = payload_ref(pipeline.payload);
            let (method, path, scope) = match &api.component {
                Component::Api { config, .. } => {
                    let path = config
                        .path
                        .clone()
                        .unwrap_or_else(|| format!("/{}", name.to_kebab_case()));
                    (config.method, path, config.scope.clone())
                }
                _ => unreachable!("api node is an api"),
            };
            let method_upper = method.as_str().to_owned();
            let method_lower = method_upper.to_ascii_lowercase();
            let has_body = !matches!(method, HttpMethod::Get | HttpMethod::Delete);
            let db_sessions = sessions_of(&handlers);
            let db_imports = db_sessions
                .iter()
                .map(|s| s.dep.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let extra_imports = extra_imports_of(&handlers);
            let call_imports = call_imports_of(&steps);
            ApiCtx {
                route: path,
                method_upper,
                method_lower,
                scope,
                has_body,
                extra_imports,
                call_imports,
                snake: name.to_snake_case(),
                has_auth_step: steps.iter().any(|s| s.kind == "auth"),
                has_publish_step: has_publish(&steps),
                needs_db: handlers.iter().any(|h| h.needs_db),
                needs_cache: handlers.iter().any(|h| h.needs_cache),
                db_sessions,
                db_imports,
                payload,
                steps,
                handlers,
                name,
            }
        })
        .collect();

    let realtime_provider = capability(NodeKind::Realtime).and_then(|node| match &node.component {
        Component::Realtime { provider, .. } => Some(match provider {
            RealtimeProvider::WebSocket => "websocket".to_owned(),
            RealtimeProvider::Sse => "sse".to_owned(),
        }),
        _ => None,
    });
    let channels = ir
        .nodes_of_kind(NodeKind::Channel)
        .filter(|channel| owned_by(channel.service, sid))
        .filter_map(|channel| {
            let name = channel.component.name().unwrap_or_default().to_owned();
            let stream = ir
                .edges_to(channel.id)
                .find(|edge| edge.kind == EdgeKind::AsyncMessage)
                .map(|edge| edge.from)?;
            let (subject, payload) = match &ir.node(stream).component {
                Component::Stream {
                    subject, record, ..
                } => (subject.clone(), payload_ref(*record)),
                _ => return None,
            };
            let path = match &channel.component {
                Component::Channel { config, .. } => config.path.clone(),
                _ => unreachable!("channel node is a channel"),
            };
            Some(ChannelCtx {
                snake: name.to_snake_case(),
                provider: realtime_provider
                    .clone()
                    .unwrap_or_else(|| "websocket".to_owned()),
                subject,
                payload,
                path,
                name,
            })
        })
        .collect();

    // Matches the subject `ciac-sema` assigns to the default stream, which
    // is always derived from the project/system name.
    let default_subject = format!("{}.events", ir.name.to_snake_case());
    let workers = ir
        .nodes_of_kind(NodeKind::Worker)
        .filter(|w| owned_by(w.service, sid) && !consumer_workers.contains(&w.id))
        .map(|worker| {
            let name = worker.component.name().unwrap_or_default().to_owned();
            let (steps, handlers) = steps_of(ir, worker.id);
            // The stream this worker consumes (via `on` or the default).
            let consumed = ir
                .edges_to(worker.id)
                .find(|e| e.kind == EdgeKind::AsyncMessage)
                .map(|e| e.from);
            let subject = consumed
                .map(|id| stream_subject(ir, id))
                .unwrap_or_else(|| default_subject.clone());
            let payload = ir
                .pipeline_of(worker.id)
                .and_then(|p| payload_ref(p.payload));
            let config = match &worker.component {
                Component::Worker { config, .. } => config,
                _ => unreachable!("worker node is a worker"),
            };
            let db_sessions = sessions_of(&handlers);
            let session_with = db_sessions
                .iter()
                .map(|s| format!("get_sessionmaker({})() as {}", s.key_arg, s.param))
                .collect::<Vec<_>>()
                .join(", ");
            let extra_imports = extra_imports_of(&handlers);
            let call_imports = call_imports_of(&steps);
            WorkerCtx {
                extra_imports,
                call_imports,
                snake: name.to_snake_case(),
                queue_group: name.to_snake_case(),
                needs_db: handlers.iter().any(|h| h.needs_db),
                needs_cache: handlers.iter().any(|h| h.needs_cache),
                has_publish_step: has_publish(&steps),
                concurrency: config.concurrency,
                max_retries: config.max_retries,
                db_sessions,
                session_with,
                subject,
                payload,
                steps,
                handlers,
                name,
            }
        })
        .collect();

    let jobs = ir
        .nodes_of_kind(NodeKind::Job)
        .filter(|job| owned_by(job.service, sid))
        .map(|job| {
            let name = job.component.name().unwrap_or_default().to_owned();
            let (steps, handlers) = steps_of(ir, job.id);
            let config = match &job.component {
                Component::Job { config, .. } => config,
                _ => unreachable!("job node is a job"),
            };
            let db_sessions = sessions_of(&handlers);
            let session_with = db_sessions
                .iter()
                .map(|s| format!("get_sessionmaker({})() as {}", s.key_arg, s.param))
                .collect::<Vec<_>>()
                .join(", ");
            let extra_imports = extra_imports_of(&handlers);
            let call_imports = call_imports_of(&steps);
            JobCtx {
                extra_imports,
                call_imports,
                snake: name.to_snake_case(),
                needs_db: handlers.iter().any(|h| h.needs_db),
                needs_cache: handlers.iter().any(|h| h.needs_cache),
                has_publish_step: has_publish(&steps),
                db_sessions,
                session_with,
                cron_crate_schedule: cron_crate_schedule(&config.schedule),
                schedule: config.schedule.clone(),
                catch_up: config.catch_up,
                steps,
                handlers,
                name,
            }
        })
        .collect();

    let consumers = ir
        .event_streams
        .iter()
        .filter(|stream| owned_by(stream.service_owner, sid))
        .map(|stream| {
            let name = ir
                .node(stream.worker)
                .component
                .name()
                .unwrap_or_default()
                .to_owned();
            ConsumerCtx {
                snake: name.to_snake_case(),
                queue_group: name.to_snake_case(),
                subject: stream_subject(ir, stream.stream),
                needs_db: has_db,
                name,
            }
        })
        .collect();

    let is_typed_handler = |component: &Component| {
        matches!(
            component,
            Component::Service {
                signature: Some(_),
                ..
            }
        )
    };

    let services = ir
        .nodes_of_kind(NodeKind::Service)
        .filter(|s| {
            owned_by(s.service, sid)
                && !resource_services.contains(&s.id)
                && !is_typed_handler(&s.component)
        })
        .map(|service| {
            let name = service.component.name().unwrap_or_default().to_owned();
            let bindings = bindings_of(ir, service.id);
            let access = access_of(&bindings);
            ServiceCtx {
                module: name.to_snake_case(),
                payload: payload_ref(handler_payloads.get(&service.id).copied().flatten()),
                needs_db: access.db.is_some(),
                needs_cache: access.cache_expr.is_some(),
                rust_db_field: access.rust_db_field,
                rust_cache_field: access.rust_cache_field,
                extras: extras_of(&bindings),
                bindings,
                class_name: name,
            }
        })
        .collect();

    // v0.7 typed handlers (inline body or `extern`): the classic
    // `ServiceCtx` shape doesn't fit a real param/return signature, so
    // backends that support them (Python, M3) lower the raw HIR
    // themselves via this node list; backends that don't (Rust, until
    // M4) never see them here since `check_support` gates the build
    // first.
    let typed_handlers: Vec<NodeId> = ir
        .nodes_of_kind(NodeKind::Service)
        .filter(|s| owned_by(s.service, sid) && is_typed_handler(&s.component))
        .map(|s| s.id)
        .collect();

    let resources: Vec<ResourceCtx> = ir
        .resources
        .iter()
        .filter(|resource| owned_by(resource.service_owner, sid))
        .map(|resource| {
            let snake = resource.name.to_snake_case();
            let bindings = bindings_of(ir, resource.service);
            let access = access_of(&bindings);
            ResourceCtx {
                name: resource.name.clone(),
                plural: format!("{snake}s"),
                store_class: format!("{}Store", resource.name),
                store_module: format!("{snake}_store"),
                record: resource.record.map(record_ctx),
                has_auth: capability(NodeKind::Auth).is_some(),
                has_cache: access.cache_expr.is_some(),
                cache_ttl: resource.config.cache_ttl,
                page_size: resource.config.page_size,
                db_session: access.db.unwrap_or(SessionCtx {
                    param: "session".to_owned(),
                    dep: "get_session".to_owned(),
                    key_arg: String::new(),
                }),
                cache_expr: access
                    .cache_expr
                    .unwrap_or_else(|| "get_cache()".to_owned()),
                rust_db_field: access.rust_db_field.unwrap_or_else(|| "db".to_owned()),
                rust_cache_field: access.rust_cache_field,
                snake,
            }
        })
        .collect();

    // One typed HTTP client per downstream service this service `call`s.
    let mut call_targets: Vec<CallTargetCtx> = Vec::new();
    for pipeline in ir.pipelines.iter().filter(|p| owned_by(p.service, sid)) {
        for target in call_nodes(&pipeline.steps) {
            let node = ir.node(target);
            let Some(owner) = node.service else { continue };
            let Component::Api { config, .. } = &node.component else {
                continue;
            };
            let target_service = ir.service(owner).name.clone();
            let api_name = node.component.name().unwrap_or_default().to_owned();
            let path = config
                .path
                .clone()
                .unwrap_or_else(|| format!("/{}", api_name.to_kebab_case()));
            let entry_index = match call_targets
                .iter()
                .position(|t| t.service == target_service)
            {
                Some(index) => index,
                None => {
                    let position = ir
                        .services()
                        .position(|s| s.id == owner)
                        .unwrap_or_default();
                    let url_field = format!("{}_url", target_service.to_snake_case());
                    call_targets.push(CallTargetCtx {
                        module: target_service.to_snake_case(),
                        kebab: target_service.to_kebab_case(),
                        class_name: format!("{}Client", target_service.to_pascal_case()),
                        env_var: url_field.to_shouty_snake_case(),
                        url_field,
                        default_url: format!("http://localhost:{}", 8000 + position),
                        schema_imports: Vec::new(),
                        needs_any: false,
                        apis: Vec::new(),
                        service: target_service.clone(),
                    });
                    call_targets.len() - 1
                }
            };
            let entry = &mut call_targets[entry_index];
            if entry.apis.iter().any(|a| a.name == api_name) {
                continue;
            }
            let payload = ir.pipeline_of(target).and_then(|p| payload_ref(p.payload));
            entry.apis.push(CallApiCtx {
                method: api_name.to_snake_case(),
                http_method_lower: config.method.as_str().to_ascii_lowercase(),
                has_body: !matches!(config.method, HttpMethod::Get | HttpMethod::Delete),
                path,
                payload,
                name: api_name,
            });
        }
    }
    for target in &mut call_targets {
        for api in &target.apis {
            match &api.payload {
                Some(payload) => {
                    if !target.schema_imports.contains(&payload.class_name) {
                        target.schema_imports.push(payload.class_name.clone());
                    }
                }
                None => target.needs_any = true,
            }
        }
    }

    let db_instances = instances_of(ir, NodeKind::Database, &module, sid, &container_prefix);
    let cache_instances = instances_of(ir, NodeKind::Cache, &module, sid, &container_prefix);
    let object_store_instances =
        ontology_instances(ir, NodeKind::ObjectStore, sid, &container_prefix);
    let email_instances = ontology_instances(ir, NodeKind::Email, sid, &container_prefix);
    let search_instances = ontology_instances(ir, NodeKind::Search, sid, &container_prefix);
    let external_http_instances =
        ontology_instances(ir, NodeKind::ExternalHttp, sid, &container_prefix);

    let records: Vec<RecordCtx> = ir.records().map(|(id, _)| build_record(ir, id)).collect();
    let tables: Vec<TableCtx> = ir
        .tables()
        .map(|(_, table)| TableCtx {
            class_name: table.name.clone(),
            snake: table.name.to_snake_case(),
            record: build_record(ir, table.record),
        })
        .collect();
    let all_fields = |records: &[RecordCtx]| -> Vec<String> {
        records
            .iter()
            .flat_map(|r| r.fields.iter().map(|f| f.py_type.clone()))
            .collect()
    };
    let field_types = all_fields(&records);

    Ctx {
        service_name: base_name,
        package,
        dir,
        host_port,
        has_auth: capability(NodeKind::Auth).is_some(),
        has_db,
        has_cache,
        schema_key_args: {
            let mut keys: Vec<String> = Vec::new();
            for resource in &resources {
                if !keys.contains(&resource.db_session.key_arg) {
                    keys.push(resource.db_session.key_arg.clone());
                }
            }
            keys
        },
        schema_state_fields: {
            let mut fields: Vec<String> = Vec::new();
            for resource in &resources {
                if !fields.contains(&resource.rust_db_field) {
                    fields.push(resource.rust_db_field.clone());
                }
            }
            fields
        },
        db_instances,
        cache_instances,
        has_object_store: !object_store_instances.is_empty(),
        has_email: !email_instances.is_empty(),
        has_search: !search_instances.is_empty(),
        has_external_http: !external_http_instances.is_empty(),
        object_store_instances,
        email_instances,
        search_instances,
        external_http_instances,
        has_queue: capability(NodeKind::Queue).is_some(),
        has_logging: capability(NodeKind::Logging).is_some(),
        has_metrics: capability(NodeKind::Metrics).is_some(),
        queue_engine: capability(NodeKind::Queue).map(|n| match n.component {
            Component::Queue {
                engine: QueueEngine::Nats,
                ..
            } => "nats".to_owned(),
            Component::Queue {
                engine: QueueEngine::Kafka,
                ..
            } => "kafka".to_owned(),
            _ => unreachable!("queue singleton is a queue"),
        }),
        events_subject: default_subject,
        records_use_datetime: field_types.iter().any(|t| t == "datetime"),
        records_use_json: field_types.iter().any(|t| t.contains("Any")),
        records_use_enum: field_types.iter().any(|t| t.starts_with("Literal")),
        records,
        tables,
        typed_handlers,
        apis,
        channels,
        workers,
        jobs,
        consumers,
        services,
        resources,
        call_targets,
        module,
    }
}

fn suffixed(base: &str, sep: &str, snake: &str, is_default: bool) -> String {
    if is_default {
        base.to_owned()
    } else {
        format!("{base}{sep}{snake}")
    }
}

fn instance_ctx(
    kind: NodeKind,
    module: &str,
    container_prefix: &str,
    name: &str,
    index: u32,
) -> InstanceCtx {
    let snake = name.to_snake_case();
    let is_default = name == "default";
    let (url_base, container_base, state_base) = match kind {
        NodeKind::Database => ("database_url", "db", "db"),
        NodeKind::Cache => ("redis_url", "cache", "cache"),
        _ => unreachable!("instances are built for db/cache only"),
    };
    let url_field = suffixed(url_base, "_", &snake, is_default);
    InstanceCtx {
        env_var: url_field.to_shouty_snake_case(),
        container: format!(
            "{container_prefix}{}",
            suffixed(container_base, "-", &name.to_kebab_case(), is_default)
        ),
        state_field: suffixed(state_base, "_", &snake, is_default),
        db_name: suffixed(module, "_", &snake, is_default),
        session_dep: suffixed("get_session", "_", &snake, is_default),
        session_param: suffixed("session", "_", &snake, is_default),
        key_arg: if is_default {
            String::new()
        } else {
            format!("\"{snake}\"")
        },
        redis_db: index,
        name: name.to_owned(),
        is_default,
        snake,
        url_field,
    }
}

/// A node is deliverable code of the scoped service (apis, workers,
/// handlers): unscoped builds take everything, scoped builds only the
/// service's own declarations.
fn owned_by(node_service: Option<ServiceId>, scope: Option<ServiceId>) -> bool {
    match scope {
        None => true,
        Some(id) => node_service == Some(id),
    }
}

/// A capability node is available to the scoped service: its own
/// declarations plus top-level (shared) ones.
fn infra_in_scope(node_service: Option<ServiceId>, scope: Option<ServiceId>) -> bool {
    match scope {
        None => true,
        Some(id) => node_service.is_none() || node_service == Some(id),
    }
}

fn instances_of(
    ir: &NormalizedIr,
    kind: NodeKind,
    module: &str,
    scope: Option<ServiceId>,
    container_prefix: &str,
) -> Vec<InstanceCtx> {
    ir.nodes_of_kind(kind)
        .filter(|node| infra_in_scope(node.service, scope))
        .enumerate()
        .map(|(index, node)| {
            instance_ctx(
                kind,
                module,
                container_prefix,
                node.component.name().unwrap_or("default"),
                index as u32,
            )
        })
        .collect()
}

fn cfg(field: String, py_ann: &str, py_default: &str, rust_default: &str) -> CfgFieldCtx {
    CfgFieldCtx {
        env: field.to_shouty_snake_case(),
        py_ann: py_ann.to_owned(),
        py_default: py_default.to_owned(),
        rust_default: rust_default.to_owned(),
        compose_value: None,
        field,
    }
}

fn ontology_instance(
    node: &ciac_ir::Node,
    container_prefix: &str,
    index: usize,
) -> OntologyInstanceCtx {
    let name = node.component.name().unwrap_or("default").to_owned();
    let snake = name.to_snake_case();
    let is_default = name == "default";
    let sfx = |base: &str| suffixed(base, "_", &snake, is_default);
    let container = |base: &str| {
        format!(
            "{container_prefix}{}",
            suffixed(base, "-", &name.to_kebab_case(), is_default)
        )
    };
    match &node.component {
        Component::ObjectStore {
            provider, bucket, ..
        } => {
            let bucket_name = bucket.clone().unwrap_or_else(|| snake.clone());
            let mut fields = vec![
                cfg(
                    sfx("s3_endpoint"),
                    "str",
                    "\"http://localhost:9000\"",
                    "http://localhost:9000",
                ),
                cfg(sfx("s3_access_key"), "str", "\"minioadmin\"", "minioadmin"),
                cfg(sfx("s3_secret_key"), "str", "\"minioadmin\"", "minioadmin"),
                cfg(
                    sfx("s3_bucket"),
                    "str",
                    &format!("\"{bucket_name}\""),
                    &bucket_name,
                ),
                cfg(sfx("s3_region"), "str", "\"us-east-1\"", "us-east-1"),
            ];
            fields[0].compose_value = Some(format!("http://{}:9000", container("minio")));
            OntologyInstanceCtx {
                kind: "object_store".to_owned(),
                provider: Some(format!("{provider:?}")),
                container: Some(container("minio")),
                state_field: sfx("object_store"),
                key_arg: key_arg(&snake, is_default),
                bucket: Some(bucket_name),
                host_port: None,
                cfg: fields,
                name,
                snake,
                is_default,
            }
        }
        Component::Email { provider, .. } => {
            let mut fields = vec![
                cfg(sfx("smtp_host"), "str", "\"localhost\"", "localhost"),
                cfg(sfx("smtp_port"), "int", "1025", "1025"),
                cfg(sfx("smtp_username"), "str", "\"\"", ""),
                cfg(sfx("smtp_password"), "str", "\"\"", ""),
                cfg(
                    sfx("smtp_from"),
                    "str",
                    "\"noreply@example.com\"",
                    "noreply@example.com",
                ),
                cfg(sfx("smtp_use_tls"), "bool", "False", "false"),
            ];
            fields[0].compose_value = Some(container("mailpit"));
            OntologyInstanceCtx {
                kind: "email".to_owned(),
                provider: Some(format!("{provider:?}")),
                container: Some(container("mailpit")),
                state_field: sfx("email"),
                key_arg: key_arg(&snake, is_default),
                bucket: None,
                host_port: Some(8025 + index as u16),
                cfg: fields,
                name,
                snake,
                is_default,
            }
        }
        Component::Search { provider, .. } => {
            let mut fields = vec![cfg(
                sfx("search_url"),
                "str",
                "\"http://localhost:9200\"",
                "http://localhost:9200",
            )];
            fields[0].compose_value = Some(format!("http://{}:9200", container("search")));
            OntologyInstanceCtx {
                kind: "search".to_owned(),
                provider: Some(format!("{provider:?}")),
                container: Some(container("search")),
                state_field: sfx("search"),
                key_arg: key_arg(&snake, is_default),
                bucket: None,
                host_port: None,
                cfg: fields,
                name,
                snake,
                is_default,
            }
        }
        Component::ExternalHttp { base_url, .. } => OntologyInstanceCtx {
            kind: "external_http".to_owned(),
            provider: None,
            container: None,
            state_field: format!("http_{snake}"),
            key_arg: format!("\"{snake}\""),
            bucket: None,
            host_port: None,
            cfg: vec![cfg(
                format!("http_base_url_{snake}"),
                "str",
                &format!("\"{base_url}\""),
                base_url,
            )],
            name,
            snake,
            is_default,
        },
        other => unreachable!("not an ontology capability: {other:?}"),
    }
}

fn key_arg(snake: &str, is_default: bool) -> String {
    if is_default {
        String::new()
    } else {
        format!("\"{snake}\"")
    }
}

fn ontology_instances(
    ir: &NormalizedIr,
    kind: NodeKind,
    scope: Option<ServiceId>,
    container_prefix: &str,
) -> Vec<OntologyInstanceCtx> {
    ir.nodes_of_kind(kind)
        .filter(|node| infra_in_scope(node.service, scope))
        .enumerate()
        .map(|(index, node)| ontology_instance(node, container_prefix, index))
        .collect()
}

fn extra_dep(binding: &BindingCtx) -> Option<ExtraDepCtx> {
    let is_default = binding.name == "default";
    let (param, py_type, py_module, py_getter, rust_type, rust_module, state_base) =
        match binding.kind.as_str() {
            "object_store" => (
                "object_store",
                "ObjectStore",
                "object_store",
                "get_object_store",
                "ObjectStore",
                "object_store",
                "object_store",
            ),
            "email" => (
                "email",
                "Email",
                "email",
                "get_email",
                "Email",
                "email",
                "email",
            ),
            "search" => (
                "search",
                "Search",
                "search",
                "get_search",
                "Search",
                "search",
                "search",
            ),
            "external_http" => (
                "http",
                "httpx.AsyncClient",
                "http_clients",
                "get_http_client",
                "ExternalHttp",
                "http_clients",
                "http",
            ),
            _ => return None,
        };
    let key = if binding.kind == "external_http" {
        format!("\"{}\"", binding.snake)
    } else {
        key_arg(&binding.snake, is_default)
    };
    let state_field = if binding.kind == "external_http" {
        format!("http_{}", binding.snake)
    } else {
        suffixed(state_base, "_", &binding.snake, is_default)
    };
    Some(ExtraDepCtx {
        kind: binding.kind.clone(),
        param: param.to_owned(),
        py_type: py_type.to_owned(),
        py_module: py_module.to_owned(),
        py_expr: format!("{py_getter}({key})"),
        py_getter: py_getter.to_owned(),
        rust_type: rust_type.to_owned(),
        rust_module: rust_module.to_owned(),
        rust_state_field: state_field,
    })
}

pub fn extras_of(bindings: &[BindingCtx]) -> Vec<ExtraDepCtx> {
    bindings.iter().filter_map(extra_dep).collect()
}

fn extra_imports_of(handlers: &[HandlerRef]) -> Vec<ExtraImportCtx> {
    let mut imports: Vec<ExtraImportCtx> = Vec::new();
    for handler in handlers {
        for extra in &handler.extras {
            let import = ExtraImportCtx {
                py_module: extra.py_module.clone(),
                py_getter: extra.py_getter.clone(),
            };
            if !imports.contains(&import) {
                imports.push(import);
            }
        }
    }
    imports
}

/// Resolved capability access for a handler/resource, derived from its
/// binding edges.
#[derive(Debug)]
pub struct Access {
    pub db: Option<SessionCtx>,
    pub cache_expr: Option<String>,
    pub rust_db_field: Option<String>,
    pub rust_cache_field: Option<String>,
}

pub fn access_of(bindings: &[BindingCtx]) -> Access {
    let find = |kind: &str| bindings.iter().find(|b| b.kind == kind);
    let db = find("db").map(|b| {
        let is_default = b.name == "default";
        SessionCtx {
            param: suffixed("session", "_", &b.snake, is_default),
            dep: suffixed("get_session", "_", &b.snake, is_default),
            key_arg: if is_default {
                String::new()
            } else {
                format!("\"{}\"", b.snake)
            },
        }
    });
    let cache = find("cache");
    Access {
        rust_db_field: find("db").map(|b| suffixed("db", "_", &b.snake, b.name == "default")),
        rust_cache_field: cache.map(|b| suffixed("cache", "_", &b.snake, b.name == "default")),
        cache_expr: cache.map(|b| {
            if b.name == "default" {
                "get_cache()".to_owned()
            } else {
                format!("get_cache(\"{}\")", b.snake)
            }
        }),
        db,
    }
}

/// Distinct sessions used by a set of handlers, in first-use order.
fn sessions_of(handlers: &[HandlerRef]) -> Vec<SessionCtx> {
    let mut sessions: Vec<SessionCtx> = Vec::new();
    for handler in handlers {
        if let Some(session) = &handler.db_session {
            if !sessions.contains(session) {
                sessions.push(session.clone());
            }
        }
    }
    sessions
}

fn steps_of(ir: &NormalizedIr, owner: NodeId) -> (Vec<StepCtx>, Vec<HandlerRef>) {
    let mut handlers = Vec::new();
    let steps = ir
        .pipeline_of(owner)
        .map(|pipeline| step_ctxs(ir, &pipeline.steps, pipeline.payload, &mut handlers))
        .unwrap_or_default();
    (steps, handlers)
}

fn step_ctxs(
    ir: &NormalizedIr,
    steps: &[Step],
    payload: Option<RecordId>,
    handlers: &mut Vec<HandlerRef>,
) -> Vec<StepCtx> {
    steps
        .iter()
        .map(|step| match &step.kind {
            StepKind::Auth { .. } => StepCtx {
                kind: "auth",
                handler: None,
                subject: None,
                call: None,
                field: None,
                arms: Vec::new(),
            },
            StepKind::Publish { stream } => StepCtx {
                kind: "publish",
                handler: None,
                subject: Some(stream_subject(ir, *stream)),
                call: None,
                field: None,
                arms: Vec::new(),
            },
            StepKind::Return => StepCtx {
                kind: "return",
                handler: None,
                subject: None,
                call: None,
                field: None,
                arms: Vec::new(),
            },
            StepKind::Call { target } => StepCtx {
                kind: "call",
                handler: None,
                subject: None,
                call: Some(call_ctx(ir, *target)),
                field: None,
                arms: Vec::new(),
            },
            StepKind::Handler { node } => {
                let handler = handler_ref(ir, *node);
                if !handlers.contains(&handler) {
                    handlers.push(handler.clone());
                }
                StepCtx {
                    kind: "handler",
                    handler: Some(handler),
                    subject: None,
                    call: None,
                    field: None,
                    arms: Vec::new(),
                }
            }
            StepKind::Match { field, arms } => StepCtx {
                kind: "match",
                handler: None,
                subject: None,
                call: None,
                field: Some(field.clone()),
                arms: arms
                    .iter()
                    .map(|arm| ArmCtx {
                        label: arm.label.clone(),
                        rust_variant: arm
                            .label
                            .as_ref()
                            .and_then(|label| rust_variant(ir, payload, field, label)),
                        steps: step_ctxs(ir, &arm.steps, payload, handlers),
                    })
                    .collect(),
            },
        })
        .collect()
}

fn handler_ref(ir: &NormalizedIr, id: NodeId) -> HandlerRef {
    let node = ir.node(id);
    let name = node.component.name().unwrap_or_default().to_owned();
    // A v0.7 typed handler has no `DataFlow` binding edges — its
    // capability usage lives in the HIR's `VerbCall`s instead. An inline
    // body lives under `app/logic/` (compiler-owned); `extern` gets a
    // seeded stub under `app/services/`, same as classic handlers.
    let (bindings, py_package) = match &node.component {
        Component::Service {
            signature: Some(hir),
            ..
        } => (
            hir_bindings(ir, hir),
            if hir.body.is_some() {
                "logic"
            } else {
                "services"
            },
        ),
        _ => (bindings_of(ir, id), "services"),
    };
    let access = access_of(&bindings);
    let extras = extras_of(&bindings);
    let mut args = Vec::new();
    if let Some(session) = &access.db {
        args.push(format!("session={}", session.param));
    }
    if let Some(cache_expr) = &access.cache_expr {
        args.push(format!("cache={cache_expr}"));
    }
    for extra in &extras {
        args.push(format!("{}={}", extra.param, extra.py_expr));
    }
    HandlerRef {
        module: name.to_snake_case(),
        py_package,
        class_name: name,
        needs_db: access.db.is_some(),
        needs_cache: access.cache_expr.is_some(),
        db_session: access.db,
        rust_db_field: access.rust_db_field,
        rust_cache_field: access.rust_cache_field,
        extras,
        bindings,
        py_args: args.join(", "),
    }
}

fn stream_subject(ir: &NormalizedIr, id: NodeId) -> String {
    match &ir.node(id).component {
        Component::Stream { subject, .. } => subject.clone(),
        other => unreachable!("publish target is a stream, found {other:?}"),
    }
}

fn call_ctx(ir: &NormalizedIr, target: NodeId) -> CallCtx {
    let node = ir.node(target);
    let service = node
        .service
        .map(|id| ir.service(id).name.clone())
        .unwrap_or_else(|| "Project".to_owned());
    let api = node.component.name().unwrap_or_default().to_owned();
    CallCtx {
        module: service.to_snake_case(),
        class_name: format!("{}Client", service.to_pascal_case()),
        method: api.to_snake_case(),
        service,
        api,
    }
}

/// Call targets reachable from a step list, in invocation order.
fn call_nodes(steps: &[Step]) -> Vec<NodeId> {
    let mut nodes = Vec::new();
    for step in steps {
        match &step.kind {
            StepKind::Call { target } => nodes.push(*target),
            StepKind::Match { arms, .. } => {
                for arm in arms {
                    nodes.extend(call_nodes(&arm.steps));
                }
            }
            StepKind::Auth { .. }
            | StepKind::Publish { .. }
            | StepKind::Return
            | StepKind::Handler { .. } => {}
        }
    }
    nodes
}

/// Deduplicated client references a route/worker imports, in invocation
/// order.
fn call_imports_of(steps: &[StepCtx]) -> Vec<CallCtx> {
    fn walk(steps: &[StepCtx], imports: &mut Vec<CallCtx>) {
        for step in steps {
            if let Some(call) = &step.call {
                if !imports.iter().any(|c| c.class_name == call.class_name) {
                    imports.push(call.clone());
                }
            }
            for arm in &step.arms {
                walk(&arm.steps, imports);
            }
        }
    }
    let mut imports = Vec::new();
    walk(steps, &mut imports);
    imports
}

fn handler_nodes(steps: &[Step]) -> Vec<NodeId> {
    let mut nodes = Vec::new();
    for step in steps {
        match &step.kind {
            StepKind::Handler { node } => nodes.push(*node),
            StepKind::Match { arms, .. } => {
                for arm in arms {
                    nodes.extend(handler_nodes(&arm.steps));
                }
            }
            StepKind::Auth { .. }
            | StepKind::Publish { .. }
            | StepKind::Return
            | StepKind::Call { .. } => {}
        }
    }
    nodes
}

fn has_publish(steps: &[StepCtx]) -> bool {
    steps.iter().any(|step| {
        step.kind == "publish"
            || step
                .arms
                .iter()
                .any(|arm| has_publish(arm.steps.as_slice()))
    })
}

/// Translates a sema-validated five-field cron expression (minute hour day
/// month weekday, weekday `0`-`7` with both `0` and `7` meaning Sunday) into
/// a six-field expression the Rust `cron` crate accepts (leading seconds
/// field, weekday `1`-`7` only). The weekday field is expanded into an
/// explicit list rather than reusing ranges/steps, because the 0-6 and 1-7
/// numbering conventions disagree on which values are contiguous (e.g.
/// `0-3` is Sun-Wed in the source convention but not a valid contiguous
/// range once `0` becomes `7`).
pub fn cron_crate_schedule(schedule: &str) -> String {
    let fields: Vec<&str> = schedule.split_whitespace().collect();
    debug_assert_eq!(fields.len(), 5, "sema guarantees a five-field schedule");
    let weekday = fields.get(4).copied().unwrap_or("*");
    let translated_weekday = translate_weekday_field(weekday);
    format!(
        "0 {} {} {} {} {}",
        fields.first().copied().unwrap_or("*"),
        fields.get(1).copied().unwrap_or("*"),
        fields.get(2).copied().unwrap_or("*"),
        fields.get(3).copied().unwrap_or("*"),
        translated_weekday,
    )
}

fn translate_weekday_field(field: &str) -> String {
    if field == "*" {
        return "*".to_owned();
    }
    let days: Vec<u32> = field
        .split(',')
        .flat_map(weekday_part_values)
        .map(|day| if day == 0 { 7 } else { day })
        .collect();
    days.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

/// Expands one comma-separated weekday token (`*`, `n`, `a-b`, or `x/n`,
/// optionally combined as `a-b/n`) into the individual day numbers (0-7)
/// it selects, using the source 0-6 numbering.
fn weekday_part_values(part: &str) -> Vec<u32> {
    let (base, step) = match part.split_once('/') {
        Some((base, step)) => (base, step.parse::<u32>().ok().filter(|s| *s > 0)),
        None => (part, None),
    };
    let (start, end) = if base == "*" {
        (0, 7)
    } else if let Some((start, end)) = base.split_once('-') {
        match (start.parse::<u32>(), end.parse::<u32>()) {
            (Ok(start), Ok(end)) if start <= end => (start, end),
            _ => return Vec::new(),
        }
    } else {
        match base.parse::<u32>() {
            // A bare `N/step` (no explicit range) steps from N to the
            // field's max, same as cron's `N/step` convention.
            Ok(day) if step.is_some() => (day, 7),
            Ok(day) => (day, day),
            Err(_) => return Vec::new(),
        }
    };
    let step = step.unwrap_or(1);
    (start..=end).step_by(step as usize).collect()
}

fn rust_variant(
    ir: &NormalizedIr,
    payload: Option<RecordId>,
    field: &str,
    label: &str,
) -> Option<String> {
    let record = ir.record(payload?);
    let field = record.fields.iter().find(|f| f.name == field)?;
    if !matches!(field.ty, FieldType::Enum { .. }) {
        return None;
    }
    Some(format!(
        "crate::schemas::{}{}::{label}",
        record.name,
        field.name.to_pascal_case()
    ))
}

pub fn build_record(ir: &NormalizedIr, id: RecordId) -> RecordCtx {
    let record = ir.record(id);
    let mut fields = Vec::new();
    let mut enums = Vec::new();
    for field in &record.fields {
        let (py_type, rust_type, sql_type) = match &field.ty {
            FieldType::Str => ("str".to_owned(), "String".to_owned(), "TEXT"),
            FieldType::Int => ("int".to_owned(), "i64".to_owned(), "BIGINT"),
            FieldType::Float => ("float".to_owned(), "f64".to_owned(), "DOUBLE PRECISION"),
            FieldType::Bool => ("bool".to_owned(), "bool".to_owned(), "BOOLEAN"),
            FieldType::Uuid => ("str".to_owned(), "String".to_owned(), "TEXT"),
            FieldType::Timestamp => (
                "datetime".to_owned(),
                "chrono::DateTime<chrono::Utc>".to_owned(),
                "TIMESTAMPTZ",
            ),
            FieldType::Json => (
                "dict[str, Any]".to_owned(),
                "serde_json::Value".to_owned(),
                "JSONB",
            ),
            FieldType::Enum { variants } => {
                let enum_name = format!("{}{}", record.name, field.name.to_pascal_case());
                let literal = variants
                    .iter()
                    .map(|v| format!("\"{v}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                enums.push(EnumCtx {
                    name: enum_name.clone(),
                    variants: variants.clone(),
                });
                (format!("Literal[{literal}]"), enum_name, "TEXT")
            }
        };
        let is_enum = matches!(field.ty, FieldType::Enum { .. });
        fields.push(FieldCtx {
            name: field.name.clone(),
            is_json: matches!(field.ty, FieldType::Json),
            py_out_type: if is_enum {
                "str".to_owned()
            } else {
                py_type.clone()
            },
            db_rust_type: if is_enum {
                "String".to_owned()
            } else {
                rust_type.clone()
            },
            is_enum,
            py_type,
            rust_type,
            sql_type: sql_type.to_owned(),
        });
    }
    let non_id: Vec<&str> = fields
        .iter()
        .filter(|f| f.name != "id")
        .map(|f| f.name.as_str())
        .collect();
    let select_cols = std::iter::once("id")
        .chain(non_id.iter().copied())
        .collect::<Vec<_>>()
        .join(", ");
    let insert_placeholders = (1..=non_id.len() + 1)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let update_assignments = non_id
        .iter()
        .enumerate()
        .map(|(i, name)| format!("{name} = ${}", i + 2))
        .collect::<Vec<_>>()
        .join(", ");
    RecordCtx {
        name: record.name.clone(),
        snake: record.name.to_snake_case(),
        has_id: fields.iter().any(|f| f.name == "id"),
        is_error: record.kind == ciac_ir::RecordKind::Error,
        select_cols,
        insert_placeholders,
        update_assignments,
        fields,
        enums,
    }
}

fn bindings_of(ir: &NormalizedIr, node: NodeId) -> Vec<BindingCtx> {
    ir.edges_from(node)
        .filter(|e| e.kind == EdgeKind::DataFlow)
        .filter_map(|e| binding_ctx_for(&ir.node(e.to).component))
        .collect()
}

fn binding_ctx_for(component: &Component) -> Option<BindingCtx> {
    let kind = binding_kind(component.kind())?;
    let name = component.name()?.to_owned();
    let snake = name.to_snake_case();
    Some(BindingCtx {
        py_attr: format!("{}_{}", kind, snake),
        rust_field: format!("{}_{}", kind, snake),
        kind: kind.to_owned(),
        name,
        snake,
    })
}

/// The same shape as [`bindings_of`], but for a v0.7 typed handler: since
/// the HIR has no `DataFlow` edges (verb calls resolve straight to a
/// capability instance node during type-checking), the bindings are
/// derived by walking the body for `VerbCall`s instead of the graph for
/// edges. Feeding the result through [`access_of`]/[`extras_of`] as usual
/// means typed and classic handlers share every downstream naming rule.
pub fn hir_bindings(ir: &NormalizedIr, body: &ciac_ir::HandlerBody) -> Vec<BindingCtx> {
    body.capability_nodes()
        .iter()
        .filter_map(|id| binding_ctx_for(&ir.node(*id).component))
        .collect()
}

fn binding_kind(kind: NodeKind) -> Option<&'static str> {
    Some(match kind {
        NodeKind::Database => "db",
        NodeKind::Cache => "cache",
        NodeKind::Queue => "queue",
        NodeKind::Auth => "auth",
        NodeKind::ObjectStore => "object_store",
        NodeKind::Email => "email",
        NodeKind::Search => "search",
        NodeKind::ExternalHttp => "external_http",
        NodeKind::Scheduler => "scheduler",
        NodeKind::Realtime => "realtime",
        NodeKind::Logging => "logging",
        NodeKind::Metrics => "metrics",
        NodeKind::Api
        | NodeKind::Service
        | NodeKind::Worker
        | NodeKind::Job
        | NodeKind::Channel
        | NodeKind::Stream => return None,
    })
}

/// Constant-style name for a subject, e.g. `media.uploaded` → `UPLOADED`.
pub fn subject_const(subject: &str) -> String {
    subject
        .rsplit('.')
        .next()
        .unwrap_or(subject)
        .to_shouty_snake_case()
}

#[cfg(test)]
mod cron_crate_schedule_tests {
    use super::cron_crate_schedule;

    #[test]
    fn passes_through_non_weekday_fields_with_a_seconds_prefix() {
        assert_eq!(cron_crate_schedule("0 3 * * *"), "0 0 3 * * *");
        assert_eq!(cron_crate_schedule("*/5 * * * *"), "0 */5 * * * *");
    }

    #[test]
    fn rewrites_bare_sunday_zero_to_seven() {
        assert_eq!(cron_crate_schedule("0 0 * * 0"), "0 0 0 * * 7");
    }

    #[test]
    fn leaves_seven_and_wildcard_weekday_untouched() {
        assert_eq!(cron_crate_schedule("0 0 * * 7"), "0 0 0 * * 7");
        assert_eq!(cron_crate_schedule("0 0 * * *"), "0 0 0 * * *");
    }

    #[test]
    fn expands_a_range_that_crosses_the_sunday_boundary() {
        // Source "0-3" means Sun,Mon,Tue,Wed; a naive 0->7 rewrite would
        // produce the invalid/reversed range "7-3", so it must expand.
        assert_eq!(cron_crate_schedule("0 0 * * 0-3"), "0 0 0 * * 7,1,2,3");
    }

    #[test]
    fn expands_a_list_containing_sunday() {
        assert_eq!(cron_crate_schedule("0 0 * * 0,2,4"), "0 0 0 * * 7,2,4");
    }

    #[test]
    fn expands_a_step_starting_at_sunday() {
        assert_eq!(cron_crate_schedule("0 0 * * 0/2"), "0 0 0 * * 7,2,4,6");
    }
}
