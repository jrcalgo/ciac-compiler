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
use ciac_ir::{Cardinality, FieldType, NormalizedIr, RefAction};
use heck::ToSnakeCase;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The physical SQL identifier for a declared table/record name.
/// Both backends already query tables by their snake-cased name (the
/// Python ORM's `__tablename__`, the Rust backend's `table_snake`) —
/// this keeps the migration DDL's `CREATE TABLE`/`REFERENCES` naming
/// consistent with that, rather than the literal declared identifier.
/// A single-word name (`Orders`) round-trips unchanged; a multi-word
/// one (`OrderAudits`) doesn't — Postgres case-folds an unquoted
/// identifier to lowercase without ever inserting a separator, so
/// `OrderAudits` used verbatim becomes `orderaudits`, not the
/// `order_audits` the ORM/query code actually addresses.
fn physical_table_name(name: &str) -> String {
    name.to_snake_case()
}

/// One foreign key a table's schema carries (v0.16 M3): a `cardinality:
/// one` `Reference<T>` field's column, or (for a compiler-owned link
/// table) either of its two FK columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKeySchema {
    pub name: String,
    pub column: String,
    pub target_table: String,
    pub on_delete: String,
    pub on_update: String,
}

/// A table's column list as of some build, in declaration order.
/// Snapshotted into the regeneration manifest so the next build can diff
/// against it without re-deriving it from scratch. `foreign_keys`/
/// `unique_columns`/`is_link_table` (v0.16 M3) default to empty/`false`
/// so a manifest written before v0.16 still deserializes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TableSchema {
    pub columns: Vec<(String, String)>,
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKeySchema>,
    /// Column names carrying a `unique: true` constraint (today, only
    /// ever a to-one reference's FK column — scalar `unique`/`index`
    /// attributes are v0.16 M4).
    #[serde(default)]
    pub unique_columns: Vec<String>,
    /// True for a compiler-owned many-relation link table: its primary
    /// key is the composite `(source_id, target_id)`, not a single `id`
    /// column, so `create_table_sql` renders it differently.
    #[serde(default)]
    pub is_link_table: bool,
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
    /// v0.16 M3: a `Reference<T>` field's FK column appeared on a table
    /// that already existed. Every reference is required (no optional/
    /// `SET NULL` in v0.16), so this can never be a safe nullable
    /// `ALTER TABLE ADD COLUMN` the way an ordinary new column is —
    /// existing rows would have no value to satisfy it. This
    /// unconditional refusal is also what makes "add FK to an existing
    /// SQLite table" (a named risk in 16UpdatePlan.md's safety matrix)
    /// moot: it's refused for every engine, not special-cased for one.
    ForeignKeyAddedToExistingTable {
        table: String,
        column: String,
    },
    /// v0.16 M3: an existing FK's target/action changed, or it was
    /// removed outright — always a retype/removal in the additive-only
    /// model, never guessed at.
    ForeignKeyChanged {
        table: String,
        column: String,
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
            SchemaChange::ForeignKeyAddedToExistingTable { table, column } => write!(
                f,
                "`{table}.{column}` is a new required reference on an existing table; there's \
                 no safe default for existing rows — add it with a manual migration that backfills \
                 a value first"
            ),
            SchemaChange::ForeignKeyChanged { table, column } => write!(
                f,
                "reference `{table}.{column}` changed target, cardinality, or referential \
                 action; change it with a manual migration"
            ),
        }
    }
}

fn ref_action_sql(action: RefAction) -> &'static str {
    match action {
        RefAction::Restrict => "RESTRICT",
        RefAction::Cascade => "CASCADE",
    }
}

/// Deterministic constraint name, truncated (with a stable semantic
/// hash) when it would exceed a conservative cross-engine limit —
/// improving the truncation scheme later is then a name-only change,
/// not a drop-and-add migration, since the differ compares constraint
/// *meaning* (target/action), not the rendered name.
fn constraint_name(prefix: &str, parts: &[&str]) -> String {
    let mut name = format!("{prefix}_{}", parts.join("_"));
    if name.len() > 63 {
        let hash = parts.iter().fold(0u64, |acc, p| {
            p.bytes()
                .fold(acc, |a, b| a.wrapping_mul(31).wrapping_add(b as u64))
        });
        name = format!("{prefix}_{hash:x}");
    }
    name
}

