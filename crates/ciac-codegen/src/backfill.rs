//! v0.18 M6: the expand/backfill/contract ladder (18UpdatePlan.md
//! Pillar 5) for a change the semantic differ recognizes but cannot
//! compute — today, exactly `Change.backfill_plan_available`: a
//! required column added to a table-backed record.
//!
//! The "expand" step needs no new machinery: `ciac-codegen::migrations
//! ::diff_schema` already emits a safely nullable `ALTER TABLE ...
//! ADD COLUMN` for a new column on an existing table (the "deliberately
//! looser than the declared schema" comment on that code path), and an
//! ordinary `ciac build`/`ciac verify` already writes and applies it.
//! This module's job starts after that: confirm the expand migration
//! has actually landed (refuse to plan a backfill for a column the
//! target tree doesn't have yet), generate a seeded, target-native
//! backfill script skeleton, and — only once a human passes
//! `--allow-destructive <plan-id>` for a plan already on record — emit
//! the contract migration that tightens the column to `NOT NULL`,
//! guarded so it refuses to run unless `_ciac_backfills` has a
//! completed row for that exact plan.
//!
//! CIaC never runs a backfill script itself and never decides a plan is
//! "done" on its own — the ledger row is written by the (user-owned,
//! user-edited) backfill script succeeding, not by any CIaC command.

use crate::migrations::TableSchema;
use crate::semantic_diff::{diff_models, Change};
use crate::semantic_model::SemanticModel;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// The ledger table every generated runtime already has the SQL
/// machinery to create (`CREATE TABLE IF NOT EXISTS` is idempotent, so
/// this is safely prepended to the first contract migration a project
/// ever gets) and that a completed backfill script inserts a row into.
pub const LEDGER_TABLE_SQL: &str = "CREATE TABLE IF NOT EXISTS _ciac_backfills (\n    plan_id VARCHAR(64) PRIMARY KEY,\n    table_name VARCHAR(255) NOT NULL,\n    column_name VARCHAR(255) NOT NULL,\n    source_semantic_hash VARCHAR(64) NOT NULL,\n    target_semantic_hash VARCHAR(64) NOT NULL,\n    row_count BIGINT,\n    script_checksum VARCHAR(64) NOT NULL,\n    completed_at TIMESTAMP NOT NULL\n)";

/// One backfill-eligible change, resolved against the current
/// program's real table/column shape. `plan_id` is a short, stable
/// hash of the change's identity plus the before/after semantic
/// hashes — the same plan re-computed from the same baseline and
/// source always gets the same id, so `--allow-destructive <plan-id>`
/// can't be satisfied by an unrelated or stale plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackfillPlan {
    pub plan_id: String,
    pub record: String,
    pub table: String,
    pub column: String,
    pub column_sql_type: String,
    pub semantic_hash_before: String,
    pub semantic_hash_after: String,
    pub message: String,
}

#[derive(Debug)]
pub enum BackfillPlanError {
    /// The change's record has no matching table in the current model
    /// (shouldn't happen if `backfill_plan_available` is set correctly,
    /// but the eligibility check is defensive rather than assumed).
    NoTable { record: String },
    /// The named table isn't in the target tree's schema snapshot at
    /// all — the tree hasn't been built against this program yet.
    TableNotYetBuilt { table: String },
    /// The column doesn't appear in the target tree's table snapshot —
    /// the expand migration (an ordinary `ciac build`/`ciac verify`)
    /// hasn't been applied to this tree yet.
    ColumnNotYetExpanded { table: String, column: String },
}

impl std::fmt::Display for BackfillPlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackfillPlanError::NoTable { record } => {
                write!(
                    f,
                    "record `{record}` has no backing table in the current model"
                )
            }
            BackfillPlanError::TableNotYetBuilt { table } => write!(
                f,
                "table `{table}` isn't in the target tree's schema snapshot yet -- run \
                 `ciac build`/`ciac verify` on it first so the expand migration lands"
            ),
            BackfillPlanError::ColumnNotYetExpanded { table, column } => write!(
                f,
                "column `{column}` on table `{table}` hasn't been expanded in the target tree \
                 yet -- run `ciac build`/`ciac verify` on it first so the expand migration lands"
            ),
        }
    }
}

