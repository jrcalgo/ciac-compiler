//! Typed data schemas: `record` declarations resolved into a closed set
//! of field types. Records are types, not architectural components, so
//! they live in a side table on the graph rather than as nodes.

use crate::hir::TableId;
use serde::Serialize;

/// Index of a record in [`crate::SystemGraph::records`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct RecordId(pub u32);

/// `cardinality:` on a `Reference<T>` field (v0.16 M2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Cardinality {
    One,
    Many,
}

/// `on_delete:`/`on_update:` on a `Reference<T>` field (v0.16 M2). Every
/// reference states both explicitly — there is no default, so an
/// author-chosen cascade is never accidental.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RefAction {
    Restrict,
    Cascade,
}

/// The closed set of field types (v0.2; `Reference` added v0.16 M2).
/// `Json` remains the untyped escape hatch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type")]
pub enum FieldType {
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
    /// `Reference<T>` (v0.16 M2) — a resolved to-one or to-many relation.
    /// `target` is `T`'s `RecordId`; `table` is the table that
    /// backs `T` (resolved from the field's `references:` attribute).
    /// `unique` marks a to-one reference as one-to-one ownership.
    Reference {
        target: RecordId,
        table: TableId,
        cardinality: Cardinality,
        on_delete: RefAction,
        on_update: RefAction,
        unique: bool,
    },
}

impl FieldType {
    /// Parses a surface type name (`String`, `Int`, ...). Inline enums are
    /// constructed directly by the builder.
    pub fn parse(name: &str) -> Option<FieldType> {
        Some(match name {
            "String" => FieldType::Str,
            "Int" => FieldType::Int,
            "Float" => FieldType::Float,
            "Bool" => FieldType::Bool,
            "Uuid" => FieldType::Uuid,
            "Timestamp" => FieldType::Timestamp,
            "Json" => FieldType::Json,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RecordField {
    pub name: String,
    pub ty: FieldType,
}

/// Distinguishes `record` (plain data) from `error` (v0.7) declarations,
/// mirroring `ciac_syntax::ast::RecordKind` — the IR keeps its own copy
/// rather than depending on the syntax crate's AST types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RecordKind {
    Data,
    Error,
}

/// A resolved `record` or `error` declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Record {
    pub name: String,
    pub fields: Vec<RecordField>,
    pub kind: RecordKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_primitive_type_names() {
        assert_eq!(FieldType::parse("String"), Some(FieldType::Str));
        assert_eq!(FieldType::parse("Timestamp"), Some(FieldType::Timestamp));
        assert_eq!(FieldType::parse("Video"), None);
    }
}
