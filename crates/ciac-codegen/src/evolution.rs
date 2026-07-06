//! v0.8 M5: record evolution checks across rebuilds.
//!
//! Mirrors `crate::migrations`'s shape one level up: snapshot the
//! current shape from the IR, diff against what the manifest recorded
//! last build, refuse (never guess at) anything destructive. Where
//! `migrations` tracks `table` columns for SQL migrations, this module
//! tracks the field shape of records used *across a service
//! boundary* — a `call` payload, or a stream published in one service
//! and consumed in another — since those are the records two
//! independently-redeployed services must stay wire-compatible on.
//! Records that never cross a boundary (including every record in a
//! single-service program) are untracked: nothing but the two sides of
//! a live edge can ever be broken by a record's evolution.
//!
//! Like `migrations::diff_schema`, this is additive-only: a new field
//! is fine, a removed or retyped field is refused. The language has no
//! optional-field syntax yet (same gap `migrations` already notes), so
//! "fine" can't mean anything stricter than "didn't remove or retype
//! something a live consumer depends on".

use ciac_ir::{Component, EdgeKind, FieldType, MatchArm, NodeId, NormalizedIr, Step, StepKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A boundary record's field shape as of some build. Types are stored
/// as their `Debug` rendering (e.g. `Str`, `Enum { variants: [..] }`),
/// the same "stable string, not the typed enum" choice
/// `migrations::TableSchema` makes for SQL types — plain string
/// equality is all a refuse-don't-guess differ needs, and it sidesteps
/// requiring `serde::Deserialize` on `ciac_ir::FieldType` (currently
/// serialize-only, used for one-way `ciac graph` dumps).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordSchema {
    pub fields: Vec<(String, String)>,
}

fn render_type(ty: &FieldType) -> String {
    format!("{ty:?}")
}

/// A record evolution the additive-only differ refuses to guess at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordChange {
    FieldRemoved {
        record: String,
        field: String,
        consumers: Vec<String>,
    },
    FieldRetyped {
        record: String,
        field: String,
        old_type: String,
        new_type: String,
        consumers: Vec<String>,
    },
}

impl std::fmt::Display for RecordChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordChange::FieldRemoved {
                record,
                field,
                consumers,
            } => write!(
                f,
                "record `{record}` removed field `{field}`, still relied on across a \
                 service boundary by: {} — this would break deserialization there; \
                 revert the field or coordinate the change with a manual migration",
                consumers.join(", ")
            ),
            RecordChange::FieldRetyped {
                record,
                field,
                old_type,
                new_type,
                consumers,
            } => write!(
                f,
                "record `{record}` field `{field}` changed type ({old_type} -> {new_type}), \
                 still relied on across a service boundary by: {} — this would break \
                 deserialization there; revert the field or coordinate the change with a \
                 manual migration",
                consumers.join(", ")
            ),
        }
    }
}

/// The current field shape of every record used across a service
/// boundary right now — the "new" side of a [`diff_records`] call.
/// A record's shape is only tracked while *something* crosses a
/// boundary with it; see the module doc for why that's also why
/// single-service programs never produce anything here.
pub fn snapshot_boundary_records(ir: &NormalizedIr) -> BTreeMap<String, RecordSchema> {
    boundary_consumers(ir)
        .into_keys()
        .filter_map(|name| {
            let id = ir.find_record(&name)?;
            let record = ir.record(id);
            Some((
                name,
                RecordSchema {
                    fields: record
                        .fields
                        .iter()
                        .map(|f| (f.name.clone(), render_type(&f.ty)))
                        .collect(),
                },
            ))
        })
        .collect()
}

