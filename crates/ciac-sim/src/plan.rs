//! `SimPlan` (17UpdatePlan.md Pillar 2): the target-neutral description
//! of a program's simulatable topology,
//! derived from [`ciac_ir::NormalizedIr`] alone. `ciac-sim` depends on
//! normalized IR, not on Python or Rust — a target adapter (M3/M9)
//! consumes a `SimPlan`, it never builds one itself.
//!
//! Scope is deliberately narrow for M2, per the Rollout strategy's
//! checkpoint-first restructuring: only the slice the M5 checkpoint
//! needs (tables, streams, workers, jobs) is modeled now. Routes, CRUD
//! resources, capability-instance bindings, and synthesized scenario
//! cases are built out in M6-M9 once the checkpoint has proven the
//! architecture, not spent upfront on completeness the checkpoint
//! doesn't need.
//!
//! IDs are stable semantic keys (`table/<name>`, `stream/<name>`, ...)
//! derived from declaration order, never from a `HashMap` iteration
//! order or a generated path — the same discipline
//! `ciac_codegen::semantic_model` already established for v0.18's
//! canonical model.

use ciac_ir::{Cardinality, EdgeKind, FieldType, NodeKind, NormalizedIr, RefAction};
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The current `SimPlan` schema version. Bump on any breaking change to
/// this module's serialized shape; `--replay` refuses a mismatched
/// version rather than guessing compatibility.
pub const PLAN_VERSION: u32 = 1;

