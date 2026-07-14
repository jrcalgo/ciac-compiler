//! v0.18 M6: `ciac backfill plan`'s CLI surface, exercised through the
//! real binary — no-eligible-changes, refusal before the expand
//! migration lands, script generation once it has, contract-migration
//! withholding vs `--allow-destructive`, and duplicate-write refusal.
//! Mirrors `baseline_cli.rs`'s spawn pattern.

use std::process::Command;

fn ciac() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ciac"))
}

const SRC_V1: &str = r#"
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

const SRC_V2: &str = r#"
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

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ciac-backfill-cli-test-{}-{name}",
            std::process::id()
        ));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("create temp dir");
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

fn plan_id_from(stderr: &str) -> String {
    stderr
        .lines()
        .find_map(|l| l.strip_prefix("plan "))
        .and_then(|rest| rest.split(':').next())
        .expect("a plan line")
        .to_owned()
}

#[test]
fn no_eligible_changes_is_a_clean_no_op() {
    let tmp = TempDir::new("no-changes");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("out");
    std::fs::write(&entry, SRC_V1).expect("write entry");

    let created = ciac()
        .args(["baseline", entry.to_str().unwrap()])
        .output()
        .expect("ciac runs");
    assert!(created.status.success(), "{created:?}");

    let build = ciac()
        .args([
            "build",
            entry.to_str().unwrap(),
            "-t",
            "python",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(build.status.success(), "{build:?}");

    let result = ciac()
        .args([
            "backfill",
            "plan",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(result.status.success(), "{result:?}");
    assert!(String::from_utf8(result.stderr)
        .unwrap()
        .contains("no backfill-eligible changes"));
}

#[test]
fn refuses_until_the_expand_migration_lands_then_plans_and_gates_the_contract() {
    let tmp = TempDir::new("full-ladder");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("out");
    std::fs::write(&entry, SRC_V1).expect("write entry");

    ciac()
        .args(["baseline", entry.to_str().unwrap()])
        .output()
        .expect("ciac runs");
    ciac()
        .args([
            "build",
            entry.to_str().unwrap(),
            "-t",
            "python",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");

    std::fs::write(&entry, SRC_V2).expect("write v2 entry");

    // Before rebuilding `out`: refused, since the expand migration
    // hasn't landed there yet.
    let too_early = ciac()
        .args([
            "backfill",
            "plan",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(!too_early.status.success());
    assert!(String::from_utf8(too_early.stderr)
        .unwrap()
        .contains("hasn't been expanded"));

    // An ordinary rebuild applies the expand migration.
    let rebuild = ciac()
        .args([
            "build",
            entry.to_str().unwrap(),
            "-t",
            "python",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(rebuild.status.success(), "{rebuild:?}");

    let planned = ciac()
        .args([
            "backfill",
            "plan",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(planned.status.success(), "{planned:?}");
    let stderr = String::from_utf8(planned.stderr).unwrap();
    assert!(stderr.contains("backfill script"));
    assert!(stderr.contains("withheld"));
    let plan_id = plan_id_from(&stderr);

    let script_path = out.join(format!("app/migrations/backfill_{plan_id}.py"));
    assert!(script_path.exists(), "{}", script_path.display());
    let plan_record = out.join(format!(".ciac/backfills/{plan_id}.json"));
    assert!(plan_record.exists());

    // No contract migration file exists yet.
    assert!(std::fs::read_dir(out.join("app/migrations"))
        .unwrap()
        .filter_map(|e| e.ok())
        .all(|e| !e.file_name().to_string_lossy().contains("contract")));

    // --allow-destructive materializes it.
    let allowed = ciac()
        .args([
            "backfill",
            "plan",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--allow-destructive",
            &plan_id,
        ])
        .output()
        .expect("ciac runs");
    assert!(allowed.status.success(), "{allowed:?}");
    let contract_files: Vec<_> = std::fs::read_dir(out.join("app/migrations"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("contract"))
        .collect();
    assert_eq!(contract_files.len(), 1);
    let contract_sql = std::fs::read_to_string(contract_files[0].path()).unwrap();
    assert!(contract_sql.contains(&plan_id));
    assert!(contract_sql.contains("_ciac_backfills"));
    assert!(contract_sql.contains("SET NOT NULL"));

    // A second --allow-destructive for the same plan is refused rather
    // than writing a duplicate contract migration.
    let duplicate = ciac()
        .args([
            "backfill",
            "plan",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--allow-destructive",
            &plan_id,
        ])
        .output()
        .expect("ciac runs");
    assert!(!duplicate.status.success());
    let contract_files_after: Vec<_> = std::fs::read_dir(out.join("app/migrations"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains("contract"))
        .collect();
    assert_eq!(
        contract_files_after.len(),
        1,
        "must not write a second contract migration for the same plan"
    );

    // `ciac verify` still passes with the backfill/contract files present.
    let verify = ciac()
        .args([
            "verify",
            entry.to_str().unwrap(),
            "-t",
            "python",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(verify.status.success(), "{verify:?}");
}

#[test]
fn legacy_manifest_is_refused() {
    let tmp = TempDir::new("legacy-manifest");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("out");
    std::fs::write(&entry, SRC_V1).expect("write entry");
    ciac()
        .args(["baseline", entry.to_str().unwrap()])
        .output()
        .expect("ciac runs");
    ciac()
        .args([
            "build",
            entry.to_str().unwrap(),
            "-t",
            "python",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");

    let manifest_path = out.join(".ciac/manifest.json");
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    doc.as_object_mut().unwrap().remove("recipe");
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();

    std::fs::write(&entry, SRC_V2).expect("write v2 entry");
    let result = ciac()
        .args([
            "backfill",
            "plan",
            entry.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(!result.status.success());
    assert!(String::from_utf8(result.stderr)
        .unwrap()
        .contains("legacy manifest"));
}
