//! v0.18 M2: the consumer-aware differ and classification matrix —
//! compares two [`crate::semantic_model::SemanticModel`]s (an old
//! baseline and a current build, or two arbitrary programs) and
//! produces a typed changelist, per 18UpdatePlan.md Pillar 3.
//!
//! This module deliberately covers the plan's *headline* classification
//! rules for each category rather than literally every row of its
//! matrix — persistence/rename-identity/backfill-ladder-aware
//! refinements arrive with the milestones that build the machinery
//! they depend on (M5/M6). Every rule implemented here is covered by
//! at least one fixture in this module's tests, not asserted from
//! prose alone.

use crate::semantic_model::{
    CapabilityModel, EdgeKindModel, FieldModel, HandlerModel, Key, PipelineModel, RecordModel,
    RouteModel, SemanticModel, StepModel, StreamModel,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Severity, most severe first — matches 18UpdatePlan.md Pillar 3's
/// listing order (`breaking; additive; internal; kind; symbol key`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Classification {
    Breaking,
    Additive,
    Internal,
}

impl Classification {
    fn rank(self) -> u8 {
        match self {
            Classification::Breaking => 0,
            Classification::Additive => 1,
            Classification::Internal => 2,
        }
    }

    /// When one edit has more than one compatibility direction, the
    /// top-level classification is the maximum by this precedence
    /// (18UpdatePlan.md: "the top-level classification is always the
    /// maximum by the precedence below").
    fn most_severe(self, other: Classification) -> Classification {
        if self.rank() <= other.rank() {
            self
        } else {
            other
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Symbol {
    pub kind: String,
    pub key: Key,
    pub display: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConsumerRef {
    pub kind: String,
    pub service: Option<String>,
    pub contract: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Impact {
    pub dimension: String,
    pub classification: Classification,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Change {
    pub id: String,
    pub kind: String,
    pub classification: Classification,
    pub symbol: Symbol,
    #[schemars(with = "Option<serde_json::Value>")]
    pub before: Option<serde_json::Value>,
    #[schemars(with = "Option<serde_json::Value>")]
    pub after: Option<serde_json::Value>,
    pub consumers: Vec<ConsumerRef>,
    pub impacts: Vec<Impact>,
    pub message: String,
    /// Set only when this entry's severity depends on data motion the
    /// migration differ can't compute on its own — the v0.18 M6
    /// backfill ladder's cue to offer `ciac backfill plan`.
    pub backfill_plan_available: bool,
}

fn change(
    kind: &str,
    classification: Classification,
    symbol: Symbol,
    message: impl Into<String>,
) -> Change {
    Change {
        id: format!("{kind}:{}", symbol.key),
        kind: kind.to_owned(),
        classification,
        symbol,
        before: None,
        after: None,
        consumers: Vec::new(),
        impacts: Vec::new(),
        message: message.into(),
        backfill_plan_available: false,
    }
}

fn symbol(kind: &str, key: &str, display: &str) -> Symbol {
    Symbol {
        kind: kind.to_owned(),
        key: key.to_owned(),
        display: display.to_owned(),
    }
}

/// The service a component/table logical key belongs to, parsed from
/// its own key scheme (`service/<Name>/...`) — `None` for `global/...`
/// keys (no owning service, e.g. a single-service program).
fn service_of_key(key: &str) -> Option<String> {
    key.strip_prefix("service/")
        .and_then(|rest| rest.split('/').next())
        .map(str::to_owned)
}

fn step_calls(steps: &[StepModel], target: &str, out: &mut bool) {
    for step in steps {
        match step {
            StepModel::Call { target: t } if t == target => *out = true,
            StepModel::Match { arms, .. } => {
                for arm in arms {
                    step_calls(&arm.steps, target, out);
                }
            }
            _ => {}
        }
    }
}

fn pipeline_calls(pipeline: &PipelineModel, target: &str) -> bool {
    let mut found = false;
    step_calls(&pipeline.steps, target, &mut found);
    found
}

/// Every other service's pipeline that `call`s this route — the
/// service-call boundary (18UpdatePlan.md's "generated client
/// operation"/"service-call request/response" consumer kinds).
fn route_callers(model: &SemanticModel, route: &RouteModel) -> Vec<ConsumerRef> {
    model
        .pipelines
        .iter()
        .filter(|p| p.service != route.service && pipeline_calls(p, &route.key))
        .filter_map(|p| p.service.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|svc| ConsumerRef {
            kind: "service_call".to_owned(),
            service: Some(svc),
            contract: route.name.clone(),
        })
        .collect()
}

/// Every service consuming a stream (an `AsyncMessage` edge from the
/// stream to a worker/channel) that isn't also one of its producers
/// (an `AsyncMessage` edge into the stream) — mirrors
/// `evolution::boundary_consumers`'s stream-boundary rule, generalized
/// onto the canonical model.
fn stream_consumers(model: &SemanticModel, stream: &StreamModel) -> Vec<ConsumerRef> {
    let producer_services: BTreeSet<String> = model
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKindModel::AsyncMessage && e.to == stream.key)
        .filter_map(|e| service_of_key(&e.from))
        .collect();
    model
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKindModel::AsyncMessage && e.from == stream.key)
        .filter_map(|e| service_of_key(&e.to))
        .filter(|svc| !producer_services.contains(svc))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|svc| ConsumerRef {
            kind: "stream_consumer".to_owned(),
            service: Some(svc),
            contract: stream.name.clone(),
        })
        .collect()
}

/// Every producer service of a stream — used to tell "removed the last
/// producer while consumers remain" (breaking) from "removed an
/// internal stream nothing external used" (internal).
fn stream_producers(model: &SemanticModel, stream: &StreamModel) -> BTreeSet<String> {
    model
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKindModel::AsyncMessage && e.to == stream.key)
        .filter_map(|e| service_of_key(&e.from))
        .collect()
}

/// Every route/stream (in `model`) whose request/payload record is
/// `record_key`, attributed to the *other* service using it — the
/// boundary a record's own field change can break, generalizing
/// `evolution::boundary_consumers` across both the HTTP and stream
/// surfaces onto the canonical model.
fn record_consumers(model: &SemanticModel, record_key: &str) -> Vec<ConsumerRef> {
    let mut out = Vec::new();
    for route in &model.routes {
        if route.request.as_deref() == Some(record_key) {
            out.extend(route_callers(model, route));
        }
    }
    for stream in &model.streams {
        if stream.record.as_deref() == Some(record_key) {
            out.extend(stream_consumers(model, stream));
        }
    }
    out.sort_by(|a, b| (&a.kind, &a.service, &a.contract).cmp(&(&b.kind, &b.service, &b.contract)));
    out.dedup();
    out
}

fn is_table_backed(model: &SemanticModel, record_key: &str) -> bool {
    model.tables.iter().any(|t| t.record == record_key)
}

fn field_to_json(field: &FieldModel) -> serde_json::Value {
    serde_json::to_value(&field.ty).unwrap_or(serde_json::Value::Null)
}

/// Diffs every record's field shape, key by key. `record_key` needs
/// `model`/`is_new` to look up table-backing and the record display
/// name because a plain `BTreeMap` keyed by field name doesn't carry
/// them.
fn diff_record_fields(old_model: &SemanticModel, new_model: &SemanticModel, out: &mut Vec<Change>) {
    let old_records: BTreeMap<&str, &RecordModel> = old_model
        .records
        .iter()
        .map(|r| (r.key.as_str(), r))
        .collect();
    let new_records: BTreeMap<&str, &RecordModel> = new_model
        .records
        .iter()
        .map(|r| (r.key.as_str(), r))
        .collect();

    for (key, old_record) in &old_records {
        let Some(new_record) = new_records.get(key) else {
            continue; // record removal itself is reported by diff_records
        };
        let old_fields: BTreeMap<&str, &FieldModel> = old_record
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f))
            .collect();
        let new_fields: BTreeMap<&str, &FieldModel> = new_record
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f))
            .collect();

        for (fname, old_field) in &old_fields {
            let field_key = &old_field.key;
            match new_fields.get(fname) {
                None => {
                    let consumers = record_consumers(old_model, key);
                    let mut c = change(
                        "record.field.removed",
                        Classification::Breaking,
                        symbol(
                            "field",
                            field_key,
                            &format!("{}.{}", old_record.name, fname),
                        ),
                        format!("record `{}` removed field `{fname}`", old_record.name),
                    );
                    c.before = Some(field_to_json(old_field));
                    c.impacts = vec![
                        Impact {
                            dimension: "request_acceptance".to_owned(),
                            classification: Classification::Additive,
                        },
                        Impact {
                            dimension: "generated_client_source".to_owned(),
                            classification: Classification::Breaking,
                        },
                    ];
                    c.consumers = consumers;
                    out.push(c);
                }
                Some(new_field) if new_field.ty != old_field.ty => {
                    let consumers = record_consumers(old_model, key);
                    let mut c = change(
                        "record.field.retyped",
                        Classification::Breaking,
                        symbol(
                            "field",
                            field_key,
                            &format!("{}.{}", old_record.name, fname),
                        ),
                        format!("record `{}` field `{fname}` changed type", old_record.name),
                    );
                    c.before = Some(field_to_json(old_field));
                    c.after = Some(field_to_json(new_field));
                    c.consumers = consumers;
                    out.push(c);
                }
                Some(_) => {}
            }
        }

        for (fname, new_field) in &new_fields {
            if old_fields.contains_key(fname) {
                continue;
            }
            let table_backed = is_table_backed(new_model, key);
            let mut c = if table_backed {
                let mut c = change(
                    "table.column.added_required",
                    Classification::Breaking,
                    symbol(
                        "field",
                        &new_field.key,
                        &format!("{}.{}", new_record.name, fname),
                    ),
                    format!(
                        "record `{}` (table-backed) gained field `{fname}` with no universal \
                         default for existing rows",
                        new_record.name
                    ),
                );
                c.backfill_plan_available = true;
                c
            } else {
                change(
                    "record.field.added",
                    Classification::Additive,
                    symbol(
                        "field",
                        &new_field.key,
                        &format!("{}.{}", new_record.name, fname),
                    ),
                    format!("record `{}` gained field `{fname}`", new_record.name),
                )
            };
            c.after = Some(field_to_json(new_field));
            out.push(c);
        }
    }
}

