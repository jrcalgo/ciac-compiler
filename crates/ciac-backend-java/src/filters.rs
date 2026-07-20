//! Backend-owned minijinja filters rendering neutral model data into
//! Java syntax (following `22UpdatePlan.md` Pillar 2 Move 2's pattern,
//! set by `ciac-backend-python`/`-rust`/`-ts`/`-go`'s own `filters.rs`).
//! Registered once in [`crate::environment`]; nothing per-language is
//! precomputed in `ciac-codegen::model` for what these cover.

use ciac_codegen::model::FieldTypeKind;
use minijinja::value::ViaDeserialize;
use serde::Deserialize;

/// The only piece of a `FieldCtx` these filters need — see the other
/// backends' identical wrapper for why deserializing just this shape
/// (serde ignores the rest) is what lets templates write
/// `{{ field | java_type }}` instead of `{{ field.type_kind | java_type }}`.
#[derive(Deserialize)]
pub(crate) struct HasTypeKind {
    type_kind: FieldTypeKind,
}

/// Java type for a field's neutral type, e.g. `String`, `long`,
/// `Instant`. Record components are always non-optional at the
/// `FieldCtx` level (25UpdatePlan.md Pillar 2's `Option<T>` discussion
/// applies to typed-handler signatures, an HIR-level concern
/// `lower.rs` handles at M4, not to declared record fields), so this
/// never needs `java.util.Optional`/a boxed nullable type.
pub fn java_type(field: ViaDeserialize<HasTypeKind>) -> String {
    java_type_of(field.0.type_kind.clone())
}

fn java_type_of(kind: FieldTypeKind) -> String {
    match kind {
        FieldTypeKind::Str | FieldTypeKind::Uuid | FieldTypeKind::Reference { .. } => {
            "String".to_owned()
        }
        FieldTypeKind::Int => "long".to_owned(),
        FieldTypeKind::Float => "double".to_owned(),
        FieldTypeKind::Bool => "boolean".to_owned(),
        FieldTypeKind::Timestamp => "java.time.Instant".to_owned(),
        FieldTypeKind::Json => "com.fasterxml.jackson.databind.JsonNode".to_owned(),
        FieldTypeKind::Enum { name, .. } => name,
    }
}

/// Java type as stored in the database (enums are TEXT -> `String`);
/// mirrors Go's `go_db_type`/Rust's `db_rust_type`. A `table`/`crud`
/// row is read into this shape, then converted to the wire type at
/// the record boundary.
pub fn java_db_type(field: ViaDeserialize<HasTypeKind>) -> String {
    if matches!(field.0.type_kind, FieldTypeKind::Enum { .. }) {
        "String".to_owned()
    } else {
        java_type_of(field.0.type_kind.clone())
    }
}

/// `true` when a field's Java type is a primitive (`long`/`double`/
/// `boolean`) rather than a reference type — primitives can never hold
/// a JSON `null`, so decode sites need this to know whether a
/// presence/null check even applies at the Jackson-conversion layer
/// (a primitive record component already can't receive `null`;
/// Jackson itself throws on that attempt, which `requireKeys`'s own
/// explicit-null check preempts with the shared error shape instead of
/// a raw Jackson stack trace).
pub fn java_is_primitive(field: ViaDeserialize<HasTypeKind>) -> bool {
    matches!(
        field.0.type_kind,
        FieldTypeKind::Int | FieldTypeKind::Float | FieldTypeKind::Bool
    )
}

/// Java `camelCase` identifier (record component / local variable
/// name) for a `snake_case` field name, e.g. `order_id` -> `orderId`.
/// CIaC source fields are snake_case; Java convention is camelCase for
/// fields/locals, PascalCase for types (see [`java_pascal`]).
pub fn java_camel(input: String) -> String {
    use heck::ToLowerCamelCase;
    input.to_lower_camel_case()
}

/// Java `PascalCase` identifier (class/record/enum name) for a
/// `snake_case`/`kebab-case` input.
pub fn java_pascal(input: String) -> String {
    use heck::ToPascalCase;
    input.to_pascal_case()
}

/// `true` for a `Uuid`-typed field — the one format constraint Pillar
/// 2's own decode discipline checks (Java has no `validate:"uuid4"`
/// struct tag; the generated decode helper calls `Schemas.requireUuid`
/// directly for fields this filter marks).
pub fn java_is_uuid(field: ViaDeserialize<HasTypeKind>) -> bool {
    matches!(field.0.type_kind, FieldTypeKind::Uuid)
}
