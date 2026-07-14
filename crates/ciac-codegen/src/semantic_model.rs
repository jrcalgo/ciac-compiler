//! v0.18 M1: the canonical semantic model — a target-independent
//! projection of `NormalizedIr` for *architecture* comparison ("did
//! this change break a consumer"), as distinct from the existing
//! generated-file `ciac diff` (byte/manifest comparison of an output
//! tree). See `docs/evolution.md`.
//!
//! `NormalizedIr`'s own `NodeId`/`ServiceId`/`RecordId`/`TableId` are
//! insertion-order indices: reordering declarations in source can
//! renumber them without changing architecture at all, which would
//! make them useless as a durable baseline identity. Every entity here
//! is keyed by a stable logical string instead (`record/Order`,
//! `service/Billing/api/Charge`, ...) computed purely from declared
//! names — never source spans, never index order. Two builds of the
//! same architecture, reordered or reformatted, produce the same
//! `SemanticModel` and the same `semantic_hash`.
//!
//! This module intentionally stops at the *model* (a typed value you
//! can compare) and its canonical hash — the actual comparator that
//! classifies a difference as breaking/additive/internal is v0.18 M2
//! (`ciac-codegen::semantic_diff`, not yet written). `SemanticModel` is
//! the thing both sides of that future comparison serialize into.

use ciac_ir::{
    Cardinality, Component, FieldType, HirType, NodeKind, NormalizedIr, RecordKind, RefAction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Bumped whenever `SemanticModel`'s shape changes in a way that isn't
/// simply additive-with-defaults — a reader must refuse a baseline
/// stamped with a version it doesn't understand rather than silently
/// dropping fields it doesn't recognize (18UpdatePlan.md Pillar 2).
pub const SEMANTIC_MODEL_VERSION: u32 = 1;

/// Bumped whenever the checked-in baseline *wrapper* shape changes
/// (independent of the model payload it carries).
pub const SEMANTIC_BASELINE_VERSION: u32 = 1;

/// A stable logical identity — never an insertion-order index. See the
/// module doc's identity scheme; e.g. `record/Order`,
/// `record/Order/field/total`, `service/Billing/api/Charge`,
/// `global/database/main`.
pub type Key = String;

fn service_prefix(service: Option<&str>) -> String {
    match service {
        Some(name) => format!("service/{name}"),
        None => "global".to_owned(),
    }
}

fn node_kind_str(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Api => "api",
        NodeKind::Service => "handler",
        NodeKind::Worker => "worker",
        NodeKind::Job => "job",
        NodeKind::Channel => "channel",
        NodeKind::Database => "database",
        NodeKind::Cache => "cache",
        NodeKind::Queue => "queue",
        NodeKind::Stream => "stream",
        NodeKind::Auth => "auth",
        NodeKind::Logging => "logging",
        NodeKind::Metrics => "metrics",
        NodeKind::Tracing => "tracing",
        NodeKind::Users => "users",
        NodeKind::ObjectStore => "object_store",
        NodeKind::Email => "email",
        NodeKind::Search => "search",
        NodeKind::ExternalHttp => "external_http",
        NodeKind::Scheduler => "scheduler",
        NodeKind::Realtime => "realtime",
    }
}

/// `service/<service>/<kind>/<name>` (or `global/<kind>/<name>` with
/// no owning service) — the one identity scheme every node-backed
/// entity (api, worker, job, channel, stream, and every capability
/// instance) shares, per 18UpdatePlan.md's "component" row.
fn component_key(service: Option<&str>, kind: NodeKind, name: &str) -> Key {
    format!("{}/{}/{name}", service_prefix(service), node_kind_str(kind))
}

fn record_key(name: &str) -> Key {
    format!("record/{name}")
}

fn field_key(record: &str, field: &str) -> Key {
    format!("record/{record}/field/{field}")
}

