//! v0.7 M5: incremental SQL migrations for `table` declarations.
//!
//! CRUD resources (`crud X: Record;`) keep the pre-existing
//! create-if-absent behavior (`create_schema`/`ensure_schema`) — this
//! module only concerns the newer, explicitly-typed `table <Name>:
//! <Record>;` declarations, per 07UpdatePlan.md's own scoping.
//!
//! The differ is deliberately additive-only: a new table becomes a
//! `CREATE TABLE`, a new column on an existing table becomes an `ALTER
//! TABLE ... ADD COLUMN`. A column being removed or changing type, or a
//! whole table disappearing, is refused (`SchemaChange`) rather than
//! guessed at — the caller (`ciac build`/`ciac verify`) surfaces that as
//! `ErrorCode::UnsupportedSchemaChange` and expects the user to write a
//! manual migration.

use crate::model::field_sql_type;
use ciac_ir::NormalizedIr;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A table's column list as of some build, in declaration order.
/// Snapshotted into the regeneration manifest so the next build can diff
/// against it without re-deriving it from scratch.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSchema {
    pub columns: Vec<(String, String)>,
}

/// A schema change the additive-only differ refuses to guess at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaChange {
    TableRemoved {
        table: String,
    },
    ColumnRemoved {
        table: String,
        column: String,
    },
    ColumnRetyped {
        table: String,
        column: String,
        old_type: String,
        new_type: String,
    },
}

impl std::fmt::Display for SchemaChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaChange::TableRemoved { table } => write!(
                f,
                "table `{table}` was removed; drop it with a manual migration"
            ),
            SchemaChange::ColumnRemoved { table, column } => write!(
                f,
                "column `{column}` on table `{table}` was removed; drop it with a manual migration"
            ),
            SchemaChange::ColumnRetyped {
                table,
                column,
                old_type,
                new_type,
            } => write!(
                f,
                "column `{table}.{column}` changed type ({old_type} -> {new_type}); \
                 change it with a manual migration"
            ),
        }
    }
}

/// The current table schema, keyed by table name, as declared in the
/// program right now — the "new" side of a [`diff_schema`] call.
pub fn snapshot_schema(ir: &NormalizedIr) -> BTreeMap<String, TableSchema> {
    ir.tables()
        .map(|(_, table)| {
            let record = ir.record(table.record);
            let columns = record
                .fields
                .iter()
                .map(|field| (field.name.clone(), field_sql_type(&field.ty).to_owned()))
                .collect();
            (table.name.clone(), TableSchema { columns })
        })
        .collect()
}

/// Diffs `old` (the schema recorded in the manifest as of the last
/// migration) against `new` (the current program's tables). `Ok(None)`
/// means no migration is needed (the common case on an unchanged
/// build); `Ok(Some(sql))` is the "up" SQL for a new migration file.
pub fn diff_schema(
    old: &BTreeMap<String, TableSchema>,
    new: &BTreeMap<String, TableSchema>,
) -> Result<Option<String>, SchemaChange> {
    for table in old.keys() {
        if !new.contains_key(table) {
            return Err(SchemaChange::TableRemoved {
                table: table.clone(),
            });
        }
    }

    let mut statements = Vec::new();
    for (table, schema) in new {
        match old.get(table) {
            None => statements.push(create_table_sql(table, schema)),
            Some(old_schema) => {
                let old_cols: BTreeMap<&str, &str> = old_schema
                    .columns
                    .iter()
                    .map(|(name, ty)| (name.as_str(), ty.as_str()))
                    .collect();
                for (name, ty) in &schema.columns {
                    match old_cols.get(name.as_str()) {
                        None => statements.push(format!(
                            // Nullable: existing rows have no value for a
                            // brand-new column, and a type-correct default
                            // literal isn't derivable in general (`''`
                            // isn't valid for e.g. BIGINT/BOOLEAN). The
                            // language has no way to declare a field
                            // optional yet, so this is deliberately looser
                            // than the declared schema until existing rows
                            // are backfilled by hand.
                            "ALTER TABLE {table} ADD COLUMN {name} {ty}"
                        )),
                        Some(old_ty) if *old_ty != ty.as_str() => {
                            return Err(SchemaChange::ColumnRetyped {
                                table: table.clone(),
                                column: name.clone(),
                                old_type: (*old_ty).to_owned(),
                                new_type: ty.clone(),
                            });
                        }
                        Some(_) => {}
                    }
                }
                for (name, _) in &old_schema.columns {
                    if !schema.columns.iter().any(|(n, _)| n == name) {
                        return Err(SchemaChange::ColumnRemoved {
                            table: table.clone(),
                            column: name.clone(),
                        });
                    }
                }
            }
        }
    }

    if statements.is_empty() {
        Ok(None)
    } else {
        Ok(Some(format!("{};\n", statements.join(";\n"))))
    }
}

