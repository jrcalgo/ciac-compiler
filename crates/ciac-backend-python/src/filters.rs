//! Backend-owned minijinja filters rendering neutral model data into
//! Python syntax (v0.22 M2 — `22UpdatePlan.md` Pillar 2 Move 2).
//! Registered once in [`crate::environment`]; nothing per-language is
//! precomputed in `ciac-codegen::model` for what these cover.

use ciac_codegen::model::FieldTypeKind;
use minijinja::value::ViaDeserialize;
use serde::Deserialize;

/// The only piece of a `FieldCtx` these filters need — deserializing
/// just this shape from the full field value (serde ignores the rest)
/// is what lets templates write `{{ field | py_type }}` instead of
/// `{{ field.type_kind | py_type }}`.
#[derive(Deserialize)]
pub(crate) struct HasTypeKind {
    type_kind: FieldTypeKind,
}

/// Python annotation for a field's neutral type, e.g. `str`, `datetime`,
/// `Literal["A", "B"]`. Faithful port of the match `build_record` used
/// to precompute inline (v0.10-era `py_type`).
pub fn py_type(field: ViaDeserialize<HasTypeKind>) -> String {
    py_type_of(field.0.type_kind.clone())
}

pub fn py_out_type(field: ViaDeserialize<HasTypeKind>) -> String {
    py_out_type_of(field.0.type_kind.clone())
}

fn py_type_of(kind: FieldTypeKind) -> String {
    match kind {
        FieldTypeKind::Str | FieldTypeKind::Uuid | FieldTypeKind::Reference { .. } => {
            "str".to_owned()
        }
        FieldTypeKind::Int => "int".to_owned(),
        FieldTypeKind::Float => "float".to_owned(),
        FieldTypeKind::Bool => "bool".to_owned(),
        FieldTypeKind::Timestamp => "datetime".to_owned(),
        FieldTypeKind::Json => "dict[str, Any]".to_owned(),
        FieldTypeKind::Enum { variants, .. } => {
            let literal = variants
                .iter()
                .map(|v| format!("\"{v}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!("Literal[{literal}]")
        }
    }
}

/// Python annotation on the read path: enums come back from storage as
/// their text form, so `Literal[..]` widens to `str` (formerly
/// `FieldCtx::py_out_type`).
fn py_out_type_of(kind: FieldTypeKind) -> String {
    if matches!(kind, FieldTypeKind::Enum { .. }) {
        "str".to_owned()
    } else {
        py_type_of(kind)
    }
}
