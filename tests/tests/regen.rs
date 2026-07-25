use ciac_codegen::manifest::{build_manifest, load_manifest, write_manifest};
use ciac_codegen::regen::{
    apply_regeneration, plan_regeneration, ApplyMode, RegenMode, RegenStatus,
};
use ciac_codegen::GeneratedProject;
use std::path::{Path, PathBuf};

#[test]
fn clean_rebuild_is_noop() {
    let dir = temp_dir("clean-rebuild-is-noop");
    let old = project([("app/main.py", "print('v1')\n")], []);
    old.write_to(&dir).unwrap();
    let manifest = build_manifest(&old, "0.6.0", "1.0.0", "source", "python");
    write_manifest(&dir, &manifest).unwrap();

    let plan = plan_regeneration(&old, &dir, Some(&manifest), RegenMode::Normal).unwrap();
    assert_eq!(statuses(&plan), [RegenStatus::Unchanged]);
    apply_regeneration(&plan, &dir, ApplyMode::Full).unwrap();
    let roundtrip = load_manifest(&dir).unwrap();
    assert_eq!(roundtrip, manifest);
    cleanup(&dir);
}

#[test]
fn modified_owned_file_gets_conflict_sidecar() {
    let dir = temp_dir("modified-owned-file-gets-conflict-sidecar");
    let old = project([("app/main.py", "print('v1')\n")], []);
    old.write_to(&dir).unwrap();
    let manifest = build_manifest(&old, "0.6.0", "1.0.0", "source", "python");
    std::fs::write(dir.join("app/main.py"), "print('user')\n").unwrap();

    let new = project([("app/main.py", "print('v2')\n")], []);
    let plan = plan_regeneration(&new, &dir, Some(&manifest), RegenMode::Normal).unwrap();
    assert!(plan.has_errors());
    assert_eq!(statuses(&plan), [RegenStatus::Conflict]);
    apply_regeneration(&plan, &dir, ApplyMode::Full).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("app/main.py")).unwrap(),
        "print('user')\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("app/main.py.ciac-new")).unwrap(),
        "print('v2')\n"
    );
    cleanup(&dir);
}

#[test]
fn seeded_file_drift_gets_warning_sidecar() {
    let dir = temp_dir("seeded-file-drift-gets-warning-sidecar");
    let old = project(
        [],
        [(
            "app/services/store.py",
            "async def run(payload):\n    pass\n",
        )],
    );
    old.write_to(&dir).unwrap();
    let manifest = build_manifest(&old, "0.6.0", "1.0.0", "source", "python");
    std::fs::write(
        dir.join("app/services/store.py"),
        "async def run(payload):\n    return payload\n",
    )
    .unwrap();

    let new = project(
        [],
        [(
            "app/services/store.py",
            "async def run(payload, session):\n    pass\n",
        )],
    );
    let plan = plan_regeneration(&new, &dir, Some(&manifest), RegenMode::Normal).unwrap();
    assert!(!plan.has_errors());
    assert!(plan.has_warnings());
    assert_eq!(statuses(&plan), [RegenStatus::SeededDrift]);
    apply_regeneration(&plan, &dir, ApplyMode::Full).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("app/services/store.py.ciac-new")).unwrap(),
        "async def run(payload, session):\n    pass\n"
    );
    cleanup(&dir);
}

#[test]
fn untouched_owned_orphan_is_deleted() {
    let dir = temp_dir("untouched-owned-orphan-is-deleted");
    let old = project([("app/old.py", "old\n")], []);
    old.write_to(&dir).unwrap();
    let manifest = build_manifest(&old, "0.6.0", "1.0.0", "source", "python");

    let new = project([], []);
    let plan = plan_regeneration(&new, &dir, Some(&manifest), RegenMode::Normal).unwrap();
    assert_eq!(statuses(&plan), [RegenStatus::OrphanDelete]);
    apply_regeneration(&plan, &dir, ApplyMode::Full).unwrap();
    assert!(!dir.join("app/old.py").exists());
    cleanup(&dir);
}

#[test]
fn adopt_preserves_existing_files_and_writes_sidecars() {
    let dir = temp_dir("adopt-preserves-existing-files-and-writes-sidecars");
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::write(dir.join("app/main.py"), "print('user')\n").unwrap();

    let new = project(
        [
            ("app/main.py", "print('generated')\n"),
            ("app/new.py", "new\n"),
        ],
        [],
    );
    let plan = plan_regeneration(&new, &dir, None, RegenMode::Adopt).unwrap();
    assert_eq!(statuses(&plan), [RegenStatus::Conflict, RegenStatus::New]);
    apply_regeneration(&plan, &dir, ApplyMode::Full).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("app/main.py")).unwrap(),
        "print('user')\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("app/main.py.ciac-new")).unwrap(),
        "print('generated')\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("app/new.py")).unwrap(),
        "new\n"
    );
    cleanup(&dir);
}

fn project<const O: usize, const S: usize>(
    owned: [(&str, &str); O],
    seeded: [(&str, &str); S],
) -> GeneratedProject {
    let mut project = GeneratedProject::new();
    for (path, content) in owned {
        project.add_file(path, content);
    }
    for (path, content) in seeded {
        project.add_seeded_file(path, content);
    }
    project
}

fn statuses(plan: &ciac_codegen::regen::RegenPlan) -> Vec<RegenStatus> {
    plan.entries.iter().map(|entry| entry.status).collect()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ciac-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(path: &Path) {
    std::fs::remove_dir_all(path).ok();
}
