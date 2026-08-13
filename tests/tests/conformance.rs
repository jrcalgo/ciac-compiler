//! The conformance harness (v0.22 M4 — `22UpdatePlan.md` Pillar 4):
//! parity as a test, not a claim. Plans 23-25 (TypeScript/Go/Java)
//! inherit this file as their definition of done.
//!
//! Assertion inventory (numbered so failures/checklists can cite them):
//!
//! - **C1** (generate succeeds) and **C2** (goldens match) already run
//!   in `tests/golden.rs::example_generated_project_snapshots` — every
//!   `.expect("examples generate")` there is C1; the `insta` snapshot
//!   assertion is C2. Not duplicated here.
//! - **C3** (cross-target OpenAPI equality): every registered target's
//!   `openapi.json` for the same program must be byte-identical.
//! - **C4** (topology equality): every subject/queue-group/cron-
//!   schedule/table-name the shared model declares must appear
//!   verbatim in every supporting target's generated output, and every
//!   migration SQL file must be byte-identical across targets by path.
//! - **C5** (validation): each generated project's own
//!   `TargetInfo::validate` steps run in CI's `generated-python`/
//!   `generated-rust` jobs (`.github/workflows/ci.yml`) — local
//!   toolchains, not re-run here (this suite stays fast); delegated,
//!   not skipped.
//! - **C6** (ratchet proofs): no content yet — a support-matrix-table
//!   discipline with nothing to check mechanically until a fourth
//!   target exists to diverge from. Named here so the numbering is
//!   stable for when it lands.
//! - **C7** (boundary decode/encode): live-verified rather than housed
//!   in this file's own harness — Go's absent/null/zero decode triple
//!   was proven at v0.24 M2 against a running binary (not a unit test:
//!   a missing field, an explicit `null`, and a legitimate zero value
//!   each got a real HTTP round-trip and the right status code); the
//!   nil-slice-normalization row got the same treatment at v0.24 M9
//!   (`examples/query-verbs.ciac`'s zero-row response), plus a
//!   structural regression test,
//!   `go_db_query_result_initializes_as_a_non_nil_empty_slice` in
//!   `typed_handler_equivalence.rs`. Neither Rust's nor TypeScript's
//!   own narrow-sim passes built a shared mechanical C7 harness either
//!   (each did its own live proof instead) — inventing one now, for a
//!   boundary class with exactly one target-specific case per target
//!   so far, would be scope beyond what any of the three arcs actually
//!   needed. Revisit if a fourth target's own boundary cases start
//!   repeating this file's other C-number shape.
//!
//! Post-`33UpdatePlan.md` follow-up: `c3`/`c4a`/`c4b` each independently
//! called `supported_projects` for every example -- three full,
//! identical generation passes over the whole corpus, the exact
//! redundancy `33UpdatePlan.md`'s own retrospective flagged as the
//! highest-value item left on the table (measured there: each fn alone
//! costs ~20s; libtest's 3-way concurrency claws back only a third of
//! the tripled work). Fixed by memoizing the generation into a
//! process-wide `LazyLock`, built once by whichever test fn reaches it
//! first (the other two block briefly, then read the same data) and
//! parallelized internally via `chunk_by_weight` (LPT scheduling on
//! source byte size). This changes *when* generation happens, not what
//! is asserted: each test fn's own logic, inputs, and assertion count
//! are unchanged, just reading from `MEMO` instead of calling
//! `supported_projects` (and `compile_file`, for C4b's `ir`) itself.
//! `supported_projects` itself -- and therefore what "supported" means
//! -- is untouched.
//!
//! `chunk_by_weight` was also tried in `determinism.rs` and reverted
//! there: it balances source bytes well but that stopped predicting
//! per-example java cost once M6's AppCDS archive made that cost
//! dominated by a near-constant per-call JVM fee rather than anything
//! proportional to source size (see that file's own note). Kept here
//! regardless -- this build phase was written from scratch, so using
//! it costs nothing extra, and it is at worst neutral rather than
//! measurably harmful for the same reason it was neutral there.

