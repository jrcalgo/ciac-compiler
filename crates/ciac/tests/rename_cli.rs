//! v0.18 M4/M5: `ciac rename`'s CLI surface, exercised through the real
//! binary — qualified/position resolution, dry-run vs `--apply`,
//! rejection cases, and (M5) `--out` regeneration replay: a clean
//! regenerate-and-commit, a legacy-manifest refusal that rolls the
//! source back untouched, and an unsafe-regeneration refusal that does
//! the same. Mirrors `baseline_cli.rs`'s spawn pattern.

use std::path::Path;
use std::process::Command;

fn ciac() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ciac"))
}

const SRC: &str = r#"
service Ping;
record Video { id: Uuid; title: String; }
api Echo: Video { method: POST; path: "/echo"; }
handler EchoHandler(v: Video) -> Video {
    return v { title: v.title };
}
pipeline Echo: EchoHandler -> Return;
"#;

struct TempDir(std::path::PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "ciac-rename-cli-test-{}-{name}",
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

#[test]
fn dry_run_qualified_and_position_forms_agree() {
    let tmp = TempDir::new("dry-run-agree");
    let entry = tmp.0.join("main.ciac");
    std::fs::write(&entry, SRC).expect("write entry");

    let qualified = ciac()
        .args(["rename", entry.to_str().unwrap(), "Video", "Clip"])
        .output()
        .expect("ciac runs");
    assert!(qualified.status.success(), "{qualified:?}");
    let qualified_out = String::from_utf8(qualified.stderr).unwrap();
    assert!(qualified_out.contains("rename record `Video` -> `Clip`"));
    assert!(qualified_out.contains("dry run"));

    let position = ciac()
        .args([
            "rename",
            entry.to_str().unwrap(),
            "--file",
            entry.to_str().unwrap(),
            "--line",
            "3",
            "--column",
            "8",
            "--to",
            "Clip",
        ])
        .output()
        .expect("ciac runs");
    assert!(position.status.success(), "{position:?}");
    let position_out = String::from_utf8(position.stderr).unwrap();
    assert!(position_out.contains("rename record `Video` -> `Clip`"));

    // A dry run never touches the source.
    assert_eq!(std::fs::read_to_string(&entry).unwrap(), SRC);
}

#[test]
fn apply_writes_source_and_recompiles_clean() {
    let tmp = TempDir::new("apply-clean");
    let entry = tmp.0.join("main.ciac");
    std::fs::write(&entry, SRC).expect("write entry");

    let result = ciac()
        .args([
            "rename",
            entry.to_str().unwrap(),
            "Video",
            "Clip",
            "--apply",
        ])
        .output()
        .expect("ciac runs");
    assert!(result.status.success(), "{result:?}");
    let edited = std::fs::read_to_string(&entry).unwrap();
    assert!(edited.contains("record Clip"));
    assert!(!edited.contains("Video"));

    let check = ciac()
        .args(["check", entry.to_str().unwrap()])
        .output()
        .expect("ciac runs");
    assert!(check.status.success(), "{check:?}");

    // No journal or backup files survive a clean commit.
    assert!(!tmp.0.join(".ciac").join("rename-journal.json").exists());
    assert!(std::fs::read_dir(&tmp.0)
        .unwrap()
        .filter_map(|e| e.ok())
        .all(|e| !e.file_name().to_string_lossy().contains("ciac-rename")));
}

#[test]
fn rejects_reserved_word_collision_and_unknown_symbol() {
    let tmp = TempDir::new("rejections");
    let entry = tmp.0.join("main.ciac");
    std::fs::write(&entry, SRC).expect("write entry");

    let reserved = ciac()
        .args(["rename", entry.to_str().unwrap(), "Video", "record"])
        .output()
        .expect("ciac runs");
    assert!(!reserved.status.success());

    let unknown = ciac()
        .args(["rename", entry.to_str().unwrap(), "Nope", "Whatever"])
        .output()
        .expect("ciac runs");
    assert!(!unknown.status.success());

    let both_forms = ciac()
        .args([
            "rename",
            entry.to_str().unwrap(),
            "--file",
            entry.to_str().unwrap(),
            "--line",
            "3",
            "--column",
            "8",
            "--to",
            "X",
            "Video",
            "Y",
        ])
        .output()
        .expect("ciac runs");
    assert!(
        !both_forms.status.success(),
        "mixing position and qualified forms must be refused"
    );

    // Nothing here should have touched the source.
    assert_eq!(std::fs::read_to_string(&entry).unwrap(), SRC);
}

fn set_manifest_field(out: &Path, mutate: impl FnOnce(&mut serde_json::Value)) {
    let manifest_path = out.join(".ciac").join("manifest.json");
    let mut doc: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    mutate(&mut doc);
    std::fs::write(&manifest_path, serde_json::to_vec_pretty(&doc).unwrap()).unwrap();
}

#[test]
fn out_replays_the_recorded_recipe_and_regenerates() {
    let tmp = TempDir::new("out-replay");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("out");
    std::fs::write(&entry, SRC).expect("write entry");

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
            "rename",
            entry.to_str().unwrap(),
            "Video",
            "Clip",
            "--apply",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(result.status.success(), "{result:?}");

    let schemas = std::fs::read_to_string(out.join("app/schemas.py")).unwrap();
    assert!(schemas.contains("class Clip"));
    assert!(!schemas.contains("class Video"));

    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(out.join(".ciac/manifest.json")).unwrap()).unwrap();
    assert!(
        manifest["recipe"].is_object(),
        "recipe must survive the replay"
    );
}