/// Diffs `old` (the shape recorded in the manifest as of the last
/// build) against `new` (the current program's boundary records).
/// `Ok(())` means every still-live boundary record is backward
/// compatible; `Err(changes)` lists every violation found (not just
/// the first), each carrying the exact consumer list from `ir`.
///
/// A record present in `old` but absent from `new` is *not* a
/// violation — if nothing crosses a boundary with it anymore there is
/// no live consumer left to break; see the module doc.
pub fn diff_records(
    old: &BTreeMap<String, RecordSchema>,
    new: &BTreeMap<String, RecordSchema>,
    ir: &NormalizedIr,
) -> Result<(), Vec<RecordChange>> {
    let mut changes = Vec::new();
    let consumers = boundary_consumers(ir);

    for (record, old_schema) in old {
        let Some(new_schema) = new.get(record) else {
            continue;
        };
        let old_fields: BTreeMap<&str, &str> = old_schema
            .fields
            .iter()
            .map(|(name, ty)| (name.as_str(), ty.as_str()))
            .collect();
        let new_fields: BTreeMap<&str, &str> = new_schema
            .fields
            .iter()
            .map(|(name, ty)| (name.as_str(), ty.as_str()))
            .collect();
        let names_of = |set: &BTreeSet<String>| -> Vec<String> { set.iter().cloned().collect() };
        let record_consumers = consumers.get(record).map(names_of).unwrap_or_default();

        for (name, old_ty) in &old_fields {
            match new_fields.get(name) {
                None => changes.push(RecordChange::FieldRemoved {
                    record: record.clone(),
                    field: (*name).to_owned(),
                    consumers: record_consumers.clone(),
                }),
                Some(new_ty) if new_ty != old_ty => changes.push(RecordChange::FieldRetyped {
                    record: record.clone(),
                    field: (*name).to_owned(),
                    old_type: (*old_ty).to_owned(),
                    new_type: (*new_ty).to_owned(),
                    consumers: record_consumers.clone(),
                }),
                Some(_) => {}
            }
        }
    }

    if changes.is_empty() {
        Ok(())
    } else {
        Err(changes)
    }
}

/// For every record used across a real service boundary right now,
/// the distinct set of service names that would break if it changed
/// shape: callers of a `call` target's api, and consumers of a stream
/// published from a different service.
fn boundary_consumers(ir: &NormalizedIr) -> BTreeMap<String, BTreeSet<String>> {
    let mut consumers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    // Call boundary: every `call` step (including nested inside a
    // `match` arm) whose target api is owned by a different service
    // than the calling pipeline.
    for pipeline in ir.pipelines.iter() {
        let Some(caller) = pipeline.service else {
            continue;
        };
        for_each_call(&pipeline.steps, &mut |target| {
            let target_node = ir.node(*target);
            let Some(callee) = target_node.service else {
                return;
            };
            if callee == caller {
                return;
            }
            let Some(target_pipeline) = ir.pipeline_of(*target) else {
                return;
            };
            let Some(record_id) = target_pipeline.payload else {
                return;
            };
            let name = ir.record(record_id).name.clone();
            consumers
                .entry(name)
                .or_default()
                .insert(ir.service(caller).name.clone());
        });
    }

    // Stream boundary: any consumer (worker/channel) of a stream whose
    // owning service differs from any of the stream's producers.
    for node in ir.nodes() {
        let Component::Stream {
            record: Some(record_id),
            ..
        } = &node.component
        else {
            continue;
        };
        let producer_services: BTreeSet<_> = ir
            .edges_to(node.id)
            .filter(|e| e.kind == EdgeKind::AsyncMessage)
            .filter_map(|e| ir.node(e.from).service)
            .collect();
        for edge in ir.edges_from(node.id) {
            if edge.kind != EdgeKind::AsyncMessage {
                continue;
            }
            let Some(consumer_service) = ir.node(edge.to).service else {
                continue;
            };
            if producer_services.contains(&consumer_service) {
                continue;
            }
            let name = ir.record(*record_id).name.clone();
            consumers
                .entry(name)
                .or_default()
                .insert(ir.service(consumer_service).name.clone());
        }
    }

    consumers
}