use ciac_codegen::model::build_system;
use ciac_codegen::GenOptions;
use ciac_integration_tests::{
    backends, chunk_by_weight, ciac_files, compile_file, examples_dir, file_weight, project_dump,
    worker_count,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::LazyLock;

/// Every registered target that accepts `ir`, generated once.
fn supported_projects(
    ir: &ciac_ir::NormalizedIr,
) -> Vec<(&'static str, ciac_codegen::GeneratedProject)> {
    backends()
        .into_iter()
        .filter(|b| ciac_codegen::check_support(b.as_ref(), ir).is_ok())
        .map(|b| {
            let project = b
                .generate(ir, &GenOptions::default())
                .expect("supported example generates");
            (b.id(), project)
        })
        .collect()
}

/// One example's memoized compile + generate result, shared read-only
/// across `c3`/`c4a`/`c4b`. `ir` is kept (not just `projects`) because
/// C4b needs `build_system(&ir, ..)`, which is cheap relative to
/// generation but still needless to redo three times.
struct MemoizedExample {
    name: String,
    ir: ciac_ir::NormalizedIr,
    projects: Vec<(&'static str, ciac_codegen::GeneratedProject)>,
}

/// Built once, by whichever test fn reaches it first; the other two
/// block on the same `LazyLock` and then read the completed result.
/// Internal build is parallel (`chunk_by_weight`/`worker_count`,
/// matching `determinism.rs`), then sorted back into `ciac_files`'
/// own order so iteration order -- and therefore which example a
/// failure names first -- is unchanged from before this fn existed.
static MEMO: LazyLock<Vec<MemoizedExample>> = LazyLock::new(|| {
    let weighted: Vec<((usize, PathBuf), u64)> = ciac_files(&examples_dir())
        .into_iter()
        .enumerate()
        .map(|(idx, path)| {
            let weight = file_weight(&path);
            ((idx, path), weight)
        })
        .collect();
    let chunks = chunk_by_weight(weighted, worker_count());

    let mut results: Vec<(usize, MemoizedExample)> = std::thread::scope(|scope| {
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || {
                    let mut local = Vec::new();
                    for (idx, path) in chunk {
                        let name = path
                            .file_stem()
                            .expect("file name")
                            .to_string_lossy()
                            .into_owned();
                        let ir = compile_file(&path);
                        let projects = supported_projects(&ir);
                        local.push((idx, MemoizedExample { name, ir, projects }));
                    }
                    local
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    });

    results.sort_by_key(|(idx, _)| *idx);
    results.into_iter().map(|(_, ex)| ex).collect()
});

/// C3: every supporting target's `openapi.json` file(s) — keyed by
/// path, so single- and multi-service systems both compare correctly
/// (a multi-service system has one per service dir plus a root
/// system-level index) — must be byte-identical across targets.
#[test]
fn c3_openapi_is_byte_identical_across_targets() {
    for ex in MEMO.iter() {
        let name = &ex.name;
        let projects = &ex.projects;
        if projects.len() < 2 {
            continue; // nothing to compare an example generated for only one target against
        }
        let openapi_files = |project: &ciac_codegen::GeneratedProject| -> BTreeMap<String, String> {
            project
                .files()
                .filter(|(p, _)| p.ends_with("openapi.json"))
                .map(|(p, c)| (p.to_owned(), c.to_owned()))
                .collect()
        };
        let (first_id, first_project) = &projects[0];
        let first_files = openapi_files(first_project);
        for (other_id, other_project) in &projects[1..] {
            let other_files = openapi_files(other_project);
            assert_eq!(
                first_files.keys().collect::<Vec<_>>(),
                other_files.keys().collect::<Vec<_>>(),
                "{name}: openapi.json path set differs between {first_id} and {other_id}"
            );
            for (openapi_path, first_content) in &first_files {
                let other_content = &other_files[openapi_path];
                assert_eq!(
                    first_content, other_content,
                    "{name}: {openapi_path} differs between {first_id} and {other_id}"
                );
            }
        }
    }
}

/// C4a: migration SQL is shared, engine-keyed code (`ciac-codegen::
/// migrations`), not language-keyed — every `*.sql` file under a
/// migrations directory must be byte-identical across targets by path
/// (the assertion the plan's own text names: it exists to catch a
/// backend accidentally post-processing a shared artifact, not to
/// duplicate the differ's own tests).
#[test]
fn c4a_migration_sql_is_byte_identical_across_targets() {
    for ex in MEMO.iter() {
        let name = &ex.name;
        let projects = &ex.projects;
        if projects.len() < 2 {
            continue;
        }
        let sql_files = |project: &ciac_codegen::GeneratedProject| -> BTreeMap<String, String> {
            project
                .files()
                .filter(|(p, _)| p.ends_with(".sql"))
                .map(|(p, c)| {
                    // Strip the target-specific migrations-dir prefix
                    // (`app/migrations/` vs `migrations/`) so the same
                    // logical migration compares by filename, not by
                    // the directory convention `TargetInfo` owns.
                    let filename = p.rsplit('/').next().unwrap_or(p);
                    (filename.to_owned(), c.to_owned())
                })
                .collect()
        };
        let (first_id, first_project) = &projects[0];
        let first_files = sql_files(first_project);
        for (other_id, other_project) in &projects[1..] {
            let other_files = sql_files(other_project);
            assert_eq!(
                first_files.keys().collect::<Vec<_>>(),
                other_files.keys().collect::<Vec<_>>(),
                "{name}: migration SQL filename set differs between {first_id} and {other_id}"
            );
            for (filename, first_content) in &first_files {
                let other_content = &other_files[filename];
                assert_eq!(
                    first_content, other_content,
                    "{name}: migration {filename} differs between {first_id} and {other_id}"
                );
            }
        }
    }
}

/// C4b: every subject/queue-group/cron-schedule/table-name the shared
/// model declares appears verbatim somewhere in every supporting
/// target's generated output — a target that silently drops or
/// renames a topology fact fails this, without needing a per-language
/// parser to prove it (the model is the neutral source of truth both
/// backends render from; this checks the render kept faith with it).
#[test]
fn c4b_declared_topology_appears_in_every_target() {
    for ex in MEMO.iter() {
        let name = &ex.name;
        let projects = &ex.projects;
        if projects.is_empty() {
            continue;
        }
        let model = build_system(&ex.ir, &GenOptions::default());
        let mut facts: Vec<String> = Vec::new();
        for ctx in &model.services {
            for worker in &ctx.workers {
                facts.push(worker.subject.clone());
                facts.push(worker.queue_group.clone());
            }
            for job in &ctx.jobs {
                facts.push(job.schedule.clone());
            }
            for channel in &ctx.channels {
                facts.push(channel.subject.clone());
            }
            for table in &ctx.tables {
                facts.push(table.class_name.clone());
            }
        }
        for (target_id, project) in projects {
            let dump = project_dump(project);
            for fact in &facts {
                assert!(
                    dump.contains(fact.as_str()),
                    "{name}: declared topology fact {fact:?} does not appear anywhere in the \
                     {target_id} target's generated output"
                );
            }
        }
    }
}