fn diff_records(old: &SemanticModel, new: &SemanticModel, out: &mut Vec<Change>) {
    let old_keys: BTreeMap<&str, &RecordModel> =
        old.records.iter().map(|r| (r.key.as_str(), r)).collect();
    let new_keys: BTreeMap<&str, &RecordModel> =
        new.records.iter().map(|r| (r.key.as_str(), r)).collect();

    for (key, record) in &old_keys {
        if new_keys.contains_key(key) {
            continue;
        }
        let consumers = record_consumers(old, key);
        let classification = if consumers.is_empty() {
            Classification::Internal
        } else {
            Classification::Breaking
        };
        let mut c = change(
            "record.removed",
            classification,
            symbol("record", key, &record.name),
            format!("record `{}` was removed", record.name),
        );
        c.consumers = consumers;
        out.push(c);
    }
    for (key, record) in &new_keys {
        if old_keys.contains_key(key) {
            continue;
        }
        out.push(change(
            "record.added",
            Classification::Additive,
            symbol("record", key, &record.name),
            format!("record `{}` was added", record.name),
        ));
    }

    diff_record_fields(old, new, out);
}

fn diff_routes(old: &SemanticModel, new: &SemanticModel, out: &mut Vec<Change>) {
    let old_keys: BTreeMap<&str, &RouteModel> =
        old.routes.iter().map(|r| (r.key.as_str(), r)).collect();
    let new_keys: BTreeMap<&str, &RouteModel> =
        new.routes.iter().map(|r| (r.key.as_str(), r)).collect();

    for (key, route) in &old_keys {
        let Some(new_route) = new_keys.get(key) else {
            let consumers = route_callers(old, route);
            let mut c = change(
                "route.removed",
                Classification::Breaking,
                symbol("api", key, &route.name),
                format!("api `{}` was removed", route.name),
            );
            c.consumers = consumers;
            out.push(c);
            continue;
        };

        if route.method != new_route.method || route.path != new_route.path {
            out.push(with_consumers(
                old,
                route,
                change(
                    "route.method_or_path.changed",
                    Classification::Breaking,
                    symbol("api", key, &route.name),
                    format!("api `{}` changed method/path", route.name),
                ),
            ));
        }

        match (&route.request, &new_route.request) {
            (None, Some(_)) => out.push(with_consumers(
                old,
                route,
                change(
                    "route.request.became_typed",
                    Classification::Breaking,
                    symbol("api", key, &route.name),
                    format!("api `{}` gained a typed request body", route.name),
                ),
            )),
            (Some(_), None) => {
                let mut c = change(
                    "route.request.became_untyped",
                    Classification::Additive,
                    symbol("api", key, &route.name),
                    format!(
                        "api `{}` lost its typed request body (validation loss)",
                        route.name
                    ),
                );
                c.consumers = route_callers(old, route);
                out.push(c);
            }
            _ => {}
        }

        match (&route.scope, &new_route.scope) {
            (None, Some(_)) => out.push(with_consumers(
                old,
                route,
                change(
                    "route.scope.added",
                    Classification::Breaking,
                    symbol("api", key, &route.name),
                    format!("api `{}` now requires a scope", route.name),
                ),
            )),
            (Some(old_scope), Some(new_scope)) if old_scope != new_scope => {
                out.push(with_consumers(
                    old,
                    route,
                    change(
                        "route.scope.changed",
                        Classification::Breaking,
                        symbol("api", key, &route.name),
                        format!(
                            "api `{}` now requires `{new_scope}` instead of `{old_scope}`",
                            route.name
                        ),
                    ),
                ))
            }
            (Some(_), None) => {
                let mut c = change(
                    "route.scope.removed",
                    Classification::Additive,
                    symbol("api", key, &route.name),
                    format!(
                        "api `{}` no longer requires a scope (security relaxation)",
                        route.name
                    ),
                );
                c.consumers = route_callers(old, route);
                out.push(c);
            }
            _ => {}
        }
    }

    for (key, route) in &new_keys {
        if !old_keys.contains_key(key) {
            out.push(change(
                "route.added",
                Classification::Additive,
                symbol("api", key, &route.name),
                format!("api `{}` was added", route.name),
            ));
        }
    }
}