impl std::error::Error for BackfillPlanError {}

/// Parses `record/<Name>/field/<Field>` into `(Name, Field)` — the key
/// shape `semantic_model`'s field entries always use.
fn parse_field_key(key: &str) -> Option<(&str, &str)> {
    let mut parts = key.split('/');
    if parts.next()? != "record" {
        return None;
    }
    let record = parts.next()?;
    if parts.next()? != "field" {
        return None;
    }
    let field = parts.next()?;
    Some((record, field))
}

fn short_hash(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(16);
    for byte in digest.iter().take(8) {
        use std::fmt::Write as _;
        write!(&mut out, "{byte:02x}").expect("writing to String cannot fail");
    }
    out
}

/// Every `backfill_plan_available` change between `baseline` and
/// `current`, resolved against `current_tables` (the current
/// program's own schema snapshot — the "after" shape the column
/// should have) and validated against `built_tables` (the target
/// tree's own manifest-recorded snapshot — confirms the expand step
/// already landed there).
pub fn plan_backfills(
    baseline: &SemanticModel,
    current: &SemanticModel,
    current_tables: &BTreeMap<String, TableSchema>,
    built_tables: &BTreeMap<String, TableSchema>,
) -> Vec<Result<BackfillPlan, BackfillPlanError>> {
    diff_models(baseline, current)
        .into_iter()
        .filter(|change| change.backfill_plan_available)
        .map(|change| resolve_plan(&change, baseline, current, current_tables, built_tables))
        .collect()
}

fn resolve_plan(
    change: &Change,
    baseline: &SemanticModel,
    current: &SemanticModel,
    current_tables: &BTreeMap<String, TableSchema>,
    built_tables: &BTreeMap<String, TableSchema>,
) -> Result<BackfillPlan, BackfillPlanError> {
    let (record_name, field_name) = parse_field_key(&change.symbol.key).unwrap_or(("", ""));
    let record_key = format!("record/{record_name}");
    let table = current
        .tables
        .iter()
        .find(|t| t.record == record_key)
        .ok_or_else(|| BackfillPlanError::NoTable {
            record: record_name.to_owned(),
        })?;

    // `TableModel.name` is the declared name (`Videos`); the schema
    // snapshot's own map (and the physical DDL/query layer) key by the
    // snake_cased physical name (`videos`, v0.16 M7) — same mismatch
    // `create_table_sql` itself has to reconcile.
    let physical_name = crate::migrations::physical_table_name(&table.name);
    let Some(schema) = current_tables.get(&physical_name) else {
        return Err(BackfillPlanError::TableNotYetBuilt {
            table: table.name.clone(),
        });
    };
    let Some((_, column_ty)) = schema.columns.iter().find(|(n, _)| n == field_name) else {
        return Err(BackfillPlanError::TableNotYetBuilt {
            table: table.name.clone(),
        });
    };

    let built = built_tables
        .get(&physical_name)
        .and_then(|s| s.columns.iter().find(|(n, _)| n == field_name));
    if built.is_none() {
        return Err(BackfillPlanError::ColumnNotYetExpanded {
            table: table.name.clone(),
            column: field_name.to_owned(),
        });
    }

    let plan_id = short_hash(&[
        &table.name,
        field_name,
        &baseline.semantic_hash(),
        &current.semantic_hash(),
    ]);

    Ok(BackfillPlan {
        plan_id,
        record: record_name.to_owned(),
        table: physical_name,
        column: field_name.to_owned(),
        column_sql_type: column_ty.clone(),
        semantic_hash_before: baseline.semantic_hash(),
        semantic_hash_after: current.semantic_hash(),
        message: change.message.clone(),
    })
}

