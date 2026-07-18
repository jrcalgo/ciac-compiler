//! Backend-owned minijinja filters rendering neutral model data into
//! Rust syntax (v0.22 M2 — `22UpdatePlan.md` Pillar 2 Move 2).
//! Registered once in [`crate::environment`]; nothing per-language is
//! precomputed in `ciac-codegen::model` for what these cover.

use ciac_codegen::model::FieldTypeKind;
use minijinja::value::ViaDeserialize;
use serde::Deserialize;

/// The only piece of a `FieldCtx` these filters need — see the Python
/// backend's identical wrapper for why deserializing just this shape
/// (serde ignores the rest) is what lets templates write
/// `{{ field | rust_type }}` instead of `{{ field.type_kind | rust_type }}`.
#[derive(Deserialize)]
pub(crate) struct HasTypeKind {
    type_kind: FieldTypeKind,
}

/// Rust type for a field's neutral type, e.g. `String`,
/// `chrono::DateTime<chrono::Utc>`, `VideoStatus`. Faithful port of the
/// match `build_record` used to precompute inline (v0.10-era
/// `rust_type`).
pub fn rust_type(field: ViaDeserialize<HasTypeKind>) -> String {
    rust_type_of(field.0.type_kind.clone())
}

pub fn db_rust_type(field: ViaDeserialize<HasTypeKind>) -> String {
    db_rust_type_of(field.0.type_kind.clone())
}

fn rust_type_of(kind: FieldTypeKind) -> String {
    match kind {
        FieldTypeKind::Str | FieldTypeKind::Uuid | FieldTypeKind::Reference { .. } => {
            "String".to_owned()
        }
        FieldTypeKind::Int => "i64".to_owned(),
        FieldTypeKind::Float => "f64".to_owned(),
        FieldTypeKind::Bool => "bool".to_owned(),
        FieldTypeKind::Timestamp => "chrono::DateTime<chrono::Utc>".to_owned(),
        FieldTypeKind::Json => "serde_json::Value".to_owned(),
        FieldTypeKind::Enum { name, .. } => name,
    }
}

/// Rust type as stored in the database (enums are TEXT → `String`);
/// formerly `FieldCtx::db_rust_type`.
fn db_rust_type_of(kind: FieldTypeKind) -> String {
    if matches!(kind, FieldTypeKind::Enum { .. }) {
        "String".to_owned()
    } else {
        rust_type_of(kind)
    }
}
