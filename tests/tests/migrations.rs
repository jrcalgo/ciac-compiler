//! v0.7 M5: the `table` migration differ, exercised against real compiled
//! programs (not hand-built `TableSchema` maps, which `ciac-codegen`'s own
//! unit tests already cover). Simulates what `ciac build`/`ciac verify`
//! do across successive builds: `snapshot_schema` + `diff_schema` feeding
//! a `Manifest`'s `tables`/`next_migration_seq`, with the resulting
//! migration files added as `Seeded` so a later build that stops
//! re-emitting an old one leaves it in place (`RegenStatus::OrphanLeft`)
//! instead of deleting it.

use ciac_codegen::manifest::{build_manifest, load_manifest, write_manifest};
use ciac_codegen::migrations::{diff_schema, snapshot_schema, SchemaChange};
use ciac_codegen::regen::{
    apply_regeneration, plan_regeneration, ApplyMode, RegenMode, RegenStatus,
};
use ciac_codegen::{Backend, GenOptions};
use ciac_integration_tests::compile;

const SCHEMA_V1: &str = r#"
service TableExample;

use {
    db Postgres;
}

record Video {
    id: Uuid;
    title: String;
}

table Videos: Video;
"#;

const SCHEMA_V2: &str = r#"
service TableExample;

use {
    db Postgres;
}

record Video {
    id: Uuid;
    title: String;
    summary: String;
}

table Videos: Video;
"#;

const SCHEMA_V3_DROPPED: &str = r#"
service TableExample;

use {
    db Postgres;
}

record Video {
    id: Uuid;
}

table Videos: Video;
"#;

/// Applies one build's worth of migration bookkeeping the way
/// `ciac`'s `commands::generate` does, returning the migration SQL (if
/// any) so the test can assert on it directly.
fn migrate_step(dir: &std::path::Path, src: &str) -> Result<Option<String>, SchemaChange> {
    let (ir, diags) = compile(src);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");

    let backend = ciac_backend_rust::RustBackend;
    let mut project = backend
        .generate(&ir, &GenOptions::default())
        .expect("rust backend generates");

    let manifest_file = ciac_codegen::manifest::manifest_path(dir);
    let previous = manifest_file
        .exists()
        .then(|| load_manifest(dir).expect("manifest reads"));
    let old_tables = previous
        .as_ref()
        .map(|m| m.tables.clone())
        .unwrap_or_default();
    let next_seq = previous.as_ref().map_or(1, |m| m.next_migration_seq);

    let new_tables = snapshot_schema(&ir);
    let sql = diff_schema(&old_tables, &new_tables)?;
    let next_seq = if let Some(sql) = &sql {
        project.add_seeded_file(
            format!("migrations/{next_seq:04}_migration.sql"),
            sql.clone(),
        );
        next_seq + 1
    } else {
        next_seq
    };

    let plan = plan_regeneration(&project, dir, previous.as_ref(), RegenMode::Normal)
        .expect("plan succeeds");
    apply_regeneration(&plan, dir, ApplyMode::Full).expect("apply succeeds");

    // Nothing here should ever land in an error/conflict state (this is a
    // clean regenerate, not a hand-edited directory) — `New`/`Update` are
    // expected whenever the source itself changed between calls.
    let broken: Vec<_> = plan.entries.iter().filter(|e| e.is_error()).collect();
    assert!(broken.is_empty(), "unexpected regen errors: {broken:?}");

    let mut manifest = build_manifest(&project, "test", "src", backend.id());
    manifest.tables = new_tables;
    manifest.next_migration_seq = next_seq;
    write_manifest(dir, &manifest).expect("manifest writes");

    Ok(sql)
}