/// Recursively visits every `call` step's target, including ones
/// nested inside `match` arms — a violation missed here would let a
/// real breaking change through undetected, unlike M4's
/// `system_tests` generator (which only needs a top-level publish site
/// to *trigger*, not an exhaustive scan to *detect*).
fn for_each_call(steps: &[Step], f: &mut impl FnMut(&NodeId)) {
    for step in steps {
        match &step.kind {
            StepKind::Call { target } => f(target),
            StepKind::Match { arms, .. } => {
                for MatchArm { steps, .. } in arms {
                    for_each_call(steps, f);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciac_diagnostics::Diagnostics;

    fn compile(src: &str) -> NormalizedIr {
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()))
    }

    const BOUNDARY_SRC_TEMPLATE: &str = r#"
project MediaSystem;
record Video { id: Uuid; {fields} }

service Billing {
    api Charge: Video { method: POST; path: "/charge"; }
    pipeline Charge: CapturePayment -> Return;
}

service UploadApi {
    api Upload: Video { method: PUT; path: "/videos"; }
    pipeline Upload: call Billing.Charge -> Return;
}
"#;

    fn boundary_ir(fields: &str) -> NormalizedIr {
        compile(&BOUNDARY_SRC_TEMPLATE.replace("{fields}", fields))
    }

    #[test]
    fn no_change_is_a_no_op() {
        let ir = boundary_ir("title: String;");
        let schema = snapshot_boundary_records(&ir);
        assert!(diff_records(&schema, &schema, &ir).is_ok());
    }

    #[test]
    fn single_service_program_tracks_nothing() {
        let ir = compile(
            "service Notes;\nuse { db Postgres; }\nrecord Note { id: Uuid; title: String; }\ncrud Note;\n",
        );
        assert!(snapshot_boundary_records(&ir).is_empty());
    }

    #[test]
    fn added_field_is_a_no_op() {
        let old_ir = boundary_ir("");
        let old = snapshot_boundary_records(&old_ir);
        let new_ir = boundary_ir("title: String;");
        let new = snapshot_boundary_records(&new_ir);
        assert!(diff_records(&old, &new, &new_ir).is_ok());
    }

    #[test]
    fn removed_field_is_refused_with_consumer() {
        let old_ir = boundary_ir("title: String;");
        let old = snapshot_boundary_records(&old_ir);
        let new_ir = boundary_ir("");
        let new = snapshot_boundary_records(&new_ir);
        assert_eq!(
            diff_records(&old, &new, &new_ir),
            Err(vec![RecordChange::FieldRemoved {
                record: "Video".to_owned(),
                field: "title".to_owned(),
                consumers: vec!["UploadApi".to_owned()],
            }])
        );
    }

    #[test]
    fn retyped_field_is_refused_with_consumer() {
        let old_ir = boundary_ir("title: String;");
        let old = snapshot_boundary_records(&old_ir);
        let new_ir = boundary_ir("title: Int;");
        let new = snapshot_boundary_records(&new_ir);
        assert_eq!(
            diff_records(&old, &new, &new_ir),
            Err(vec![RecordChange::FieldRetyped {
                record: "Video".to_owned(),
                field: "title".to_owned(),
                old_type: "Str".to_owned(),
                new_type: "Int".to_owned(),
                consumers: vec!["UploadApi".to_owned()],
            }])
        );
    }

    #[test]
    fn record_no_longer_crossing_a_boundary_is_not_flagged() {
        let old_ir = boundary_ir("title: String;");
        let old = snapshot_boundary_records(&old_ir);
        // No more cross-service call at all: Video no longer crosses a
        // boundary, so its removed field must not be flagged.
        let new_ir = compile(
            "project MediaSystem;\nrecord Video { id: Uuid; }\nservice Billing;\nservice UploadApi;\n",
        );
        let new = snapshot_boundary_records(&new_ir);
        assert!(diff_records(&old, &new, &new_ir).is_ok());
    }

    #[test]
    fn enum_variant_set_change_is_a_retype() {
        let old_ir = boundary_ir("status: enum { Ready, Failed };");
        let old = snapshot_boundary_records(&old_ir);
        let new_ir = boundary_ir("status: enum { Ready, Failed, Pending };");
        let new = snapshot_boundary_records(&new_ir);
        assert!(matches!(
            diff_records(&old, &new, &new_ir),
            Err(changes) if matches!(&changes[..], [RecordChange::FieldRetyped { field, .. }] if field == "status")
        ));
    }
}
