//! Typed data schemas: `record` declarations resolved into a closed set
//! of field types. Records are types, not architectural components, so
//! they live in a side table on the graph rather than as nodes.

use serde::Serialize;

/// Index of a record in [`crate::SystemGraph::records`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct RecordId(pub u32);

/// The closed set of field types (v0.2). Nested records are not yet
/// expressible; `Json` is the untyped escape hatch.
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
    Enum { variants: Vec<String> },
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

/// A resolved `record` declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Record {
    pub name: String,
    pub fields: Vec<RecordField>,
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
