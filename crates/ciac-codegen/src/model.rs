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
    RecordId, Step, StepKind,
};
use heck::{ToKebabCase, ToPascalCase, ToShoutySnakeCase, ToSnakeCase};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// Root template context for the whole project.
#[derive(Debug, Serialize)]
pub struct Ctx {
    /// Original service name, e.g. `VideoPlatform`.
    pub service_name: String,
    /// Package name, e.g. `video-platform`.
    pub package: String,
    /// Module/crate prefix, e.g. `video_platform`.
    pub module: String,
    pub has_auth: bool,
    pub has_db: bool,
    pub has_cache: bool,
    pub has_queue: bool,
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
    pub workers: Vec<WorkerCtx>,
    pub consumers: Vec<ConsumerCtx>,
    pub services: Vec<ServiceCtx>,
    pub resources: Vec<ResourceCtx>,
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
    /// Deduplicated handlers, in invocation order, for imports.
    pub handlers: Vec<HandlerRef>,
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
    pub needs_db: bool,
    pub needs_cache: bool,
    pub bindings: Vec<BindingCtx>,
    /// Precomputed Python constructor arguments for invoking this handler
    /// from a route/worker, e.g. `session=session, cache=get_cache()`.
    /// Keeps templates free of whitespace gymnastics.
    pub py_args: String,
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
}

pub fn build(ir: &NormalizedIr, opts: &GenOptions) -> Ctx {
    let package = crate::project_name(ir, opts);
    let module = ir.name.to_snake_case();
    let has_db = ir.singleton(NodeKind::Database).is_some();
    let has_cache = ir.singleton(NodeKind::Cache).is_some();

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
    for pipeline in &ir.pipelines {
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
        .filter(|api| ir.pipeline_of(api.id).is_some())
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
            ApiCtx {
                route: path,
                method_upper,
                method_lower,
                scope,
                has_body,
                snake: name.to_snake_case(),
                has_auth_step: steps.iter().any(|s| s.kind == "auth"),
                has_publish_step: has_publish(&steps),
                needs_db: handlers.iter().any(|h| h.needs_db),
                needs_cache: handlers.iter().any(|h| h.needs_cache),
                payload,
                steps,
                handlers,
                name,
            }
        })
        .collect();

    let default_subject = format!("{module}.events");
    let workers = ir
        .nodes_of_kind(NodeKind::Worker)
        .filter(|w| !consumer_workers.contains(&w.id))
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
            WorkerCtx {
                snake: name.to_snake_case(),
                queue_group: name.to_snake_case(),
                needs_db: handlers.iter().any(|h| h.needs_db),
                needs_cache: handlers.iter().any(|h| h.needs_cache),
                has_publish_step: has_publish(&steps),
                concurrency: config.concurrency,
                max_retries: config.max_retries,
                subject,
                payload,
                steps,
                handlers,
                name,
            }
        })
        .collect();

    let consumers = ir
        .event_streams
        .iter()
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

    let services = ir
        .nodes_of_kind(NodeKind::Service)
        .filter(|s| !resource_services.contains(&s.id))
        .map(|service| {
            let name = service.component.name().unwrap_or_default().to_owned();
            ServiceCtx {
                module: name.to_snake_case(),
                payload: payload_ref(handler_payloads.get(&service.id).copied().flatten()),
                needs_db: touches(ir, service.id, NodeKind::Database),
                needs_cache: touches(ir, service.id, NodeKind::Cache),
                bindings: bindings_of(ir, service.id),
                class_name: name,
            }
        })
        .collect();

    let resources = ir
        .resources
        .iter()
        .map(|resource| {
            let snake = resource.name.to_snake_case();
            ResourceCtx {
                name: resource.name.clone(),
                plural: format!("{snake}s"),
                store_class: format!("{}Store", resource.name),
                store_module: format!("{snake}_store"),
                record: resource.record.map(record_ctx),
                has_auth: ir.singleton(NodeKind::Auth).is_some(),
                has_cache,
                cache_ttl: resource.config.cache_ttl,
                page_size: resource.config.page_size,
                snake,
            }
        })
        .collect();

    let records: Vec<RecordCtx> = ir.records().map(|(id, _)| build_record(ir, id)).collect();
    let all_fields = |records: &[RecordCtx]| -> Vec<String> {
        records
            .iter()
            .flat_map(|r| r.fields.iter().map(|f| f.py_type.clone()))
            .collect()
    };
    let field_types = all_fields(&records);

    Ctx {
        service_name: ir.name.clone(),
        package,
        has_auth: ir.singleton(NodeKind::Auth).is_some(),
        has_db,
        has_cache,
        has_queue: ir.singleton(NodeKind::Queue).is_some(),
        has_logging: ir.singleton(NodeKind::Logging).is_some(),
        has_metrics: ir.singleton(NodeKind::Metrics).is_some(),
        queue_engine: ir.singleton(NodeKind::Queue).map(|n| match n.component {
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
        apis,
        workers,
        consumers,
        services,
        resources,
        module,
    }
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
    let needs_db = touches(ir, id, NodeKind::Database);
    let needs_cache = touches(ir, id, NodeKind::Cache);
    let mut args = Vec::new();
    if needs_db {
        args.push("session=session".to_owned());
    }
    if needs_cache {
        args.push("cache=get_cache()".to_owned());
    }
    HandlerRef {
        module: name.to_snake_case(),
        class_name: name,
        needs_db,
        needs_cache,
        bindings: bindings_of(ir, id),
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
    CallCtx {
        service,
        api: node.component.name().unwrap_or_default().to_owned(),
    }
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

fn build_record(ir: &NormalizedIr, id: RecordId) -> RecordCtx {
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
        select_cols,
        insert_placeholders,
        update_assignments,
        fields,
        enums,
    }
}

/// Whether `node` has a data-flow edge to the (singleton) component of the
/// given kind — i.e. codegen must inject that client into the handler.
fn touches(ir: &NormalizedIr, node: NodeId, kind: NodeKind) -> bool {
    ir.edges_from(node)
        .filter(|e| e.kind == EdgeKind::DataFlow)
        .any(|e| ir.node(e.to).component.kind() == kind)
}

fn bindings_of(ir: &NormalizedIr, node: NodeId) -> Vec<BindingCtx> {
    ir.edges_from(node)
        .filter(|e| e.kind == EdgeKind::DataFlow)
        .filter_map(|e| {
            let component = &ir.node(e.to).component;
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
        })
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
        NodeKind::Api | NodeKind::Service | NodeKind::Worker | NodeKind::Stream => return None,
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