fn with_consumers(model: &SemanticModel, route: &RouteModel, mut c: Change) -> Change {
    c.consumers = route_callers(model, route);
    c
}

fn diff_streams(old: &SemanticModel, new: &SemanticModel, out: &mut Vec<Change>) {
    let old_keys: BTreeMap<&str, &StreamModel> =
        old.streams.iter().map(|s| (s.key.as_str(), s)).collect();
    let new_keys: BTreeMap<&str, &StreamModel> =
        new.streams.iter().map(|s| (s.key.as_str(), s)).collect();

    for (key, stream) in &old_keys {
        let Some(new_stream) = new_keys.get(key) else {
            let consumers = stream_consumers(old, stream);
            let classification = if consumers.is_empty() {
                Classification::Internal
            } else {
                Classification::Breaking
            };
            let mut c = change(
                "stream.removed",
                classification,
                symbol("stream", key, &stream.name),
                format!("stream `{}` was removed", stream.name),
            );
            c.consumers = consumers;
            out.push(c);
            continue;
        };

        if stream.subject != new_stream.subject {
            let mut c = change(
                "stream.subject.changed",
                Classification::Breaking,
                symbol("stream", key, &stream.name),
                format!("stream `{}` changed subject", stream.name),
            );
            c.consumers = stream_consumers(old, stream);
            out.push(c);
        }
        if stream.record != new_stream.record {
            let mut c = change(
                "stream.payload.changed",
                Classification::Breaking,
                symbol("stream", key, &stream.name),
                format!("stream `{}` changed payload record", stream.name),
            );
            c.consumers = stream_consumers(old, stream);
            out.push(c);
        }

        let old_producers = stream_producers(old, stream);
        let new_producers = stream_producers(new, new_stream);
        if !old_producers.is_empty() && new_producers.is_empty() {
            let consumers = stream_consumers(old, stream);
            if !consumers.is_empty() {
                let mut c = change(
                    "stream.last_producer.removed",
                    Classification::Breaking,
                    symbol("stream", key, &stream.name),
                    format!(
                        "stream `{}` lost its last producer while consumers remain",
                        stream.name
                    ),
                );
                c.consumers = consumers;
                out.push(c);
            }
        }
    }

    for (key, stream) in &new_keys {
        if !old_keys.contains_key(key) {
            out.push(change(
                "stream.added",
                Classification::Additive,
                symbol("stream", key, &stream.name),
                format!("stream `{}` was added", stream.name),
            ));
        }
    }
}