fn create_table_sql(table: &str, schema: &TableSchema) -> String {
    let columns: Vec<String> = schema
        .columns
        .iter()
        .map(|(name, ty)| {
            if name == "id" {
                // MySQL rejects `TEXT PRIMARY KEY` (index keys need a
                // length); a sized VARCHAR holding a stringified UUID
                // is portable across every supported engine (v0.13 M1).
                let ty = if ty == "TEXT" { "VARCHAR(36)" } else { ty };
                format!("    {name} {ty} PRIMARY KEY")
            } else {
                format!("    {name} {ty} NOT NULL")
            }
        })
        .collect();
    format!(
        "CREATE TABLE IF NOT EXISTS {table} (\n{}\n)",
        columns.join(",\n")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn schema(columns: &[(&str, &str)]) -> TableSchema {
        TableSchema {
            columns: columns
                .iter()
                .map(|(n, t)| (n.to_string(), t.to_string()))
                .collect(),
        }
    }

    #[test]
    fn no_change_is_a_no_op() {
        let s = BTreeMap::from([("videos".to_owned(), schema(&[("id", "TEXT")]))]);
        assert_eq!(diff_schema(&s, &s), Ok(None));
    }

    #[test]
    fn new_table_emits_create_table() {
        let old = BTreeMap::new();
        let new = BTreeMap::from([(
            "videos".to_owned(),
            schema(&[("id", "TEXT"), ("title", "TEXT")]),
        )]);
        let sql = diff_schema(&old, &new).unwrap().unwrap();
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS videos"));
        assert!(
            sql.contains("id VARCHAR(36) PRIMARY KEY"),
            "TEXT ids become sized VARCHAR keys (MySQL-portable): {sql}"
        );
        assert!(sql.contains("title TEXT NOT NULL"));
    }

    #[test]
    fn new_column_emits_add_column_only() {
        let old = BTreeMap::from([("videos".to_owned(), schema(&[("id", "TEXT")]))]);
        let new = BTreeMap::from([(
            "videos".to_owned(),
            schema(&[("id", "TEXT"), ("title", "TEXT")]),
        )]);
        let sql = diff_schema(&old, &new).unwrap().unwrap();
        assert!(sql.contains("ALTER TABLE videos ADD COLUMN title TEXT"));
        assert!(!sql.contains("CREATE TABLE"));
    }

    #[test]
    fn removed_column_is_refused() {
        let old = BTreeMap::from([(
            "videos".to_owned(),
            schema(&[("id", "TEXT"), ("title", "TEXT")]),
        )]);
        let new = BTreeMap::from([("videos".to_owned(), schema(&[("id", "TEXT")]))]);
        assert_eq!(
            diff_schema(&old, &new),
            Err(SchemaChange::ColumnRemoved {
                table: "videos".to_owned(),
                column: "title".to_owned(),
            })
        );
    }

    #[test]
    fn retyped_column_is_refused() {
        let old = BTreeMap::from([("videos".to_owned(), schema(&[("count", "BIGINT")]))]);
        let new = BTreeMap::from([("videos".to_owned(), schema(&[("count", "TEXT")]))]);
        assert_eq!(
            diff_schema(&old, &new),
            Err(SchemaChange::ColumnRetyped {
                table: "videos".to_owned(),
                column: "count".to_owned(),
                old_type: "BIGINT".to_owned(),
                new_type: "TEXT".to_owned(),
            })
        );
    }

    #[test]
    fn removed_table_is_refused() {
        let old = BTreeMap::from([("videos".to_owned(), schema(&[("id", "TEXT")]))]);
        let new = BTreeMap::new();
        assert_eq!(
            diff_schema(&old, &new),
            Err(SchemaChange::TableRemoved {
                table: "videos".to_owned(),
            })
        );
    }
}
