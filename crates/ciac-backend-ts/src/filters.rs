//! Backend-owned minijinja filters rendering neutral model data into
//! TypeScript syntax (v0.23 M2), mirroring
//! `ciac-backend-python`/`-rust`'s own `filters.rs`: a `HasTypeKind`
//! wrapper deserialized via `ViaDeserialize` from just the piece of a
//! `FieldCtx` each filter needs, so templates write `{{ field |
//! ts_type }}` instead of `{{ field.type_kind | ts_type }}`.

use ciac_codegen::model::FieldTypeKind;
use minijinja::value::ViaDeserialize;
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct HasTypeKind {
    type_kind: FieldTypeKind,
}

/// The TS in-memory type for a field's neutral type, e.g. `string`,
/// `Date`, `"A" | "B"`.
pub fn ts_type(field: ViaDeserialize<HasTypeKind>) -> String {
    ts_type_of(&field.0.type_kind)
}

/// The zod schema fragment validating/parsing a field's wire form,
/// e.g. `z.string()`, `z.coerce.date()`, `z.enum(["A", "B"])` — see
/// Pillar 2's type-mapping table (`23UpdatePlan.md`).
pub fn zod_schema(field: ViaDeserialize<HasTypeKind>) -> String {
    zod_schema_of(&field.0.type_kind)
}

fn ts_type_of(kind: &FieldTypeKind) -> String {
    match kind {
        FieldTypeKind::Str | FieldTypeKind::Uuid | FieldTypeKind::Reference { .. } => {
            "string".to_owned()
        }
        FieldTypeKind::Int | FieldTypeKind::Float => "number".to_owned(),
        FieldTypeKind::Bool => "boolean".to_owned(),
        FieldTypeKind::Timestamp => "Date".to_owned(),
        FieldTypeKind::Json => "unknown".to_owned(),
        FieldTypeKind::Enum { variants, .. } => variants
            .iter()
            .map(|v| format!("\"{v}\""))
            .collect::<Vec<_>>()
            .join(" | "),
    }
}

/// The Drizzle column-builder expression for a field's neutral type on
/// a given engine, e.g. `pgCore.text("title")`, `mysqlCore.varchar("id",
/// { length: 36 })`, `sqliteCore.text("payload")`. Calls are qualified
/// through a `import * as pgCore/mysqlCore/sqliteCore from
/// "drizzle-orm/*-core"` namespace import (`models.ts.j2`'s
/// counterpart half of this contract) rather than per-symbol aliased
/// imports, so a table that doesn't happen to use every column kind
/// never trips `@typescript-eslint/no-unused-vars` on an unused
/// aliased builder. id/string/uuid/enum columns are TEXT/VARCHAR(36)
/// everywhere (ids are already stringified UUIDs, matching Python's/
/// Rust's own convention: never a native uuid column),
/// booleans/integers/floats/timestamps get each engine's real typed
/// column so the driver's own value mapping (a `Date` object for a
/// real timestamp column, a JS number for a real integer column)
/// matches what Drizzle's inferred TS type promises, and JSON gets
/// each engine's native json column (`jsonb`/`json`) or `TEXT` on
/// SQLite (no native json type).
pub fn drizzle_column(field: ViaDeserialize<HasTypeKind>, name: String, engine: String) -> String {
    drizzle_column_of(&field.0.type_kind, &name, &engine)
}

fn drizzle_column_of(kind: &FieldTypeKind, name: &str, engine: &str) -> String {
    match (kind, engine) {
        (
            FieldTypeKind::Str
            | FieldTypeKind::Uuid
            | FieldTypeKind::Enum { .. }
            | FieldTypeKind::Reference { .. },
            "postgres",
        ) => format!("pgCore.text(\"{name}\")"),
        (
            FieldTypeKind::Str
            | FieldTypeKind::Uuid
            | FieldTypeKind::Enum { .. }
            | FieldTypeKind::Reference { .. },
            "mysql",
        ) => format!("mysqlCore.text(\"{name}\")"),
        (
            FieldTypeKind::Str
            | FieldTypeKind::Uuid
            | FieldTypeKind::Enum { .. }
            | FieldTypeKind::Reference { .. },
            _,
        ) => format!("sqliteCore.text(\"{name}\")"),
        (FieldTypeKind::Int, "postgres") => format!("pgCore.integer(\"{name}\")"),
        (FieldTypeKind::Int, "mysql") => format!("mysqlCore.int(\"{name}\")"),
        (FieldTypeKind::Int, _) => format!("sqliteCore.integer(\"{name}\")"),
        (FieldTypeKind::Float, "postgres") => format!("pgCore.doublePrecision(\"{name}\")"),
        (FieldTypeKind::Float, "mysql") => format!("mysqlCore.double(\"{name}\")"),
        (FieldTypeKind::Float, _) => format!("sqliteCore.real(\"{name}\")"),
        (FieldTypeKind::Bool, "postgres") => format!("pgCore.boolean(\"{name}\")"),
        (FieldTypeKind::Bool, "mysql") => format!("mysqlCore.boolean(\"{name}\")"),
        (FieldTypeKind::Bool, _) => {
            format!("sqliteCore.integer(\"{name}\", {{ mode: \"boolean\" }})")
        }
        (FieldTypeKind::Timestamp, "postgres") => {
            format!("pgCore.timestamp(\"{name}\", {{ withTimezone: true, mode: \"date\" }})")
        }
        (FieldTypeKind::Timestamp, "mysql") => {
            format!("mysqlCore.timestamp(\"{name}\", {{ mode: \"date\" }})")
        }
        (FieldTypeKind::Timestamp, _) => format!("sqliteCore.text(\"{name}\")"),
        (FieldTypeKind::Json, "postgres") => format!("pgCore.jsonb(\"{name}\")"),
        (FieldTypeKind::Json, "mysql") => format!("mysqlCore.json(\"{name}\")"),
        (FieldTypeKind::Json, _) => format!("sqliteCore.text(\"{name}\", {{ mode: \"json\" }})"),
    }
}