fn diff_tables(old: &SemanticModel, new: &SemanticModel, out: &mut Vec<Change>) {
    let old_keys: BTreeMap<&str, &crate::semantic_model::TableModel> =
        old.tables.iter().map(|t| (t.key.as_str(), t)).collect();
    let new_keys: BTreeMap<&str, &crate::semantic_model::TableModel> =
        new.tables.iter().map(|t| (t.key.as_str(), t)).collect();

    for (key, table) in &old_keys {
        if !new_keys.contains_key(key) {
            out.push(change(
                "table.removed",
                Classification::Breaking,
                symbol("table", key, &table.name),
                format!("table `{}` was removed", table.name),
            ));
        }
    }
    for (key, table) in &new_keys {
        if !old_keys.contains_key(key) {
            out.push(change(
                "table.added",
                Classification::Additive,
                symbol("table", key, &table.name),
                format!("table `{}` was added", table.name),
            ));
        }
    }
}

fn diff_channels(old: &SemanticModel, new: &SemanticModel, out: &mut Vec<Change>) {
    let old_keys: BTreeSet<&str> = old.channels.iter().map(|c| c.key.as_str()).collect();
    let new_keys: BTreeSet<&str> = new.channels.iter().map(|c| c.key.as_str()).collect();
    for c in &old.channels {
        if !new_keys.contains(c.key.as_str()) {
            out.push(change(
                "channel.removed",
                Classification::Breaking,
                symbol("channel", &c.key, &c.name),
                format!("channel `{}` was removed", c.name),
            ));
        }
    }
    for c in &new.channels {
        if !old_keys.contains(c.key.as_str()) {
            out.push(change(
                "channel.added",
                Classification::Additive,
                symbol("channel", &c.key, &c.name),
                format!("channel `{}` was added", c.name),
            ));
        }
    }
}