/// The contract migration's SQL: tightens the expanded column to `NOT
/// NULL`, guarded by a portable "assert" (a division by zero, which
/// every supported engine raises as a runtime error) so it refuses to
/// apply unless `_ciac_backfills` already has a completed row for this
/// exact plan. The ledger table's own `CREATE TABLE IF NOT EXISTS` is
/// prepended every time — idempotent, so it's harmless after the first
/// backfill on a project and never needs its own bookkeeping.
pub fn contract_sql(plan: &BackfillPlan) -> String {
    format!(
        "{LEDGER_TABLE_SQL};\n\n\
         -- Refuses to apply unless plan {plan_id} is recorded complete in\n\
         -- _ciac_backfills. A completed row is written by the seeded backfill\n\
         -- script, never by CIaC itself.\n\
         SELECT CASE WHEN NOT EXISTS (\n    \
             SELECT 1 FROM _ciac_backfills WHERE plan_id = '{plan_id}'\n\
         ) THEN 1/0 ELSE 0 END;\n\n\
         ALTER TABLE {table} ALTER COLUMN {column} SET NOT NULL",
        plan_id = plan.plan_id,
        table = plan.table,
        column = plan.column,
    )
}

/// A seeded, target-native backfill script skeleton: typed table/column
/// names and a bounded iteration shape, but no invented conversion
/// expression (the plan's own words) — the actual per-row value comes
/// from the user, and the ledger insert at the end is what satisfies
/// [`contract_sql`]'s guard once the script has actually been run.
/// Returns the script's bare filename (not a path — the caller places
/// it in whichever `migrations/` directory this target's project
/// actually uses, the same convention `ciac build`'s own migration
/// files follow) and its content.
// target-literal-ok: discovered during the v0.22 M1 grep-fence audit,
// outside the six sites the plan's own pre-audit enumerated —
// `python_backfill_script`/`rust_backfill_script` below are genuinely
// per-language template text living in the *shared* crate, the same
// seam-3 pattern `TargetInfo` closes elsewhere. Left as a disclosed,
// deferred finding rather than folded into this milestone
// (22UpdatePlan.md's Risks section: "hidden coupling surfaces late" —
// this is exactly that, and it's annotated rather than silently
// worked around): moving these into per-backend trait methods is
// real, contained follow-up work, tracked but not done here.
pub fn seeded_script(plan: &BackfillPlan, target: &str) -> (String, String) {
    match target {
        "python" => (
            format!("backfill_{}.py", plan.plan_id),
            python_backfill_script(plan),
        ),
        _ => (
            format!("backfill_{}.rs", plan.plan_id),
            rust_backfill_script(plan),
        ),
    }
}