/// The bare SQL type keyword for a field's neutral type on a given
/// engine, e.g. `TEXT`, `INTEGER`, `DOUBLE PRECISION` — used by
/// `db.ts.j2`'s hand-written `CREATE TABLE IF NOT EXISTS` DDL for CRUD
/// resources (Drizzle has no declarative-sync API outside drizzle-kit,
/// so this is the TS analog of Python's `Base.metadata.create_all`/
/// Rust's `ensure_schema_<engine>`). Deliberately kept in lockstep
/// with [`drizzle_column`] above rather than reusing Python's/Rust's
/// own `field_sql_type` (Postgres-flavored, e.g. `BIGINT` for `Int`):
/// each engine's keyword here is exactly the DDL type its matching
/// Drizzle column builder actually declares (`integer()` → `INTEGER`,
/// not `BIGINT`), so the two files never describe two different
/// tables for the same field.
pub fn sql_ddl_type(field: ViaDeserialize<HasTypeKind>, engine: String) -> String {
    sql_ddl_type_of(&field.0.type_kind, &engine)
}

fn sql_ddl_type_of(kind: &FieldTypeKind, engine: &str) -> String {
    match (kind, engine) {
        (
            FieldTypeKind::Str
            | FieldTypeKind::Uuid
            | FieldTypeKind::Enum { .. }
            | FieldTypeKind::Reference { .. },
            _,
        ) => "TEXT",
        (FieldTypeKind::Int, "mysql") => "INT",
        (FieldTypeKind::Int, _) => "INTEGER",
        (FieldTypeKind::Float, "postgres") => "DOUBLE PRECISION",
        (FieldTypeKind::Float, "mysql") => "DOUBLE",
        (FieldTypeKind::Float, _) => "REAL",
        (FieldTypeKind::Bool, "postgres" | "mysql") => "BOOLEAN",
        (FieldTypeKind::Bool, _) => "INTEGER",
        (FieldTypeKind::Timestamp, "postgres") => "TIMESTAMPTZ",
        (FieldTypeKind::Timestamp, "mysql") => "TIMESTAMP",
        (FieldTypeKind::Timestamp, _) => "TEXT",
        (FieldTypeKind::Json, "postgres") => "JSONB",
        (FieldTypeKind::Json, "mysql") => "JSON",
        (FieldTypeKind::Json, _) => "TEXT",
    }
    .to_owned()
}

/// The `id` primary key's bare SQL type per engine — `VARCHAR(36)` on
/// MySQL only (an indexed `TEXT` column needs an explicit length
/// there; Postgres/SQLite don't), matching [`drizzle_column`]'s own
/// `id_column` counterpart in `models.ts.j2`.
pub fn id_ddl_type(engine: &str) -> &'static str {
    if engine == "mysql" {
        "VARCHAR(36)"
    } else {
        "TEXT"
    }
}

fn zod_schema_of(kind: &FieldTypeKind) -> String {
    match kind {
        FieldTypeKind::Str | FieldTypeKind::Reference { .. } => "z.string()".to_owned(),
        FieldTypeKind::Uuid => "z.string().uuid()".to_owned(),
        FieldTypeKind::Int => "z.number().int()".to_owned(),
        FieldTypeKind::Float => "z.number()".to_owned(),
        FieldTypeKind::Bool => "z.boolean()".to_owned(),
        FieldTypeKind::Timestamp => "z.coerce.date()".to_owned(),
        FieldTypeKind::Json => "z.unknown()".to_owned(),
        FieldTypeKind::Enum { variants, .. } => {
            let literal = variants
                .iter()
                .map(|v| format!("\"{v}\""))
                .collect::<Vec<_>>()
                .join(", ");
            format!("z.enum([{literal}])")
        }
    }
}