const TABLE_SRC: &str = r#"
service Ping;
record Video { id: Uuid; title: String; }
table Videos: Video;
"#;

/// v0.23 M8: the same `--out` replay, against the TypeScript backend
/// and a `table`-bearing program — proves `backfill::migrations_dir`
/// resolves through `TsBackend::target_info().migrations_dir`
/// ("migrations", matching Rust's value but *not* Python's own
/// "app/migrations") rather than falling back to some hardcoded
/// path, and that the replayed regeneration doesn't move or lose the
/// migration file once a new target's directory convention is in
/// play. Today this is an identity check (TS's value happens to
/// equal Rust's), but it's the same seam a future Java backend's own
/// (likely different) convention would need to pass through
/// correctly, so it's tested now rather than assumed.
#[test]
fn out_replay_resolves_the_typescript_target_migrations_dir() {
    let tmp = TempDir::new("out-replay-ts-migrations");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("out");
    std::fs::write(&entry, TABLE_SRC).expect("write entry");

    let build = ciac()
        .args([
            "build",
            entry.to_str().unwrap(),
            "-t",
            "typescript",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(build.status.success(), "{build:?}");
    assert!(
        out.join("migrations").join("0001_migration.sql").is_file(),
        "TS target's migrations_dir must resolve to migrations/, not app/migrations/ (Python's value)"
    );

    let result = ciac()
        .args([
            "rename",
            entry.to_str().unwrap(),
            "Video",
            "Clip",
            "--apply",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(result.status.success(), "{result:?}");

    let schemas = std::fs::read_to_string(out.join("src/schemas.ts")).unwrap();
    assert!(schemas.contains("Clip"));
    assert!(!schemas.contains("Video"));
    assert!(
        out.join("migrations").join("0001_migration.sql").is_file(),
        "the replayed regeneration must not lose the migration at its resolved path"
    );
}

/// v0.24 M8: the same `--out` replay, against the Go backend —
/// proves `backfill::migrations_dir` resolves through
/// `GoBackend::target_info().migrations_dir` ("migrations", matching
/// Rust's/TS's own value) and that the replayed regeneration survives
/// a fourth target's own directory convention.
#[test]
fn out_replay_resolves_the_go_target_migrations_dir() {
    let tmp = TempDir::new("out-replay-go-migrations");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("out");
    std::fs::write(&entry, TABLE_SRC).expect("write entry");

    let build = ciac()
        .args([
            "build",
            entry.to_str().unwrap(),
            "-t",
            "go",
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(build.status.success(), "{build:?}");
    assert!(
        out.join("migrations").join("0001_migration.sql").is_file(),
        "Go target's migrations_dir must resolve to migrations/, not app/migrations/ (Python's value)"
    );

    let result = ciac()
        .args([
            "rename",
            entry.to_str().unwrap(),
            "Video",
            "Clip",
            "--apply",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(result.status.success(), "{result:?}");

    let schemas = std::fs::read_to_string(out.join("internal/schemas/schemas.go")).unwrap();
    assert!(schemas.contains("Clip"));
    assert!(!schemas.contains("Video"));
    assert!(
        out.join("migrations").join("0001_migration.sql").is_file(),
        "the replayed regeneration must not lose the migration at its resolved path"
    );
}

#[test]
fn out_with_a_legacy_manifest_refuses_and_leaves_source_untouched() {
    let tmp = TempDir::new("out-legacy");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("out");
    std::fs::write(&entry, SRC).expect("write entry");

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

    set_manifest_field(&out, |doc| {
        doc.as_object_mut().unwrap().remove("recipe");
    });

    let result = ciac()
        .args([
            "rename",
            entry.to_str().unwrap(),
            "Video",
            "Clip",
            "--apply",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(
        !result.status.success(),
        "a legacy manifest (no recipe) must be refused"
    );
    assert_eq!(
        std::fs::read_to_string(&entry).unwrap(),
        SRC,
        "the source edit must roll back when --out can't proceed"
    );
    assert!(!tmp.0.join(".ciac").join("rename-journal.json").exists());
}

#[test]
fn out_with_an_unsafe_regeneration_refuses_and_leaves_source_untouched() {
    let tmp = TempDir::new("out-unsafe");
    let entry = tmp.0.join("main.ciac");
    let out = tmp.0.join("out");
    std::fs::write(&entry, SRC).expect("write entry");

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

    // Hand-edit a compiler-owned generated file so the replayed
    // regeneration plan reports a real conflict.
    let schemas_path = out.join("app/schemas.py");
    let mut schemas = std::fs::read_to_string(&schemas_path).unwrap();
    schemas.push_str("\n# hand-edited, must not be silently overwritten\n");
    std::fs::write(&schemas_path, schemas).unwrap();

    let result = ciac()
        .args([
            "rename",
            entry.to_str().unwrap(),
            "Video",
            "Clip",
            "--apply",
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("ciac runs");
    assert!(
        !result.status.success(),
        "an owned-file conflict in --out must refuse the whole rename"
    );
    assert_eq!(
        std::fs::read_to_string(&entry).unwrap(),
        SRC,
        "the source edit must roll back when --out can't regenerate safely"
    );
}