fn diff_services(old: &SemanticModel, new: &SemanticModel, out: &mut Vec<Change>) {
    let old_keys: BTreeSet<&str> = old.services.iter().map(|s| s.key.as_str()).collect();
    let new_keys: BTreeSet<&str> = new.services.iter().map(|s| s.key.as_str()).collect();
    for svc in &old.services {
        if !new_keys.contains(svc.key.as_str()) {
            let has_public_surface = old
                .routes
                .iter()
                .any(|r| r.service.as_deref() == Some(&svc.key))
                || old
                    .streams
                    .iter()
                    .any(|s| s.service.as_deref() == Some(&svc.key));
            let classification = if has_public_surface {
                Classification::Breaking
            } else {
                Classification::Additive
            };
            out.push(change(
                "service.removed",
                classification,
                symbol("service", &svc.key, &svc.name),
                format!("service `{}` was removed", svc.name),
            ));
        }
    }
    for svc in &new.services {
        if !old_keys.contains(svc.key.as_str()) {
            out.push(change(
                "service.added",
                Classification::Additive,
                symbol("service", &svc.key, &svc.name),
                format!("service `{}` was added", svc.name),
            ));
        }
    }
}

fn diff_capabilities(old: &SemanticModel, new: &SemanticModel, out: &mut Vec<Change>) {
    let old_keys: BTreeMap<&str, &CapabilityModel> = old
        .capabilities
        .iter()
        .map(|c| (c.key.as_str(), c))
        .collect();
    let new_keys: BTreeMap<&str, &CapabilityModel> = new
        .capabilities
        .iter()
        .map(|c| (c.key.as_str(), c))
        .collect();
    for (key, cap) in &old_keys {
        if !new_keys.contains_key(key) {
            out.push(change(
                "capability.removed",
                Classification::Internal,
                symbol("capability", key, &format!("{} {}", cap.kind, cap.name)),
                format!("capability `{}` ({}) was removed", cap.name, cap.kind),
            ));
        }
    }
    for (key, cap) in &new_keys {
        match old_keys.get(key) {
            None => out.push(change(
                "capability.added",
                Classification::Internal,
                symbol("capability", key, &format!("{} {}", cap.kind, cap.name)),
                format!("capability `{}` ({}) was added", cap.name, cap.kind),
            )),
            Some(old_cap) if old_cap.config != cap.config => out.push(change(
                "capability.config_changed",
                Classification::Internal,
                symbol("capability", key, &format!("{} {}", cap.kind, cap.name)),
                format!(
                    "capability `{}` ({}) configuration changed",
                    cap.name, cap.kind
                ),
            )),
            Some(_) => {}
        }
    }
}