/// The target-neutral simulation plan for one compiled program.
#[derive(Debug, Clone, Serialize)]
pub struct SimPlan {
    pub plan_version: u32,
    /// The `.ciac` program's own architecture identity — the caller
    /// supplies this (typically the same `source_hash`
    /// `ciac-codegen::manifest` already computes) rather than `ciac-sim`
    /// re-deriving a second hash of source text it never reads.
    pub source_hash: String,
    pub project_name: String,
    pub multi_service: bool,
    pub services: Vec<SimService>,
    pub tables: Vec<SimTable>,
    pub streams: Vec<SimStream>,
    pub jobs: Vec<SimJob>,
    pub workers: Vec<SimWorker>,
    /// 28UpdatePlan.md M1.
    pub apis: Vec<SimApi>,
    /// 28UpdatePlan.md M1.
    pub call_edges: Vec<SimCallEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimService {
    /// `service/<Name>`, or `service/<program-name>` for a single
    /// implicit service (mirrors `ciac_codegen::semantic_model`'s own
    /// "no service blocks" fallback).
    pub key: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum SimFieldType {
    Str,
    Int,
    Float,
    Bool,
    Uuid,
    Timestamp,
    Json,
    Enum {
        variants: Vec<String>,
    },
    /// A resolved `Reference<T>` field. `target_table` is `None` when
    /// the reference's target record has no backing table in this
    /// plan's slice yet (frozen scope) — the reference is recorded but
    /// not resolvable, and the fake must refuse rather than guess.
    Reference {
        target_table: Option<String>,
        cardinality: SimCardinality,
        on_delete: SimRefAction,
        on_update: SimRefAction,
        unique: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SimCardinality {
    One,
    Many,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SimRefAction {
    Restrict,
    Cascade,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimColumn {
    pub name: String,
    pub ty: SimFieldType,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimTable {
    /// `table/<name>`.
    pub key: String,
    pub name: String,
    pub service_key: Option<String>,
    pub columns: Vec<SimColumn>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimStream {
    /// `stream/<name>`.
    pub key: String,
    pub name: String,
    pub subject: String,
    pub service_key: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimJob {
    /// `job/<name>`.
    pub key: String,
    pub name: String,
    pub service_key: Option<String>,
    pub schedule: String,
    pub catch_up: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SimWorker {
    /// `worker/<name>`.
    pub key: String,
    pub name: String,
    pub service_key: Option<String>,
    /// The stream subject this worker consumes, or `None` when no
    /// `AsyncMessage` edge feeds it (an unreachable worker `ciac check`
    /// would already have warned about — the plan still records it so a
    /// scenario referencing it by name gets a clear preflight error
    /// rather than a missing-key panic).
    pub subject: Option<String>,
    pub queue_group: String,
    pub concurrency: u32,
    pub max_retries: u32,
}

/// 28UpdatePlan.md M1: an api node, the router-registration and
/// call-target identity a routed cross-service call needs. Distinct
/// from `ciac_codegen::model::CallApiCtx` (a target-rendering context
/// carrying HTTP method/path/payload-class detail this plan-derivation
/// layer has no reason to know) — `SimApi` is only what the simulator
/// itself needs: which service owns the api, keyed the same way every
/// other `Sim*` fact is.
#[derive(Debug, Clone, Serialize)]
pub struct SimApi {
    /// `api/<name>`.
    pub key: String,
    pub name: String,
    pub service_key: Option<String>,
}

/// 28UpdatePlan.md M1: one `call <Service>.<Api>` edge, resolved to the
/// stable keys of its two endpoints. `caller_key` is the *owning*
/// api/worker/job node's key (the pipeline the `Call` step lives in),
/// not a second api key — a worker or job can issue a routed call
/// without itself being callable. M2's call router consumes this list
/// to resolve a `call.request` effect to its callee at simulation run
/// time; cycle detection is *not* this crate's job — `ciac-sema`'s own
/// `CycleDetection` pass already includes `EdgeKind::ServiceCall` in
/// its combined flow-cycle check (`passes/cycles.rs`), so a program
/// with a call cycle already fails `ciac check`/`ciac build` and can
/// never reach `SimPlan::from_ir` in the first place — see the M1
/// Shipped note for why `check_acyclic`/`SIM0012` were dropped.
#[derive(Debug, Clone, Serialize)]
pub struct SimCallEdge {
    pub caller_key: String,
    pub callee_key: String,
}

impl SimPlan {
    /// Builds a plan from validated IR. Deterministic: two `NormalizedIr`
    /// values describing the same architecture (regardless of
    /// declaration order) produce the same `plan_hash`, because every
    /// collection here is sorted by its own stable key before being
    /// stored, never left in raw node-table order.
    pub fn from_ir(ir: &NormalizedIr, source_hash: impl Into<String>) -> SimPlan {
        let mut services: Vec<SimService> = ir
            .services()
            .map(|s| SimService {
                key: format!("service/{}", s.name),
                name: s.name.clone(),
            })
            .collect();
        services.sort_by(|a, b| a.key.cmp(&b.key));

        let service_key = |node_service: Option<ciac_ir::ServiceId>| {
            node_service.map(|sid| format!("service/{}", ir.service(sid).name))
        };

        let mut tables: Vec<SimTable> = ir
            .tables()
            .map(|(_, table)| {
                let record = ir.record(table.record);
                let columns = record
                    .fields
                    .iter()
                    .map(|f| SimColumn {
                        name: f.name.clone(),
                        ty: sim_field_type(ir, &f.ty),
                    })
                    .collect();
                SimTable {
                    key: format!("table/{}", table.name),
                    name: table.name.clone(),
                    service_key: service_key(table.service),
                    columns,
                }
            })
            .collect();
        tables.sort_by(|a, b| a.key.cmp(&b.key));

        let mut streams: Vec<SimStream> = ir
            .nodes_of_kind(NodeKind::Stream)
            .filter_map(|n| {
                let (name, subject) = match &n.component {
                    ciac_ir::Component::Stream { name, subject, .. } => {
                        (name.clone(), subject.clone())
                    }
                    _ => return None,
                };
                Some(SimStream {
                    key: format!("stream/{name}"),
                    name,
                    subject,
                    service_key: service_key(n.service),
                })
            })
            .collect();
        streams.sort_by(|a, b| a.key.cmp(&b.key));

        let mut jobs: Vec<SimJob> = ir
            .nodes_of_kind(NodeKind::Job)
            .filter_map(|n| {
                let (name, config) = match &n.component {
                    ciac_ir::Component::Job { name, config } => (name.clone(), config),
                    _ => return None,
                };
                Some(SimJob {
                    key: format!("job/{name}"),
                    name,
                    service_key: service_key(n.service),
                    schedule: config.schedule.clone(),
                    catch_up: config.catch_up,
                })
            })
            .collect();
        jobs.sort_by(|a, b| a.key.cmp(&b.key));

        let mut workers: Vec<SimWorker> = ir
            .nodes_of_kind(NodeKind::Worker)
            .filter_map(|n| {
                let (name, config) = match &n.component {
                    ciac_ir::Component::Worker { name, config } => (name.clone(), config),
                    _ => return None,
                };
                // Matches ciac-codegen::model's own derivation: the
                // stream feeding this worker's sole AsyncMessage inbound
                // edge, if any.
                let subject = ir
                    .edges_to(n.id)
                    .find(|e| e.kind == EdgeKind::AsyncMessage)
                    .and_then(|e| match &ir.node(e.from).component {
                        ciac_ir::Component::Stream { subject, .. } => Some(subject.clone()),
                        _ => None,
                    });
                Some(SimWorker {
                    key: format!("worker/{name}"),
                    queue_group: snake_case(&name),
                    name,
                    service_key: service_key(n.service),
                    subject,
                    concurrency: config.concurrency,
                    max_retries: config.max_retries,
                })
            })
            .collect();
        workers.sort_by(|a, b| a.key.cmp(&b.key));

        let mut apis: Vec<SimApi> = ir
            .nodes_of_kind(NodeKind::Api)
            .filter_map(|n| {
                let name = n.component.name()?.to_owned();
                Some(SimApi {
                    key: format!("api/{name}"),
                    name,
                    service_key: service_key(n.service),
                })
            })
            .collect();
        apis.sort_by(|a, b| a.key.cmp(&b.key));

        // Every `Call` step wires a `ServiceCall` edge from the
        // pipeline's own owning node (an api, worker, or job -- see
        // `Pipeline::owner`'s doc) to the callee api node
        // (`ciac-sema::build::wire_steps`). Only apis are ever call
        // *targets* (`ciac-codegen::model::call_targets`'s own filter on
        // `Component::Api`), so `callee_key` is always resolvable
        // through the `apis` list just derived above.
        let mut call_edges: Vec<SimCallEdge> = ir
            .edges()
            .filter(|e| e.kind == EdgeKind::ServiceCall)
            .filter_map(|e| {
                Some(SimCallEdge {
                    caller_key: node_key(ir, e.from)?,
                    callee_key: node_key(ir, e.to)?,
                })
            })
            .collect();
        call_edges
            .sort_by(|a, b| (&a.caller_key, &a.callee_key).cmp(&(&b.caller_key, &b.callee_key)));

        SimPlan {
            plan_version: PLAN_VERSION,
            source_hash: source_hash.into(),
            project_name: ir.name.clone(),
            multi_service: ir.multi_service,
            services,
            tables,
            streams,
            jobs,
            workers,
            apis,
            call_edges,
        }
    }

    /// A deterministic hash over the canonical, sorted plan — the same
    /// pattern `ciac_codegen::semantic_model::SemanticModel::semantic_hash`
    /// already established: `serde_json::to_vec` over a struct whose
    /// collections are pre-sorted by stable key, not a canonicalizing
    /// serializer reordering an unsorted one.
    pub fn plan_hash(&self) -> String {
        let json = serde_json::to_vec(self).expect("SimPlan serializes");
        let mut hasher = Sha256::new();
        hasher.update(&json);
        format!("sha256:{:x}", hasher.finalize())
    }
}

fn sim_field_type(ir: &NormalizedIr, ty: &FieldType) -> SimFieldType {
    match ty {
        FieldType::Str => SimFieldType::Str,
        FieldType::Int => SimFieldType::Int,
        FieldType::Float => SimFieldType::Float,
        FieldType::Bool => SimFieldType::Bool,
        FieldType::Uuid => SimFieldType::Uuid,
        FieldType::Timestamp => SimFieldType::Timestamp,
        FieldType::Json => SimFieldType::Json,
        FieldType::Enum { variants } => SimFieldType::Enum {
            variants: variants.clone(),
        },
        FieldType::Reference {
            table,
            cardinality,
            on_delete,
            on_update,
            unique,
            ..
        } => SimFieldType::Reference {
            target_table: Some(format!("table/{}", ir.table(*table).name)),
            cardinality: match cardinality {
                Cardinality::One => SimCardinality::One,
                Cardinality::Many => SimCardinality::Many,
            },
            on_delete: sim_ref_action(*on_delete),
            on_update: sim_ref_action(*on_update),
            unique: *unique,
        },
    }
}

/// 28UpdatePlan.md M1: the stable key for a `ServiceCall` edge endpoint
/// — only ever an api, worker, or job node (a pipeline's own owner, or
/// its call target; `wire_steps` in `ciac-sema::build` never wires a
/// `ServiceCall` edge to or from anything else). `None` for any other
/// `NodeKind` rather than panicking — a future edge source ciac-sema
/// doesn't emit today is a plan that silently drops the edge, not a
/// crash in an unrelated backend.
fn node_key(ir: &NormalizedIr, id: ciac_ir::NodeId) -> Option<String> {
    let node = ir.node(id);
    let name = node.component.name()?;
    let prefix = match node.component.kind() {
        NodeKind::Api => "api",
        NodeKind::Worker => "worker",
        NodeKind::Job => "job",
        _ => return None,
    };
    Some(format!("{prefix}/{name}"))
}

fn sim_ref_action(action: RefAction) -> SimRefAction {
    match action {
        RefAction::Restrict => SimRefAction::Restrict,
        RefAction::Cascade => SimRefAction::Cascade,
    }
}

/// Matches `heck::ToSnakeCase` closely enough for the ASCII identifiers
/// CIaC names are restricted to, without pulling in the dependency for
/// one call site — `ciac-sim` has no other use for it.
fn snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciac_diagnostics::{Diagnostics, SourceMap};

    fn compile(src: &str) -> NormalizedIr {
        let mut sources = SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        let ir = ciac_sema::analyze(&program, &mut diags);
        assert!(!diags.has_errors(), "{diags:?}");
        ir.expect("analyzes")
    }

    const SRC: &str = r#"
service Ops;
use { db Postgres; queue NATS; scheduler Cron; }
record Item {
    id: Uuid;
    name: String;
}
table Items: Item;
stream Uploaded: Item;
worker Ingest on Uploaded { max_retries: 3; concurrency: 2; }
job Cleanup { schedule: "0 3 * * *"; catch_up: true; }
"#;

    #[test]
    fn plan_construction_is_deterministic_across_declaration_reorder() {
        const REORDERED: &str = r#"
service Ops;
use { db Postgres; queue NATS; scheduler Cron; }
job Cleanup { schedule: "0 3 * * *"; catch_up: true; }
record Item {
    id: Uuid;
    name: String;
}
stream Uploaded: Item;
worker Ingest on Uploaded { max_retries: 3; concurrency: 2; }
table Items: Item;
"#;
        let plan1 = SimPlan::from_ir(&compile(SRC), "sha256:fixed");
        let plan2 = SimPlan::from_ir(&compile(REORDERED), "sha256:fixed");
        assert_eq!(plan1.plan_hash(), plan2.plan_hash());
    }

    #[test]
    fn plan_captures_worker_subject_and_job_catch_up() {
        let plan = SimPlan::from_ir(&compile(SRC), "sha256:fixed");
        let worker = plan
            .workers
            .iter()
            .find(|w| w.name == "Ingest")
            .expect("worker present");
        assert_eq!(worker.subject.as_deref(), Some("ops.uploaded"));
        assert_eq!(worker.max_retries, 3);
        assert_eq!(worker.concurrency, 2);
        assert_eq!(worker.queue_group, "ingest");

        let job = plan.jobs.iter().find(|j| j.name == "Cleanup").unwrap();
        assert!(job.catch_up);
        assert_eq!(job.schedule, "0 3 * * *");

        let table = plan.tables.iter().find(|t| t.name == "Items").unwrap();
        assert_eq!(table.columns.len(), 2);
    }

    const CROSS_SERVICE_CALL: &str = r#"
project X;
record A { id: Uuid; }
service Caller {
    api In: A;
    pipeline In: call Callee.Out -> Return;
}
service Callee {
    api Out: A;
    pipeline Out: Return;
}
"#;

    #[test]
    fn derives_apis_and_call_edges_across_services() {
        let plan = SimPlan::from_ir(&compile(CROSS_SERVICE_CALL), "sha256:fixed");
        let api_names: Vec<&str> = plan.apis.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(api_names, vec!["In", "Out"]);

        let caller = plan.apis.iter().find(|a| a.name == "In").unwrap();
        assert_eq!(caller.service_key.as_deref(), Some("service/Caller"));
        let callee = plan.apis.iter().find(|a| a.name == "Out").unwrap();
        assert_eq!(callee.service_key.as_deref(), Some("service/Callee"));

        assert_eq!(plan.call_edges.len(), 1);
        assert_eq!(plan.call_edges[0].caller_key, "api/In");
        assert_eq!(plan.call_edges[0].callee_key, "api/Out");
    }

    #[test]
    fn a_semantically_different_program_hashes_differently() {
        const CHANGED: &str = r#"
service Ops;
use { db Postgres; queue NATS; scheduler Cron; }
record Item {
    id: Uuid;
    name: String;
}
table Items: Item;
stream Uploaded: Item;
worker Ingest on Uploaded { max_retries: 5; concurrency: 2; }
job Cleanup { schedule: "0 3 * * *"; catch_up: true; }
"#;
        let plan1 = SimPlan::from_ir(&compile(SRC), "sha256:fixed");
        let plan2 = SimPlan::from_ir(&compile(CHANGED), "sha256:fixed");
        assert_ne!(plan1.plan_hash(), plan2.plan_hash());
    }
}