/// The current table schema, keyed by table name, as declared in the
/// program right now — the "new" side of a [`diff_schema`] call.
/// `cardinality: one` references add a column to the source table;
/// `cardinality: many` references add a separate, deterministically
/// named compiler-owned link table (v0.16 M3) rather than a column.
pub fn snapshot_schema(ir: &NormalizedIr) -> BTreeMap<String, TableSchema> {
    let mut out = BTreeMap::new();
    for (_, table) in ir.tables() {
        let record = ir.record(table.record);
        let mut columns = Vec::new();
        let mut foreign_keys = Vec::new();
        let mut unique_columns = Vec::new();
        for field in &record.fields {
            match &field.ty {
                FieldType::Reference {
                    table: target_table_id,
                    cardinality: Cardinality::One,
                    on_delete,
                    on_update,
                    unique,
                    ..
                } => {
                    let table_name = physical_table_name(&table.name);
                    let target_table = ir.table(*target_table_id);
                    let target_name = physical_table_name(&target_table.name);
                    let column = format!("{}_id", field.name);
                    columns.push((column.clone(), field_sql_type(&FieldType::Uuid).to_owned()));
                    foreign_keys.push(ForeignKeySchema {
                        name: constraint_name("fk", &[&table_name, &column, &target_name]),
                        column: column.clone(),
                        target_table: target_name,
                        on_delete: ref_action_sql(*on_delete).to_owned(),
                        on_update: ref_action_sql(*on_update).to_owned(),
                    });
                    if *unique {
                        unique_columns.push(column);
                    }
                }
                FieldType::Reference {
                    table: target_table_id,
                    cardinality: Cardinality::Many,
                    on_delete,
                    on_update,
                    ..
                } => {
                    let table_name = physical_table_name(&table.name);
                    let target_table = ir.table(*target_table_id);
                    let target_name = physical_table_name(&target_table.name);
                    let link_name =
                        format!("{}__{}", table_name, field.name.as_str().to_snake_case());
                    let uuid_ty = field_sql_type(&FieldType::Uuid).to_owned();
                    out.insert(
                        link_name.clone(),
                        TableSchema {
                            columns: vec![
                                ("source_id".to_owned(), uuid_ty.clone()),
                                ("target_id".to_owned(), uuid_ty),
                            ],
                            foreign_keys: vec![
                                ForeignKeySchema {
                                    name: constraint_name(
                                        "fk",
                                        &[&link_name, "source_id", &table_name],
                                    ),
                                    column: "source_id".to_owned(),
                                    target_table: table_name.clone(),
                                    // Deleting the source always removes
                                    // compiler-owned links (16UpdatePlan.md
                                    // Pillar 1) — not user-configurable.
                                    on_delete: "CASCADE".to_owned(),
                                    on_update: "CASCADE".to_owned(),
                                },
                                ForeignKeySchema {
                                    name: constraint_name(
                                        "fk",
                                        &[&link_name, "target_id", &target_name],
                                    ),
                                    column: "target_id".to_owned(),
                                    target_table: target_name,
                                    on_delete: ref_action_sql(*on_delete).to_owned(),
                                    on_update: ref_action_sql(*on_update).to_owned(),
                                },
                            ],
                            unique_columns: Vec::new(),
                            is_link_table: true,
                        },
                    );
                }
                other => columns.push((field.name.clone(), field_sql_type(other).to_owned())),
            }
        }
        out.insert(
            physical_table_name(&table.name),
            TableSchema {
                columns,
                foreign_keys,
                unique_columns,
                is_link_table: false,
            },
        );
    }
    out
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
    let mut new_tables = Vec::new();
    for (table, schema) in new {
        match old.get(table) {
            None => new_tables.push((table.as_str(), schema)),
            Some(old_schema) => {
                let old_cols: BTreeMap<&str, &str> = old_schema
                    .columns
                    .iter()
                    .map(|(name, ty)| (name.as_str(), ty.as_str()))
                    .collect();
                let new_fks: BTreeMap<&str, &ForeignKeySchema> = schema
                    .foreign_keys
                    .iter()
                    .map(|fk| (fk.column.as_str(), fk))
                    .collect();
                let old_fks: BTreeMap<&str, &ForeignKeySchema> = old_schema
                    .foreign_keys
                    .iter()
                    .map(|fk| (fk.column.as_str(), fk))
                    .collect();
                for (name, ty) in &schema.columns {
                    match old_cols.get(name.as_str()) {
                        None => {
                            // v0.16 M3: a `Reference<T>` field's column is
                            // always required (no optional references in
                            // v0.16), so — unlike an ordinary new column,
                            // safely nullable for existing rows — a new FK
                            // column on an existing table has no safe
                            // default and is refused outright.
                            if new_fks.contains_key(name.as_str()) {
                                return Err(SchemaChange::ForeignKeyAddedToExistingTable {
                                    table: table.clone(),
                                    column: name.clone(),
                                });
                            }
                            statements.push(format!(
                                // Nullable: existing rows have no value for
                                // a brand-new column, and a type-correct
                                // default literal isn't derivable in
                                // general (`''` isn't valid for e.g.
                                // BIGINT/BOOLEAN). The language has no way
                                // to declare a field optional yet, so this
                                // is deliberately looser than the declared
                                // schema until existing rows are backfilled
                                // by hand.
                                "ALTER TABLE {table} ADD COLUMN {name} {ty}"
                            ));
                        }
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
                for (column, new_fk) in &new_fks {
                    if let Some(old_fk) = old_fks.get(column) {
                        if old_fk.target_table != new_fk.target_table
                            || old_fk.on_delete != new_fk.on_delete
                            || old_fk.on_update != new_fk.on_update
                        {
                            return Err(SchemaChange::ForeignKeyChanged {
                                table: table.clone(),
                                column: (*column).to_owned(),
                            });
                        }
                    }
                }
            }
        }
    }

    let mut creates: Vec<String> = order_new_tables(new_tables)
        .into_iter()
        .map(|(table, schema)| create_table_sql(table, schema))
        .collect();
    creates.append(&mut statements);
    let statements = creates;

    if statements.is_empty() {
        Ok(None)
    } else {
        Ok(Some(format!("{};\n", statements.join(";\n"))))
    }
}

/// Orders newly-created tables so a table's `CREATE TABLE` never
/// precedes a table its foreign keys reference — plain `BTreeMap`
/// (alphabetical) order breaks the moment a referencing table's name
/// sorts before its target's (e.g. `LineItems` referencing `Orders`).
/// Only dependencies on another table in this same new-table batch
/// matter; a reference to a table that already existed before this
/// migration needs no reordering. Cycles among direct (non-link)
/// foreign keys are rejected earlier by sema's relation-graph check
/// (`find_reference_cycles`), so this never needs to break one — the
/// leftover-node fallback only guards against that invariant somehow
/// not holding, rather than a case this function expects to hit.
fn order_new_tables<'a>(
    tables: Vec<(&'a str, &'a TableSchema)>,
) -> Vec<(&'a str, &'a TableSchema)> {
    let mut remaining: BTreeMap<&str, &TableSchema> = tables.iter().copied().collect();
    let mut ordered = Vec::with_capacity(tables.len());
    while !remaining.is_empty() {
        let ready: Vec<&str> = remaining
            .iter()
            .filter(|(_, schema)| {
                schema
                    .foreign_keys
                    .iter()
                    .all(|fk| !remaining.contains_key(fk.target_table.as_str()))
            })
            .map(|(name, _)| *name)
            .collect();
        if ready.is_empty() {
            // Defensive fallback only: emit whatever remains in
            // deterministic (alphabetical) order rather than looping
            // forever.
            for (name, schema) in remaining {
                ordered.push((name, schema));
            }
            break;
        }
        for name in ready {
            ordered.push((name, remaining.remove(name).unwrap()));
        }
    }
    ordered
}

fn create_table_sql(table: &str, schema: &TableSchema) -> String {
    let fk_columns: std::collections::HashSet<&str> = schema
        .foreign_keys
        .iter()
        .map(|fk| fk.column.as_str())
        .collect();
    // MySQL rejects `TEXT PRIMARY KEY`/`TEXT` in an indexed constraint
    // (index keys need a length); a sized VARCHAR holding a stringified
    // UUID is portable across every supported engine (v0.13 M1) — needed
    // for the `id` primary key and, since v0.16 M3, every FK column too
    // (both are always Uuid-typed).
    let sized = |ty: &str| -> String {
        if ty == "TEXT" {
            "VARCHAR(36)".to_owned()
        } else {
            ty.to_owned()
        }
    };
    let mut lines: Vec<String> = schema
        .columns
        .iter()
        .map(|(name, ty)| {
            if !schema.is_link_table && name == "id" {
                format!("    {name} {} PRIMARY KEY", sized(ty))
            } else if fk_columns.contains(name.as_str()) {
                format!("    {name} {} NOT NULL", sized(ty))
            } else {
                format!("    {name} {ty} NOT NULL")
            }
        })
        .collect();
    if schema.is_link_table {
        let pk_cols: Vec<&str> = schema.columns.iter().map(|(n, _)| n.as_str()).collect();
        lines.push(format!("    PRIMARY KEY ({})", pk_cols.join(", ")));
    }
    for column in &schema.unique_columns {
        lines.push(format!(
            "    CONSTRAINT {} UNIQUE ({column})",
            constraint_name("uq", &[table, column])
        ));
    }
    for fk in &schema.foreign_keys {
        lines.push(format!(
            "    CONSTRAINT {} FOREIGN KEY ({}) REFERENCES {} (id) ON DELETE {} ON UPDATE {}",
            fk.name, fk.column, fk.target_table, fk.on_delete, fk.on_update
        ));
    }
    format!(
        "CREATE TABLE IF NOT EXISTS {table} (\n{}\n)",
        lines.join(",\n")
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
            ..Default::default()
        }
    }

    fn fk(
        name: &str,
        column: &str,
        target_table: &str,
        on_delete: &str,
        on_update: &str,
    ) -> ForeignKeySchema {
        ForeignKeySchema {
            name: name.to_owned(),
            column: column.to_owned(),
            target_table: target_table.to_owned(),
            on_delete: on_delete.to_owned(),
            on_update: on_update.to_owned(),
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

    fn with_fk(columns: &[(&str, &str)], foreign_keys: Vec<ForeignKeySchema>) -> TableSchema {
        TableSchema {
            foreign_keys,
            ..schema(columns)
        }
    }

    #[test]
    fn new_table_with_reference_emits_fk_constraint() {
        let old = BTreeMap::new();
        let new = BTreeMap::from([(
            "orders".to_owned(),
            with_fk(
                &[("id", "TEXT"), ("customer_id", "TEXT")],
                vec![fk(
                    "fk_orders_customer_id_customers",
                    "customer_id",
                    "customers",
                    "RESTRICT",
                    "CASCADE",
                )],
            ),
        )]);
        let sql = diff_schema(&old, &new).unwrap().unwrap();
        assert!(sql.contains("customer_id VARCHAR(36) NOT NULL"));
        assert!(sql.contains(
            "CONSTRAINT fk_orders_customer_id_customers FOREIGN KEY (customer_id) \
             REFERENCES customers (id) ON DELETE RESTRICT ON UPDATE CASCADE"
        ));
    }

    #[test]
    fn unique_reference_emits_unique_constraint() {
        let old = BTreeMap::new();
        let mut t = with_fk(
            &[("id", "TEXT"), ("profile_id", "TEXT")],
            vec![fk(
                "fk_users_profile_id_profiles",
                "profile_id",
                "profiles",
                "CASCADE",
                "CASCADE",
            )],
        );
        t.unique_columns = vec!["profile_id".to_owned()];
        let new = BTreeMap::from([("users".to_owned(), t)]);
        let sql = diff_schema(&old, &new).unwrap().unwrap();
        assert!(sql.contains("CONSTRAINT uq_users_profile_id UNIQUE (profile_id)"));
    }

    #[test]
    fn link_table_gets_composite_primary_key_and_both_fks() {
        let old = BTreeMap::new();
        let mut link = with_fk(
            &[("source_id", "TEXT"), ("target_id", "TEXT")],
            vec![
                fk(
                    "fk_orders__tags_source_id_orders",
                    "source_id",
                    "orders",
                    "CASCADE",
                    "CASCADE",
                ),
                fk(
                    "fk_orders__tags_target_id_tags",
                    "target_id",
                    "tags",
                    "CASCADE",
                    "CASCADE",
                ),
            ],
        );
        link.is_link_table = true;
        let new = BTreeMap::from([("orders__tags".to_owned(), link)]);
        let sql = diff_schema(&old, &new).unwrap().unwrap();
        assert!(sql.contains("PRIMARY KEY (source_id, target_id)"));
        assert!(!sql.contains("source_id VARCHAR(36) PRIMARY KEY"));
        assert!(sql.contains("REFERENCES orders (id)"));
        assert!(sql.contains("REFERENCES tags (id)"));
    }

    #[test]
    fn new_required_reference_on_existing_table_is_refused() {
        let old = BTreeMap::from([("orders".to_owned(), schema(&[("id", "TEXT")]))]);
        let new = BTreeMap::from([(
            "orders".to_owned(),
            with_fk(
                &[("id", "TEXT"), ("customer_id", "TEXT")],
                vec![fk(
                    "fk_orders_customer_id_customers",
                    "customer_id",
                    "customers",
                    "RESTRICT",
                    "CASCADE",
                )],
            ),
        )]);
        assert_eq!(
            diff_schema(&old, &new),
            Err(SchemaChange::ForeignKeyAddedToExistingTable {
                table: "orders".to_owned(),
                column: "customer_id".to_owned(),
            })
        );
    }

    #[test]
    fn changed_reference_target_is_refused() {
        let old = BTreeMap::from([(
            "orders".to_owned(),
            with_fk(
                &[("id", "TEXT"), ("customer_id", "TEXT")],
                vec![fk(
                    "fk_orders_customer_id_customers",
                    "customer_id",
                    "customers",
                    "RESTRICT",
                    "CASCADE",
                )],
            ),
        )]);
        let new = BTreeMap::from([(
            "orders".to_owned(),
            with_fk(
                &[("id", "TEXT"), ("customer_id", "TEXT")],
                vec![fk(
                    "fk_orders_customer_id_accounts",
                    "customer_id",
                    "accounts",
                    "RESTRICT",
                    "CASCADE",
                )],
            ),
        )]);
        assert_eq!(
            diff_schema(&old, &new),
            Err(SchemaChange::ForeignKeyChanged {
                table: "orders".to_owned(),
                column: "customer_id".to_owned(),
            })
        );
    }

    #[test]
    fn changed_on_delete_action_is_refused() {
        let old = BTreeMap::from([(
            "orders".to_owned(),
            with_fk(
                &[("id", "TEXT"), ("customer_id", "TEXT")],
                vec![fk(
                    "fk_orders_customer_id_customers",
                    "customer_id",
                    "customers",
                    "RESTRICT",
                    "CASCADE",
                )],
            ),
        )]);
        let new = BTreeMap::from([(
            "orders".to_owned(),
            with_fk(
                &[("id", "TEXT"), ("customer_id", "TEXT")],
                vec![fk(
                    "fk_orders_customer_id_customers",
                    "customer_id",
                    "customers",
                    "CASCADE",
                    "CASCADE",
                )],
            ),
        )]);
        assert_eq!(
            diff_schema(&old, &new),
            Err(SchemaChange::ForeignKeyChanged {
                table: "orders".to_owned(),
                column: "customer_id".to_owned(),
            })
        );
    }

    #[test]
    fn unchanged_reference_is_a_no_op() {
        let t = with_fk(
            &[("id", "TEXT"), ("customer_id", "TEXT")],
            vec![fk(
                "fk_orders_customer_id_customers",
                "customer_id",
                "customers",
                "RESTRICT",
                "CASCADE",
            )],
        );
        let s = BTreeMap::from([("orders".to_owned(), t)]);
        assert_eq!(diff_schema(&s, &s), Ok(None));
    }

    #[test]
    fn long_constraint_name_is_truncated_deterministically() {
        let long_table = "a".repeat(40);
        let long_target = "b".repeat(40);
        let name = constraint_name("fk", &[&long_table, "some_column", &long_target]);
        assert!(name.len() <= 63);
        // Same inputs always truncate to the same name (determinism is
        // what makes drop/add migrations reviewable).
        assert_eq!(
            name,
            constraint_name("fk", &[&long_table, "some_column", &long_target])
        );
    }
}