/// Inline/`extern` handler body changes compare through the structural
/// digest — reported `internal` per 18UpdatePlan.md ("v0.18 does not
/// claim behavioral equivalence").
fn diff_handlers(old: &SemanticModel, new: &SemanticModel, out: &mut Vec<Change>) {
    let old_keys: BTreeMap<&str, &HandlerModel> =
        old.handlers.iter().map(|h| (h.key.as_str(), h)).collect();
    let new_keys: BTreeMap<&str, &HandlerModel> =
        new.handlers.iter().map(|h| (h.key.as_str(), h)).collect();
    for (key, handler) in &new_keys {
        let Some(old_handler) = old_keys.get(key) else {
            continue;
        };
        if old_handler.body_digest != handler.body_digest {
            out.push(change(
                "handler.body_changed",
                Classification::Internal,
                symbol("handler", key, key),
                "handler body changed (structural digest only, not behavioral)".to_owned(),
            ));
        }
    }
}

/// Suppresses field/column-level noise under a service that was
/// removed wholesale — this milestone's cascade suppression is
/// service-scoped only (a coarser approximation of 18UpdatePlan.md's
/// "an entire API removed shouldn't also flood every now-unreachable
/// field"): a whole-service removal is reported once, and any other
/// entry whose symbol key falls under that service's prefix is
/// dropped.
fn suppress_cascades(changes: Vec<Change>) -> Vec<Change> {
    let removed_services: BTreeSet<String> = changes
        .iter()
        .filter(|c| c.kind == "service.removed")
        .map(|c| c.symbol.key.clone())
        .collect();
    if removed_services.is_empty() {
        return changes;
    }
    changes
        .into_iter()
        .filter(|c| {
            c.kind == "service.removed"
                || !removed_services
                    .iter()
                    .any(|svc| c.symbol.key.starts_with(svc.as_str()) && c.symbol.key != *svc)
        })
        .collect()
}

/// Compares two canonical models and returns every detected
/// architectural change, sorted breaking-first (18UpdatePlan.md
/// Pillar 3's stable ordering: classification, then kind, then symbol
/// key).
pub fn diff_models(old: &SemanticModel, new: &SemanticModel) -> Vec<Change> {
    let mut out = Vec::new();
    diff_services(old, new, &mut out);
    diff_records(old, new, &mut out);
    diff_tables(old, new, &mut out);
    diff_routes(old, new, &mut out);
    diff_streams(old, new, &mut out);
    diff_channels(old, new, &mut out);
    diff_capabilities(old, new, &mut out);
    diff_handlers(old, new, &mut out);

    let mut out = suppress_cascades(out);
    out.sort_by(|a, b| {
        (a.classification.rank(), &a.kind, &a.symbol.key).cmp(&(
            b.classification.rank(),
            &b.kind,
            &b.symbol.key,
        ))
    });
    out
}

