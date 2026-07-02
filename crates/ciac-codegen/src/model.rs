//! Language-neutral template model built from the validated IR.
//!
//! Everything backend templates need is precomputed here as plain
//! serializable data: casing variants, per-pipeline step lists, and which
//! capabilities each generated unit must have injected. Backends share
//! this model so targets stay structurally comparable; templates stay
//! purely presentational.

use crate::GenOptions;
use ciac_ir::{EdgeKind, NodeId, NodeKind, NormalizedIr, QueueEngine, Step};
use heck::{ToKebabCase, ToSnakeCase};
use serde::Serialize;
use std::collections::BTreeSet;

/// Root template context for the whole project.
#[derive(Debug, Serialize)]
pub struct Ctx {
    /// Original service name, e.g. `VideoPlatform`.
    pub service_name: String,
    /// Package name, e.g. `video-platform`.
    pub package: String,
    /// Python module prefix, e.g. `video_platform`.
    pub module: String,
    pub has_auth: bool,
    pub has_db: bool,
    pub has_cache: bool,
    pub has_queue: bool,
    pub has_logging: bool,
    pub has_metrics: bool,
    pub queue_engine: Option<String>,
    /// Subject all `Queue` steps publish to and pipeline workers consume.
    pub events_subject: String,
    pub apis: Vec<ApiCtx>,
    pub workers: Vec<WorkerCtx>,
    pub consumers: Vec<ConsumerCtx>,
    pub services: Vec<ServiceCtx>,
    pub resources: Vec<ResourceCtx>,
}

/// An api with a request pipeline.
#[derive(Debug, Serialize)]
pub struct ApiCtx {
    pub name: String,
    pub snake: String,
    /// Route path, e.g. `/upload`.
    pub route: String,
    pub steps: Vec<StepCtx>,
    pub has_auth_step: bool,
    pub has_queue_step: bool,
    /// Whether any invoked handler needs a database session / cache client.
    pub needs_db: bool,
    pub needs_cache: bool,
    /// Deduplicated handlers, in invocation order, for imports.
    pub handlers: Vec<HandlerRef>,
}

#[derive(Debug, Serialize)]
pub struct StepCtx {
    /// One of `auth`, `handler`, `queue`, `return`.
    pub kind: &'static str,
    pub handler: Option<HandlerRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HandlerRef {
    /// Class name, e.g. `StoreVideo`.
    pub class_name: String,
    /// Module name, e.g. `store_video`.
    pub module: String,
    pub needs_db: bool,
    pub needs_cache: bool,
}

/// A declared worker with (or without) a processing pipeline.
#[derive(Debug, Serialize)]
pub struct WorkerCtx {
    pub name: String,
    pub snake: String,
    /// NATS queue group so replicas of this worker load-balance.
    pub queue_group: String,
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
    pub needs_db: bool,
    pub needs_cache: bool,
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
    pub has_auth: bool,
    pub has_cache: bool,
}

pub fn build(ir: &NormalizedIr, opts: &GenOptions) -> Ctx {
    let package = crate::project_name(ir, opts);
    let module = ir.name.to_snake_case();
    let has_db = ir.singleton(NodeKind::Database).is_some();
    let has_cache = ir.singleton(NodeKind::Cache).is_some();

    let resource_services: BTreeSet<NodeId> = ir.resources.iter().map(|r| r.service).collect();
    let consumer_workers: BTreeSet<NodeId> = ir.event_streams.iter().map(|s| s.worker).collect();

    let handler_ref = |id: NodeId| -> HandlerRef {
        let node = ir.node(id);
        let name = node.component.name().unwrap_or_default().to_owned();
        HandlerRef {
            module: name.to_snake_case(),
            class_name: name,
            needs_db: touches(ir, id, NodeKind::Database),
            needs_cache: touches(ir, id, NodeKind::Cache),
        }
    };

    let steps_of = |owner: NodeId| -> (Vec<StepCtx>, Vec<HandlerRef>) {
        let mut steps = Vec::new();
        let mut handlers: Vec<HandlerRef> = Vec::new();
        if let Some(pipeline) = ir.pipeline_of(owner) {
            for step in &pipeline.steps {
                let ctx = match step {
                    Step::Auth { .. } => StepCtx {
                        kind: "auth",
                        handler: None,
                    },
                    Step::Queue { .. } => StepCtx {
                        kind: "queue",
                        handler: None,
                    },
                    Step::Return => StepCtx {
                        kind: "return",
                        handler: None,
                    },
                    Step::Handler { node } => {
                        let handler = handler_ref(*node);
                        if !handlers.contains(&handler) {
                            handlers.push(handler.clone());
                        }
                        StepCtx {
                            kind: "handler",
                            handler: Some(handler),
                        }
                    }
                };
                steps.push(ctx);
            }
        }
        (steps, handlers)
    };

    let apis = ir
        .nodes_of_kind(NodeKind::Api)
        .filter(|api| ir.pipeline_of(api.id).is_some())
        .map(|api| {
            let name = api.component.name().unwrap_or_default().to_owned();
            let (steps, handlers) = steps_of(api.id);
            ApiCtx {
                route: format!("/{}", name.to_kebab_case()),
                snake: name.to_snake_case(),
                has_auth_step: steps.iter().any(|s| s.kind == "auth"),
                has_queue_step: steps.iter().any(|s| s.kind == "queue"),
                needs_db: handlers.iter().any(|h| h.needs_db),
                needs_cache: handlers.iter().any(|h| h.needs_cache),
                steps,
                handlers,
                name,
            }
        })
        .collect();

    let workers = ir
        .nodes_of_kind(NodeKind::Worker)
        .filter(|w| !consumer_workers.contains(&w.id))
        .map(|worker| {
            let name = worker.component.name().unwrap_or_default().to_owned();
            let (_, handlers) = steps_of(worker.id);
            WorkerCtx {
                snake: name.to_snake_case(),
                queue_group: name.to_snake_case(),
                needs_db: handlers.iter().any(|h| h.needs_db),
                needs_cache: handlers.iter().any(|h| h.needs_cache),
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
                subject: format!("{module}.{}", stream.subject),
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
                needs_db: touches(ir, service.id, NodeKind::Database),
                needs_cache: touches(ir, service.id, NodeKind::Cache),
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
                has_auth: ir.singleton(NodeKind::Auth).is_some(),
                has_cache,
                snake,
            }
        })
        .collect();

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
            ciac_ir::Component::Queue {
                engine: QueueEngine::Nats,
            } => "nats".to_owned(),
            ciac_ir::Component::Queue {
                engine: QueueEngine::Kafka,
            } => "kafka".to_owned(),
            _ => unreachable!("queue singleton is a queue"),
        }),
        events_subject: format!("{module}.events"),
        apis,
        workers,
        consumers,
        services,
        resources,
        module,
    }
}

/// Whether `node` has a data-flow edge to the (singleton) component of the
/// given kind — i.e. codegen must inject that client into the handler.
fn touches(ir: &NormalizedIr, node: NodeId, kind: NodeKind) -> bool {
    ir.edges_from(node)
        .filter(|e| e.kind == EdgeKind::DataFlow)
        .any(|e| ir.node(e.to).component.kind() == kind)
}