fn table_key(service: Option<&str>, name: &str) -> Key {
    format!("{}/table/{name}", service_prefix(service))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProjectModel {
    pub key: Key,
    pub name: String,
    pub multi_service: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ServiceModel {
    pub key: Key,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum CardinalityModel {
    One,
    Many,
}

impl From<Cardinality> for CardinalityModel {
    fn from(value: Cardinality) -> Self {
        match value {
            Cardinality::One => CardinalityModel::One,
            Cardinality::Many => CardinalityModel::Many,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RefActionModel {
    Restrict,
    Cascade,
}

impl From<RefAction> for RefActionModel {
    fn from(value: RefAction) -> Self {
        match value {
            RefAction::Restrict => RefActionModel::Restrict,
            RefAction::Cascade => RefActionModel::Cascade,
        }
    }
}

/// [`FieldType`] with every index (`RecordId`/`TableId`) resolved to a
/// logical [`Key`] — the same closed set, serialized as a typed enum
/// rather than a `Debug` string (unlike the older, narrower
/// `evolution::RecordSchema`, which predates this canonical model).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum FieldTypeModel {
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
    Reference {
        target: Key,
        table: Key,
        cardinality: CardinalityModel,
        on_delete: RefActionModel,
        on_update: RefActionModel,
        unique: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FieldModel {
    pub key: Key,
    pub name: String,
    pub ty: FieldTypeModel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum RecordKindModel {
    Data,
    Error,
}

impl From<RecordKind> for RecordKindModel {
    fn from(value: RecordKind) -> Self {
        match value {
            RecordKind::Data => RecordKindModel::Data,
            RecordKind::Error => RecordKindModel::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecordModel {
    pub key: Key,
    pub name: String,
    pub kind: RecordKindModel,
    pub fields: Vec<FieldModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TableModel {
    pub key: Key,
    pub name: String,
    pub service: Option<Key>,
    pub record: Key,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RouteModel {
    pub key: Key,
    pub service: Option<Key>,
    pub name: String,
    pub method: String,
    pub path: Option<String>,
    pub scope: Option<String>,
    pub request: Option<Key>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StreamModel {
    pub key: Key,
    pub service: Option<Key>,
    pub name: String,
    pub subject: String,
    pub record: Option<Key>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChannelModel {
    pub key: Key,
    pub service: Option<Key>,
    pub name: String,
    pub path: String,
}

/// Every other node-backed component: worker/job/service(handler) and
/// every infrastructure capability instance (database, cache, queue,
/// auth, logging, metrics, tracing, users, object_store, email,
/// search, external_http, scheduler, realtime). `config` is the
/// component's own typed configuration, rendered as canonical JSON
/// (already a typed `serde_json::Value` derived from typed structs —
/// not a `Debug` string) so kind-specific fields don't need one model
/// type per `NodeKind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CapabilityModel {
    pub key: Key,
    pub service: Option<Key>,
    pub kind: String,
    pub name: String,
    #[schemars(with = "serde_json::Value")]
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "step")]
pub enum StepModel {
    Auth {
        node: Key,
    },
    Publish {
        stream: Key,
    },
    Return,
    Handler {
        node: Key,
    },
    Call {
        target: Key,
    },
    Match {
        field: String,
        arms: Vec<MatchArmModel>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MatchArmModel {
    pub label: Option<String>,
    pub steps: Vec<StepModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PipelineModel {
    pub key: Key,
    pub service: Option<Key>,
    pub payload: Option<Key>,
    pub steps: Vec<StepModel>,
}

/// [`HirType`] projected the same way [`FieldTypeModel`] projects
/// [`FieldType`] — every `RecordId` resolved to a logical [`Key`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type")]
pub enum HirTypeModel {
    Str,
    Int,
    Float,
    Bool,
    Uuid,
    Timestamp,
    Json,
    Enum { variants: Vec<String> },
    Record { key: Key },
    Option { of: Box<HirTypeModel> },
    List { of: Box<HirTypeModel> },
    Unit,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HandlerParamModel {
    pub name: String,
    pub ty: HirTypeModel,
}

/// A typed inline/`extern` handler's signature (v0.7+). `body_digest`
/// is a stable hash of the body's canonical (span-free) HIR when one
/// exists — a structural digest, not a behavioral proof: two
/// body_digests differing means *something* in the handler's logic
/// changed, classified (as `internal`, per 18UpdatePlan.md — this
/// version does not claim behavioral equivalence) by v0.18 M2's
/// differ, not here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HandlerModel {
    pub key: Key,
    pub params: Vec<HandlerParamModel>,
    pub return_ty: HirTypeModel,
    pub has_body: bool,
    pub body_digest: Option<String>,
}

/// The canonical, target-independent projection of a validated
/// program. Collections are sorted by logical `key` so serialization
/// is deterministic regardless of declaration order — the property
/// `semantic_hash` depends on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticModel {
    pub semantic_model_version: u32,
    pub project: ProjectModel,
    pub services: Vec<ServiceModel>,
    pub records: Vec<RecordModel>,
    pub tables: Vec<TableModel>,
    pub routes: Vec<RouteModel>,
    pub streams: Vec<StreamModel>,
    pub channels: Vec<ChannelModel>,
    pub capabilities: Vec<CapabilityModel>,
    pub pipelines: Vec<PipelineModel>,
    pub handlers: Vec<HandlerModel>,
}

impl SemanticModel {
    pub fn from_ir(ir: &NormalizedIr) -> SemanticModel {
        let service_name = |id: ciac_ir::ServiceId| ir.service(id).name.clone();
        let service_key_of = |id: Option<ciac_ir::ServiceId>| id.map(service_name);

        let mut services: Vec<ServiceModel> = ir
            .services()
            .map(|s| ServiceModel {
                key: format!("service/{}", s.name),
                name: s.name.clone(),
            })
            .collect();
        services.sort_by(|a, b| a.key.cmp(&b.key));

        let field_type_model = |ty: &FieldType| -> FieldTypeModel {
            match ty {
                FieldType::Str => FieldTypeModel::Str,
                FieldType::Int => FieldTypeModel::Int,
                FieldType::Float => FieldTypeModel::Float,
                FieldType::Bool => FieldTypeModel::Bool,
                FieldType::Uuid => FieldTypeModel::Uuid,
                FieldType::Timestamp => FieldTypeModel::Timestamp,
                FieldType::Json => FieldTypeModel::Json,
                FieldType::Enum { variants } => FieldTypeModel::Enum {
                    variants: variants.clone(),
                },
                FieldType::Reference {
                    target,
                    table,
                    cardinality,
                    on_delete,
                    on_update,
                    unique,
                } => {
                    let target_record = ir.record(*target);
                    let target_table = ir.table(*table);
                    FieldTypeModel::Reference {
                        target: record_key(&target_record.name),
                        table: table_key(
                            service_key_of(target_table.service).as_deref(),
                            &target_table.name,
                        ),
                        cardinality: (*cardinality).into(),
                        on_delete: (*on_delete).into(),
                        on_update: (*on_update).into(),
                        unique: *unique,
                    }
                }
            }
        };

        let mut records: Vec<RecordModel> = ir
            .records()
            .map(|(_, record)| RecordModel {
                key: record_key(&record.name),
                name: record.name.clone(),
                kind: record.kind.into(),
                fields: record
                    .fields
                    .iter()
                    .map(|f| FieldModel {
                        key: field_key(&record.name, &f.name),
                        name: f.name.clone(),
                        ty: field_type_model(&f.ty),
                    })
                    .collect(),
            })
            .collect();
        records.sort_by(|a, b| a.key.cmp(&b.key));

        let mut tables: Vec<TableModel> = ir
            .tables()
            .map(|(_, table)| {
                let svc = service_key_of(table.service);
                TableModel {
                    key: table_key(svc.as_deref(), &table.name),
                    name: table.name.clone(),
                    service: svc.map(|s| format!("service/{s}")),
                    record: record_key(&ir.record(table.record).name),
                }
            })
            .collect();
        tables.sort_by(|a, b| a.key.cmp(&b.key));

        let mut routes = Vec::new();
        let mut streams = Vec::new();
        let mut channels = Vec::new();
        let mut capabilities = Vec::new();
        let mut handlers = Vec::new();

        for node in ir.nodes() {
            let svc = service_key_of(node.service);
            let name = node.component.name().unwrap_or_default().to_owned();
            match &node.component {
                Component::Api {
                    name,
                    request,
                    config,
                } => routes.push(RouteModel {
                    key: component_key(svc.as_deref(), NodeKind::Api, name),
                    service: svc.clone().map(|s| format!("service/{s}")),
                    name: name.clone(),
                    method: config.method.as_str().to_owned(),
                    path: config.path.clone(),
                    scope: config.scope.clone(),
                    request: request.map(|r| record_key(&ir.record(r).name)),
                }),
                Component::Stream {
                    name,
                    subject,
                    record,
                } => streams.push(StreamModel {
                    key: component_key(svc.as_deref(), NodeKind::Stream, name),
                    service: svc.clone().map(|s| format!("service/{s}")),
                    name: name.clone(),
                    subject: subject.clone(),
                    record: record.map(|r| record_key(&ir.record(r).name)),
                }),
                Component::Channel { name, config } => channels.push(ChannelModel {
                    key: component_key(svc.as_deref(), NodeKind::Channel, name),
                    service: svc.clone().map(|s| format!("service/{s}")),
                    name: name.clone(),
                    path: config.path.clone(),
                }),
                Component::Service {
                    name,
                    signature: Some(body),
                } => {
                    let key = component_key(svc.as_deref(), NodeKind::Service, name);
                    handlers.push(HandlerModel {
                        key: key.clone(),
                        params: body
                            .params
                            .iter()
                            .map(|(pname, ty)| HandlerParamModel {
                                name: pname.clone(),
                                ty: hir_type_model(ir, ty),
                            })
                            .collect(),
                        return_ty: hir_type_model(ir, &body.return_ty),
                        has_body: body.body.is_some(),
                        body_digest: body.body.as_ref().map(|stmts| digest_of(stmts)),
                    });
                    capabilities.push(CapabilityModel {
                        key,
                        service: svc.clone().map(|s| format!("service/{s}")),
                        kind: node_kind_str(NodeKind::Service).to_owned(),
                        name: name.clone(),
                        config: serde_json::Value::Null,
                    });
                }
                other => {
                    let kind = other.kind();
                    capabilities.push(CapabilityModel {
                        key: component_key(svc.as_deref(), kind, &name),
                        service: svc.clone().map(|s| format!("service/{s}")),
                        kind: node_kind_str(kind).to_owned(),
                        name,
                        config: component_config_json(other),
                    });
                }
            }
        }
        routes.sort_by(|a, b| a.key.cmp(&b.key));
        streams.sort_by(|a, b| a.key.cmp(&b.key));
        channels.sort_by(|a, b| a.key.cmp(&b.key));
        capabilities.sort_by(|a, b| a.key.cmp(&b.key));
        handlers.sort_by(|a, b| a.key.cmp(&b.key));

        let node_key = |id: ciac_ir::NodeId| -> Key {
            let node = ir.node(id);
            let svc = service_key_of(node.service);
            let name = node.component.name().unwrap_or_default();
            component_key(svc.as_deref(), node.component.kind(), name)
        };

        let mut pipelines: Vec<PipelineModel> = ir
            .pipelines
            .iter()
            .map(|p| PipelineModel {
                key: node_key(p.owner),
                service: service_key_of(p.service).map(|s| format!("service/{s}")),
                payload: p.payload.map(|r| record_key(&ir.record(r).name)),
                steps: p
                    .steps
                    .iter()
                    .map(|s| step_model(&s.kind, &node_key))
                    .collect(),
            })
            .collect();
        pipelines.sort_by(|a, b| a.key.cmp(&b.key));

        SemanticModel {
            semantic_model_version: SEMANTIC_MODEL_VERSION,
            project: ProjectModel {
                key: format!("project/{}", ir.name),
                name: ir.name.clone(),
                multi_service: ir.multi_service,
            },
            services,
            records,
            tables,
            routes,
            streams,
            channels,
            capabilities,
            pipelines,
            handlers,
        }
    }

    /// A deterministic hash over the canonical, sorted model — two
    /// programs with the same architecture (regardless of declaration
    /// order, formatting, or comments) produce the same hash.
    pub fn semantic_hash(&self) -> String {
        let json = serde_json::to_vec(self).expect("SemanticModel serializes");
        let mut hasher = Sha256::new();
        hasher.update(&json);
        format!("sha256:{:x}", hasher.finalize())
    }
}

fn hir_type_model(ir: &NormalizedIr, ty: &HirType) -> HirTypeModel {
    match ty {
        HirType::Str => HirTypeModel::Str,
        HirType::Int => HirTypeModel::Int,
        HirType::Float => HirTypeModel::Float,
        HirType::Bool => HirTypeModel::Bool,
        HirType::Uuid => HirTypeModel::Uuid,
        HirType::Timestamp => HirTypeModel::Timestamp,
        HirType::Json => HirTypeModel::Json,
        HirType::Enum { variants } => HirTypeModel::Enum {
            variants: variants.clone(),
        },
        HirType::Record(id) => HirTypeModel::Record {
            key: record_key(&ir.record(*id).name),
        },
        HirType::Option(inner) => HirTypeModel::Option {
            of: Box::new(hir_type_model(ir, inner)),
        },
        HirType::List(inner) => HirTypeModel::List {
            of: Box::new(hir_type_model(ir, inner)),
        },
        HirType::Unit => HirTypeModel::Unit,
        HirType::Never => HirTypeModel::Never,
    }
}

fn step_model(kind: &ciac_ir::StepKind, node_key: &impl Fn(ciac_ir::NodeId) -> Key) -> StepModel {
    match kind {
        ciac_ir::StepKind::Auth { node } => StepModel::Auth {
            node: node_key(*node),
        },
        ciac_ir::StepKind::Publish { stream } => StepModel::Publish {
            stream: node_key(*stream),
        },
        ciac_ir::StepKind::Return => StepModel::Return,
        ciac_ir::StepKind::Handler { node } => StepModel::Handler {
            node: node_key(*node),
        },
        ciac_ir::StepKind::Call { target } => StepModel::Call {
            target: node_key(*target),
        },
        ciac_ir::StepKind::Match { field, arms } => StepModel::Match {
            field: field.clone(),
            arms: arms
                .iter()
                .map(|arm| MatchArmModel {
                    label: arm.label.clone(),
                    steps: arm
                        .steps
                        .iter()
                        .map(|s| step_model(&s.kind, node_key))
                        .collect(),
                })
                .collect(),
        },
    }
}

/// A component's own configuration as canonical JSON — typed structs
/// serialized through `serde_json`, not `format!("{:?}")`. Node
/// name/kind are carried on [`CapabilityModel`] itself, so they're
/// dropped from this payload to avoid duplicating them inside `config`
/// too.
fn component_config_json(component: &Component) -> serde_json::Value {
    let mut value = serde_json::to_value(component).expect("Component serializes");
    if let serde_json::Value::Object(map) = &mut value {
        map.remove("kind");
        map.remove("name");
    }
    value
}

fn digest_of(stmts: &[ciac_ir::HirStmt]) -> String {
    let json = serde_json::to_vec(stmts).expect("HirStmt serializes");
    let mut hasher = Sha256::new();
    hasher.update(&json);
    format!("sha256:{:x}", hasher.finalize())
}

/// The checked-in baseline wrapper (18UpdatePlan.md Pillar 2):
/// independently versioned from the `SemanticModel` payload it
/// carries, with audit metadata (`compiler_version`, `entry`,
/// `source_hash`) that does not participate in `semantic_hash` itself
/// — formatting, comments, source path movement, and equivalent
/// blueprint expansion all leave `semantic_hash` unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SemanticBaseline {
    pub semantic_baseline_version: u32,
    pub semantic_model_version: u32,
    pub compiler_version: String,
    pub entry: String,
    pub source_hash: String,
    pub semantic_hash: String,
    pub model: SemanticModel,
}

impl SemanticBaseline {
    pub fn new(
        compiler_version: impl Into<String>,
        entry: impl Into<String>,
        source_hash: impl Into<String>,
        model: SemanticModel,
    ) -> SemanticBaseline {
        SemanticBaseline {
            semantic_baseline_version: SEMANTIC_BASELINE_VERSION,
            semantic_model_version: model.semantic_model_version,
            compiler_version: compiler_version.into(),
            entry: entry.into(),
            source_hash: source_hash.into(),
            semantic_hash: model.semantic_hash(),
            model,
        }
    }
}

/// A generated JSON Schema for the checked-in baseline document —
/// `docs/semantic-baseline-schema.json` is this, held byte-identical
/// by a staleness test (mirrors `protocol::schema_document`).
pub fn baseline_schema_document() -> serde_json::Value {
    serde_json::json!({
        "semantic_baseline_version": SEMANTIC_BASELINE_VERSION,
        "semantic_model_version": SEMANTIC_MODEL_VERSION,
        "schema": schemars::schema_for!(SemanticBaseline),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciac_diagnostics::Diagnostics;

    fn compile(src: &str) -> ciac_ir::NormalizedIr {
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()))
    }

    const SRC: &str = r#"
service Test;
use { db Postgres; }

record Video {
    id: Uuid;
    title: String;
}
table Videos: Video;

handler StoreVideo(v: Video) -> Video {
    let inserted = db.insert(Videos, v);
    return inserted;
}

api Upload: Video {
    method: POST;
    path: "/upload";
}
pipeline Upload:
    StoreVideo
    -> Return;
"#;

    fn ir() -> ciac_ir::NormalizedIr {
        compile(SRC)
    }

    #[test]
    fn keys_are_stable_not_index_based() {
        let model = SemanticModel::from_ir(&ir());
        assert_eq!(model.records[0].key, "record/Video");
        assert_eq!(model.records[0].fields[0].key, "record/Video/field/id");
        assert!(model.tables.iter().any(|t| t.key.contains("table/Videos")));
        assert!(model.routes.iter().any(|r| r.name == "Upload"));
        assert!(model.handlers.iter().any(|h| h.key.contains("StoreVideo")));
    }

    #[test]
    fn hash_is_deterministic_across_reserialization() {
        let model = SemanticModel::from_ir(&ir());
        let hash1 = model.semantic_hash();
        let model2 = SemanticModel::from_ir(&ir());
        let hash2 = model2.semantic_hash();
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn baseline_wrapper_round_trips() {
        let model = SemanticModel::from_ir(&ir());
        let baseline = SemanticBaseline::new("0.18.0", "main.ciac", "srchash", model);
        let json = serde_json::to_string_pretty(&baseline).expect("serializes");
        let back: SemanticBaseline = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(baseline, back);
    }

    /// The same architecture, declared in a different order with extra
    /// comments/whitespace, must hash identically — `NodeId`/`RecordId`
    /// insertion order (which *does* change here) must never leak into
    /// the canonical model or its hash.
    const SRC_REORDERED: &str = r#"
// A totally different comment, and the api/table/handler declared in
// reverse order relative to SRC above.
service Test;
use { db Postgres; }

api Upload: Video {
    method: POST;
    path: "/upload";
}
pipeline Upload:
    StoreVideo
    -> Return;

handler StoreVideo(v: Video) -> Video {
    let inserted = db.insert(Videos, v);
    return inserted;
}

table Videos: Video;
record Video {
    id: Uuid;
    title: String;
}
"#;

    #[test]
    fn declaration_reorder_and_comments_produce_the_same_hash() {
        let model = SemanticModel::from_ir(&ir());
        let reordered = SemanticModel::from_ir(&compile(SRC_REORDERED));
        assert_eq!(model.semantic_hash(), reordered.semantic_hash());
        assert_eq!(model, reordered);
    }

    #[test]
    fn a_real_architecture_change_changes_the_hash() {
        let model = SemanticModel::from_ir(&ir());
        const SRC_CHANGED: &str = r#"
service Test;
use { db Postgres; }

record Video {
    id: Uuid;
    title: String;
    summary: String;
}
table Videos: Video;

handler StoreVideo(v: Video) -> Video {
    let inserted = db.insert(Videos, v);
    return inserted;
}

api Upload: Video {
    method: POST;
    path: "/upload";
}
pipeline Upload:
    StoreVideo
    -> Return;
"#;
        let changed = SemanticModel::from_ir(&compile(SRC_CHANGED));
        assert_ne!(model.semantic_hash(), changed.semantic_hash());
    }
}