fn python_backfill_script(plan: &BackfillPlan) -> String {
    format!(
        r#""""Backfill script for plan {plan_id} (generated by `ciac backfill plan`).

{message}

This file is yours: fill in the per-row value for `{table}.{column}`
below. Run it once against the target database, by hand, before
passing `--allow-destructive {plan_id}` to `ciac backfill plan` -- the
contract migration refuses to apply until the INSERT at the end of
this script has actually run.
"""
from datetime import datetime, timezone

# TODO: use your own connection setup here (this mirrors the generated
# app's own database URL, but this script is standalone by design).
import os
import sqlalchemy


def backfill(engine: sqlalchemy.engine.Engine) -> int:
    """Returns the number of rows touched."""
    row_count = 0
    with engine.begin() as conn:
        rows = conn.execute(
            sqlalchemy.text("SELECT id FROM {table} WHERE {column} IS NULL")
        ).fetchall()
        for row in rows:
            # TODO: compute the real value for `{column}` here -- CIaC
            # cannot invent this, per row: it depends on your domain.
            value = None
            conn.execute(
                sqlalchemy.text(
                    "UPDATE {table} SET {column} = :value WHERE id = :id"
                ),
                {{"value": value, "id": row.id}},
            )
            row_count += 1
        conn.execute(
            sqlalchemy.text(
                "INSERT INTO _ciac_backfills "
                "(plan_id, table_name, column_name, source_semantic_hash, "
                " target_semantic_hash, row_count, script_checksum, completed_at) "
                "VALUES (:plan_id, :table_name, :column_name, :before, :after, "
                " :row_count, :checksum, :completed_at)"
            ),
            {{
                "plan_id": "{plan_id}",
                "table_name": "{table}",
                "column_name": "{column}",
                "before": "{before}",
                "after": "{after}",
                "row_count": row_count,
                "checksum": "unchecked",  # TODO: hash this file once you're done editing it.
                "completed_at": datetime.now(timezone.utc),
            }},
        )
    return row_count


if __name__ == "__main__":
    url = os.environ["DATABASE_URL"]
    engine = sqlalchemy.create_engine(url)
    touched = backfill(engine)
    print(f"backfilled {{touched}} row(s) for plan {plan_id}")
"#,
        plan_id = plan.plan_id,
        message = plan.message,
        table = plan.table,
        column = plan.column,
        before = plan.semantic_hash_before,
        after = plan.semantic_hash_after,
    )
}

fn rust_backfill_script(plan: &BackfillPlan) -> String {
    format!(
        r#"//! Backfill script for plan {plan_id} (generated by `ciac backfill plan`).
//!
//! {message}
//!
//! This file is yours: fill in the per-row value for `{table}.{column}`
//! below. Run it once against the target database, by hand, before
//! passing `--allow-destructive {plan_id}` to `ciac backfill plan` --
//! the contract migration refuses to apply until the INSERT at the end
//! of this script has actually run. Not wired into the generated
//! binary's own `Cargo.toml` -- run it as a standalone `cargo script`
//! or adapt it into your own bin target.

use sqlx::Row;

#[tokio::main]
async fn main() -> Result<(), sqlx::Error> {{
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
    let pool = sqlx::PgPool::connect(&url).await?;

    let rows = sqlx::query("SELECT id FROM {table} WHERE {column} IS NULL")
        .fetch_all(&pool)
        .await?;
    let mut row_count: i64 = 0;
    for row in &rows {{
        let id: String = row.get("id");
        // TODO: compute the real value for `{column}` here -- CIaC
        // cannot invent this, per row: it depends on your domain.
        let value: Option<String> = None;
        sqlx::query("UPDATE {table} SET {column} = $1 WHERE id = $2")
            .bind(value)
            .bind(id)
            .execute(&pool)
            .await?;
        row_count += 1;
    }}

    sqlx::query(
        "INSERT INTO _ciac_backfills \
         (plan_id, table_name, column_name, source_semantic_hash, \
          target_semantic_hash, row_count, script_checksum, completed_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now())",
    )
    .bind("{plan_id}")
    .bind("{table}")
    .bind("{column}")
    .bind("{before}")
    .bind("{after}")
    .bind(row_count)
    .bind("unchecked") // TODO: hash this file once you're done editing it.
    .execute(&pool)
    .await?;

    println!("backfilled {{row_count}} row(s) for plan {plan_id}");
    Ok(())
}}
"#,
        plan_id = plan.plan_id,
        message = plan.message,
        table = plan.table,
        column = plan.column,
        before = plan.semantic_hash_before,
        after = plan.semantic_hash_after,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrations::snapshot_schema;
    use ciac_diagnostics::{Diagnostics, SourceMap};

    fn model(src: &str) -> (SemanticModel, ciac_ir::NormalizedIr) {
        let mut sources = SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        let ir = ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()));
        (SemanticModel::from_ir(&ir), ir)
    }

    const BASE: &str = r#"
service Billing;
use { db Postgres; }
record Video {
    id: Uuid;
    title: String;
}
table Videos: Video;
api Create: Video { method: POST; path: "/videos"; }
handler CreateHandler(v: Video) -> Video {
    db.insert(Videos, v);
    return v;
}
pipeline Create: CreateHandler -> Return;
"#;

    const NEW: &str = r#"
service Billing;
use { db Postgres; }
record Video {
    id: Uuid;
    title: String;
    duration_seconds: Int;
}
table Videos: Video;
api Create: Video { method: POST; path: "/videos"; }
handler CreateHandler(v: Video) -> Video {
    db.insert(Videos, v);
    return v;
}
pipeline Create: CreateHandler -> Return;
"#;

    #[test]
    fn plans_a_backfill_once_the_column_is_built() {
        let (baseline, _) = model(BASE);
        let (current, ir) = model(NEW);
        let current_tables = snapshot_schema(&ir);

        // Not yet built: refused.
        let plans = plan_backfills(&baseline, &current, &current_tables, &BTreeMap::new());
        assert_eq!(plans.len(), 1);
        assert!(matches!(
            plans[0],
            Err(BackfillPlanError::ColumnNotYetExpanded { .. })
        ));

        // Once the target tree's own snapshot has the column (an
        // ordinary `ciac build` already ran the expand migration):
        let plans = plan_backfills(&baseline, &current, &current_tables, &current_tables);
        assert_eq!(plans.len(), 1);
        let plan = plans[0].as_ref().expect("resolves");
        assert_eq!(plan.table, "videos");
        assert_eq!(plan.column, "duration_seconds");
        assert_eq!(plan.column_sql_type, "BIGINT");
    }

    #[test]
    fn plan_id_is_stable_and_contract_sql_guards_on_it() {
        let (baseline, _) = model(BASE);
        let (current, ir) = model(NEW);
        let current_tables = snapshot_schema(&ir);
        let plans_a = plan_backfills(&baseline, &current, &current_tables, &current_tables);
        let plans_b = plan_backfills(&baseline, &current, &current_tables, &current_tables);
        assert_eq!(
            plans_a[0].as_ref().unwrap().plan_id,
            plans_b[0].as_ref().unwrap().plan_id
        );

        let plan = plans_a[0].as_ref().unwrap();
        let sql = contract_sql(plan);
        assert!(sql.contains(&plan.plan_id));
        assert!(sql.contains("_ciac_backfills"));
        assert!(sql.contains("SET NOT NULL"));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS _ciac_backfills"));
    }

    #[test]
    fn seeded_script_picks_extension_by_target() {
        let (baseline, _) = model(BASE);
        let (current, ir) = model(NEW);
        let current_tables = snapshot_schema(&ir);
        let plans = plan_backfills(&baseline, &current, &current_tables, &current_tables);
        let plan = plans[0].as_ref().unwrap();

        let (py_path, py_src) = seeded_script(plan, "python");
        assert!(py_path.ends_with(".py"));
        assert!(py_src.contains("_ciac_backfills"));

        let (rs_path, rs_src) = seeded_script(plan, "rust");
        assert!(rs_path.ends_with(".rs"));
        assert!(rs_src.contains("_ciac_backfills"));
    }

    #[test]
    fn non_table_backed_field_addition_is_never_plan_eligible() {
        const V1: &str = "service Simple;\nrecord Ping { id: Uuid; }\napi Check: Ping { method: POST; path: \"/check\"; }\npipeline Check: Return;\n";
        const V2: &str = "service Simple;\nrecord Ping { id: Uuid; note: String; }\napi Check: Ping { method: POST; path: \"/check\"; }\npipeline Check: Return;\n";
        let (baseline, _) = model(V1);
        let (current, ir) = model(V2);
        let current_tables = snapshot_schema(&ir);
        let plans = plan_backfills(&baseline, &current, &current_tables, &current_tables);
        assert!(plans.is_empty());
    }
}