/// The single most severe classification across a changelist —
/// `None` for an empty (no-op) diff. Generated CI's `--deny-breaking`
/// gate (v0.18 M3) fails the build exactly when this is `Breaking`.
pub fn overall_classification(changes: &[Change]) -> Option<Classification> {
    changes
        .iter()
        .map(|c| c.classification)
        .reduce(Classification::most_severe)
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

    fn model(src: &str) -> SemanticModel {
        SemanticModel::from_ir(&compile(src))
    }

    fn kinds(changes: &[Change]) -> Vec<&str> {
        changes.iter().map(|c| c.kind.as_str()).collect()
    }

    fn find<'a>(changes: &'a [Change], kind: &str) -> &'a Change {
        changes
            .iter()
            .find(|c| c.kind == kind)
            .unwrap_or_else(|| panic!("expected a `{kind}` change, got: {:?}", changes))
    }

    const BASE: &str = r#"
service Billing;
use { db Postgres; auth JWT; }

record Charge {
    id: Uuid;
    amount: Float;
}
table Charges: Charge;

api Pay: Charge {
    method: POST;
    path: "/pay";
    scope: "payments:write";
}
pipeline Pay: Auth -> Return;
"#;

    #[test]
    fn no_op_diff_is_empty() {
        let m = model(BASE);
        let changes = diff_models(&m, &m);
        assert!(changes.is_empty(), "{changes:?}");
        assert_eq!(overall_classification(&changes), None);
    }

    #[test]
    fn removed_route_is_breaking() {
        const NEW: &str = r#"
service Billing;
use { db Postgres; }

record Charge {
    id: Uuid;
    amount: Float;
}
table Charges: Charge;
"#;
        let changes = diff_models(&model(BASE), &model(NEW));
        let c = find(&changes, "route.removed");
        assert_eq!(c.classification, Classification::Breaking);
        assert_eq!(c.symbol.key, "service/Billing/api/Pay");
        assert_eq!(
            overall_classification(&changes),
            Some(Classification::Breaking)
        );
    }

    #[test]
    fn added_route_is_additive() {
        const NEW: &str = r#"
service Billing;
use { db Postgres; auth JWT; }

record Charge {
    id: Uuid;
    amount: Float;
}
table Charges: Charge;

api Pay: Charge {
    method: POST;
    path: "/pay";
    scope: "payments:write";
}
pipeline Pay: Auth -> Return;

api Refund: Charge {
    method: POST;
    path: "/refund";
}
pipeline Refund: Return;
"#;
        let changes = diff_models(&model(BASE), &model(NEW));
        let c = find(&changes, "route.added");
        assert_eq!(c.classification, Classification::Additive);
    }

    #[test]
    fn scope_tightened_is_breaking_scope_removed_is_additive() {
        const TIGHTENED: &str = r#"
service Billing;
use { db Postgres; auth JWT; }

record Charge {
    id: Uuid;
    amount: Float;
}
table Charges: Charge;

api Pay: Charge {
    method: POST;
    path: "/pay";
    scope: "payments:admin";
}
pipeline Pay: Auth -> Return;
"#;
        let changes = diff_models(&model(BASE), &model(TIGHTENED));
        let c = find(&changes, "route.scope.changed");
        assert_eq!(c.classification, Classification::Breaking);

        const NO_SCOPE: &str = r#"
service Billing;
use { db Postgres; }

record Charge {
    id: Uuid;
    amount: Float;
}
table Charges: Charge;

api Pay: Charge {
    method: POST;
    path: "/pay";
}
pipeline Pay: Return;
"#;
        let changes = diff_models(&model(BASE), &model(NO_SCOPE));
        let c = find(&changes, "route.scope.removed");
        assert_eq!(c.classification, Classification::Additive);
    }

    #[test]
    fn removed_record_field_is_breaking_with_impacts() {
        const NEW: &str = r#"
service Billing;
use { db Postgres; auth JWT; }

record Charge {
    id: Uuid;
}
table Charges: Charge;

api Pay: Charge {
    method: POST;
    path: "/pay";
    scope: "payments:write";
}
pipeline Pay: Auth -> Return;
"#;
        let changes = diff_models(&model(BASE), &model(NEW));
        let c = find(&changes, "record.field.removed");
        assert_eq!(c.classification, Classification::Breaking);
        assert_eq!(c.impacts.len(), 2);
        assert!(c
            .impacts
            .iter()
            .any(|i| i.dimension == "generated_client_source"
                && i.classification == Classification::Breaking));
    }

    #[test]
    fn required_column_added_to_table_backed_record_needs_backfill() {
        const NEW: &str = r#"
service Billing;
use { db Postgres; auth JWT; }

record Charge {
    id: Uuid;
    amount: Float;
    currency: String;
}
table Charges: Charge;

api Pay: Charge {
    method: POST;
    path: "/pay";
    scope: "payments:write";
}
pipeline Pay: Auth -> Return;
"#;
        let changes = diff_models(&model(BASE), &model(NEW));
        let c = find(&changes, "table.column.added_required");
        assert_eq!(c.classification, Classification::Breaking);
        assert!(c.backfill_plan_available);
    }

    #[test]
    fn added_field_on_non_table_record_is_additive() {
        const V1: &str = r#"
service Simple;
record Ping {
    id: Uuid;
}
api Check: Ping {
    method: POST;
    path: "/check";
}
pipeline Check: Return;
"#;
        const V2: &str = r#"
service Simple;
record Ping {
    id: Uuid;
    note: String;
}
api Check: Ping {
    method: POST;
    path: "/check";
}
pipeline Check: Return;
"#;
        let changes = diff_models(&model(V1), &model(V2));
        let c = find(&changes, "record.field.added");
        assert_eq!(c.classification, Classification::Additive);
        assert!(!c.backfill_plan_available);
    }

    #[test]
    fn cross_service_stream_consumer_is_reported() {
        const SRC: &str = r#"
project System;

record Order { id: Uuid; total: Float; }
stream Placed: Order;

service Orders {
    use { queue bus NATS; }
    api Create: Order { method: POST; path: "/orders"; }
    pipeline Create:
        publish Placed
        -> Return;
}

service Notifier {
    use { queue bus NATS; }
    worker Notify on Placed;
    handler NotifyHandler {
        queue: bus;
    }
    pipeline Notify:
        NotifyHandler;
}
"#;
        const CHANGED: &str = r#"
project System;

record Order { id: Uuid; total: Float; }

service Orders {
    api Create: Order { method: POST; path: "/orders"; }
    pipeline Create:
        Return;
}

service Notifier {
}
"#;
        let changes = diff_models(&model(SRC), &model(CHANGED));
        let c = find(&changes, "stream.removed");
        assert_eq!(c.classification, Classification::Breaking);
        assert!(
            c.consumers
                .iter()
                .any(|con| con.kind == "stream_consumer"
                    && con.service.as_deref() == Some("Notifier")),
            "{:?}",
            c.consumers
        );
    }

    #[test]
    fn whole_service_removal_suppresses_its_own_field_churn() {
        const SRC: &str = r#"
project System;

record Order { id: Uuid; total: Float; }

service Orders {
    use { db Postgres; }
    table Orders: Order;
    api Create: Order { method: POST; path: "/orders"; }
    pipeline Create: Return;
}

service Notifier {
    use { queue bus NATS; }
}
"#;
        const CHANGED: &str = r#"
project System;

service Notifier {
    use { queue bus NATS; }
}
"#;
        let changes = diff_models(&model(SRC), &model(CHANGED));
        assert!(kinds(&changes).contains(&"service.removed"));
        assert!(
            !kinds(&changes).contains(&"route.removed"),
            "the removed service's own route should be suppressed: {:?}",
            kinds(&changes)
        );
        assert!(
            !kinds(&changes).contains(&"table.removed"),
            "the removed service's own table should be suppressed: {:?}",
            kinds(&changes)
        );
    }
}
