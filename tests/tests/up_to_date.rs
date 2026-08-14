//! `32UpdatePlan.md` M3: CLI coverage for `ciac build`'s up-to-date
//! early-out. Unlike `tests/tests/regen.rs` (which exercises
//! `ciac-codegen::regen` directly as a library), the early-out lives in
//! `commands::is_up_to_date`, private to the `ciac` binary crate --
//! there is no library target to link against, so these tests shell
//! out to the real compiled binary, matching `tests/src/bench.rs`'s
//! own `resolve_ciac_release_binary` convention. Every test asserts on
//! the presence or absence of the `"up to date"` line `build_inner`
//! prints on the skip path, which is the only externally observable
//! signal of whether the early-out fired.

use ciac_integration_tests::bench::resolve_ciac_release_binary;
use std::path::{Path, PathBuf};
use std::process::Command;

fn ping_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/ping.ciac")
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ciac-up-to-date-{name}-{}", std::process::id()));
    std::fs::remove_dir_all(&dir).ok();
    dir
}

fn cleanup(path: &Path) {
    std::fs::remove_dir_all(path).ok();
}

/// Runs `ciac build <ping> --target python --out <out> <extra_args...>`
/// against the release binary and returns whether it succeeded plus
/// stdout+stderr combined (the skip message goes to stderr, same
/// stream as every other status line `build_inner` prints).
fn build(out: &Path, extra_args: &[&str]) -> (bool, String) {
    let binary = resolve_ciac_release_binary();
    let ping = ping_path();
    let output = Command::new(binary)
        .arg("build")
        .arg(&ping)
        .arg("--target")
        .arg("python")
        .arg("--out")
        .arg(out)
        .args(extra_args)
        .output()
        .expect("spawning `ciac build`");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), combined)
}

#[test]
fn skip_fires_on_an_untouched_tree() {
    let out = temp_dir("skip-fires-on-an-untouched-tree");

    let (ok, cold) = build(&out, &[]);
    assert!(ok, "cold build failed: {cold}");
    assert!(!cold.contains("up to date"), "cold build: {cold}");

    let (ok, warm) = build(&out, &[]);
    assert!(ok, "warm rebuild failed: {warm}");
    assert!(
        warm.contains("up to date"),
        "untouched warm rebuild should skip: {warm}"
    );

    cleanup(&out);
}

#[test]
fn force_regenerates_anyway() {
    let out = temp_dir("force-regenerates-anyway");

    build(&out, &[]);
    let (ok, warm) = build(&out, &[]);
    assert!(ok && warm.contains("up to date"), "setup: {warm}");

    let (ok, forced) = build(&out, &["--force"]);
    assert!(ok, "--force build failed: {forced}");
    assert!(
        !forced.contains("up to date"),
        "--force must bypass the skip entirely: {forced}"
    );

    cleanup(&out);
}

#[test]
fn editing_a_generated_file_on_disk_defeats_the_skip() {
    let out = temp_dir("editing-a-generated-file-on-disk-defeats-the-skip");

    build(&out, &[]);
    let owned_file = out.join("app/main.py");
    let original = std::fs::read_to_string(&owned_file).expect("owned file exists after build");
    std::fs::write(&owned_file, format!("{original}# tampered\n")).unwrap();

    let (_, rebuilt) = build(&out, &[]);
    assert!(
        !rebuilt.contains("up to date"),
        "a hand-edited owned file must defeat the skip: {rebuilt}"
    );

    cleanup(&out);
}

#[test]
fn deleting_a_manifest_tracked_file_defeats_the_skip() {
    let out = temp_dir("deleting-a-manifest-tracked-file-defeats-the-skip");

    build(&out, &[]);
    let tracked_file = out.join("app/config.py");
    assert!(
        tracked_file.exists(),
        "fixture assumption: app/config.py is generated"
    );
    std::fs::remove_file(&tracked_file).unwrap();

    let (ok, rebuilt) = build(&out, &[]);
    assert!(ok, "rebuild after a deleted file should succeed: {rebuilt}");
    assert!(
        !rebuilt.contains("up to date"),
        "a deleted manifest-tracked file must defeat the skip: {rebuilt}"
    );
    assert!(
        tracked_file.exists(),
        "the deleted file must be regenerated, not left missing"
    );

    cleanup(&out);
}

#[test]
fn a_manifest_with_recipe_none_always_falls_through() {
    let out = temp_dir("a-manifest-with-recipe-none-always-falls-through");

    build(&out, &[]);
    let manifest_path = out.join(".ciac/manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    manifest["recipe"] = serde_json::Value::Null;
    std::fs::write(
        &manifest_path,
        serde_json::to_string_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let (ok, rebuilt) = build(&out, &[]);
    assert!(
        ok,
        "rebuild over a legacy (recipe: null) manifest should succeed: {rebuilt}"
    );
    assert!(
        !rebuilt.contains("up to date"),
        "a manifest with recipe: None must always fall through: {rebuilt}"
    );

    // The fallback path writes a fresh manifest with a real recipe, so
    // the *next* rebuild is a normal untouched-tree skip again.
    let (ok, again) = build(&out, &[]);
    assert!(
        ok && again.contains("up to date"),
        "post-fallback rebuild: {again}"
    );

    cleanup(&out);
}

#[test]
fn changing_deploy_between_builds_defeats_the_skip() {
    let out = temp_dir("changing-deploy-between-builds-defeats-the-skip");

    build(&out, &[]);
    let (ok, warm) = build(&out, &[]);
    assert!(ok && warm.contains("up to date"), "setup: {warm}");

    let (ok, changed) = build(&out, &["--deploy", "k8s"]);
    assert!(ok, "--deploy k8s build failed: {changed}");
    assert!(
        !changed.contains("up to date"),
        "adding --deploy k8s must defeat the skip: {changed}"
    );

    let (ok, stable) = build(&out, &["--deploy", "k8s"]);
    assert!(
        ok && stable.contains("up to date"),
        "repeating the same --deploy k8s should skip again: {stable}"
    );

    cleanup(&out);
}

#[test]
fn changing_client_between_builds_defeats_the_skip() {
    let out = temp_dir("changing-client-between-builds-defeats-the-skip");

    build(&out, &[]);
    let (ok, warm) = build(&out, &[]);
    assert!(ok && warm.contains("up to date"), "setup: {warm}");

    let (ok, changed) = build(&out, &["--client", "ts"]);
    assert!(ok, "--client ts build failed: {changed}");
    assert!(
        !changed.contains("up to date"),
        "adding --client ts must defeat the skip: {changed}"
    );

    let (ok, stable) = build(&out, &["--client", "ts"]);
    assert!(
        ok && stable.contains("up to date"),
        "repeating the same --client ts should skip again: {stable}"
    );

    cleanup(&out);
}