/// Asserts a build against `src` is fully converged against `dir`: every
/// entry is either byte-identical (`Unchanged`) or a past migration file
/// intentionally left in place (`OrphanLeft`) — i.e. a third build with
/// no source change is a true no-op.
fn assert_converged(dir: &std::path::Path, src: &str) {
    let (ir, diags) = compile(src);
    assert!(!diags.has_errors(), "unexpected: {:?}", diags.codes());
    let ir = ir.expect("well-typed program produces IR");
    let project = ciac_backend_rust::RustBackend
        .generate(&ir, &GenOptions::default())
        .expect("rust backend generates");
    let manifest = load_manifest(dir).expect("manifest reads");
    let plan = plan_regeneration(&project, dir, Some(&manifest), RegenMode::Normal)
        .expect("plan succeeds");
    let not_converged: Vec<_> = plan
        .entries
        .iter()
        .filter(|e| e.status != RegenStatus::Unchanged && e.status != RegenStatus::OrphanLeft)
        .collect();
    assert!(
        not_converged.is_empty(),
        "expected a fully converged no-op build: {not_converged:?}"
    );
}

#[test]
fn successive_builds_add_incremental_migrations_only() {
    let dir = std::env::temp_dir().join(format!(
        "ciac-migrations-test-{}-{}",
        std::process::id(),
        "incremental"
    ));
    std::fs::remove_dir_all(&dir).ok();

    let sql1 = migrate_step(&dir, SCHEMA_V1)
        .expect("first build has no prior schema to conflict with")
        .expect("a brand-new table produces a migration");
    assert!(
        sql1.contains("CREATE TABLE IF NOT EXISTS Videos"),
        "expected a CREATE TABLE for the new table: {sql1}"
    );
    let first_migration = dir.join("migrations/0001_migration.sql");
    let first_migration_content =
        std::fs::read_to_string(&first_migration).expect("first migration file exists");
    assert_eq!(first_migration_content, sql1);

    let sql2 = migrate_step(&dir, SCHEMA_V2)
        .expect("adding a column is additive")
        .expect("a new column produces a migration");
    assert!(
        sql2.contains("ALTER TABLE Videos ADD COLUMN summary"),
        "expected an ADD COLUMN for the new field: {sql2}"
    );
    assert!(
        !sql2.contains("CREATE TABLE"),
        "the second migration must not recreate the table: {sql2}"
    );
    // The first migration file is untouched: it's `Seeded`, and the
    // second build never re-emits path `0001_migration.sql`.
    assert_eq!(
        std::fs::read_to_string(&first_migration).expect("still exists"),
        first_migration_content,
        "the first migration file must survive byte-identical"
    );
    let second_migration = dir.join("migrations/0002_migration.sql");
    assert_eq!(
        std::fs::read_to_string(&second_migration).expect("second migration file exists"),
        sql2
    );

    let sql3 = migrate_step(&dir, SCHEMA_V2).expect("unchanged schema is not a conflict");
    assert!(
        sql3.is_none(),
        "an unchanged schema must not produce a third migration: {sql3:?}"
    );
    assert!(
        !dir.join("migrations/0003_migration.sql").exists(),
        "no third migration file should have been written"
    );
    assert_converged(&dir, SCHEMA_V2);

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn dropped_column_is_refused_between_builds() {
    let dir = std::env::temp_dir().join(format!(
        "ciac-migrations-test-{}-{}",
        std::process::id(),
        "dropped-column"
    ));
    std::fs::remove_dir_all(&dir).ok();

    migrate_step(&dir, SCHEMA_V2)
        .expect("first build succeeds")
        .expect("emits a migration");

    let err = migrate_step(&dir, SCHEMA_V3_DROPPED)
        .expect_err("dropping `title`/`summary` must be refused, not silently migrated");
    assert!(
        matches!(&err, SchemaChange::ColumnRemoved { table, column } if table == "Videos" && (column == "title" || column == "summary")),
        "expected a ColumnRemoved refusal: {err:?}"
    );
    assert!(
        err.to_string().contains("manual migration"),
        "expected the error to point at a manual migration: {err}"
    );

    std::fs::remove_dir_all(&dir).ok();
}
