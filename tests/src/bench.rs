//! `31UpdatePlan.md` M2: the phase-level generation-cost instrument.
//!
//! Promoted from a throwaway tool used to gather M1's own numbers
//! (`docs/perf/noise-floor.md`) — the same measurement shape, checked
//! in so it's reusable rather than living in a scratch directory.
//! `tests/bin/ciac-bench.rs` is the CLI that drives this module;
//! [`measure_example`] is the entry point everything else in this
//! file exists to support.
//!
//! Every phase `31UpdatePlan.md`'s own Pillar 3 table names is
//! measured here: front-end parse and analyse, the LSP overlay path,
//! the context model, the semantic model and its hash, generation per
//! registered backend (steady-state and the one-time template-setup
//! cost), the regeneration plan in both its cold and warm shapes, the
//! manifest, and the write. This module does not gate anything and
//! does not read or write a committed baseline — that's `31UpdatePlan.md`
//! M4's own job, built on top of the measurement primitives here
//! rather than duplicating them.

use ciac_codegen::manifest::build_manifest;
use ciac_codegen::regen::{plan_regeneration, RegenMode};
use ciac_codegen::semantic_model::SemanticModel;
use ciac_codegen::{Backend, GenOptions};
use ciac_diagnostics::{Diagnostics, SourceMap};
use ciac_ir::NormalizedIr;
use ciac_syntax::ast::Program;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Mean/stddev/min/max/p95 over a set of microsecond samples — the
/// same five numbers `docs/perf/noise-floor.md` reports by hand,
/// computed once here so every caller agrees on the definition
/// (in particular, `p95` as `ceil(0.95 * n) - 1`, clamped to the last
/// index, matching the noise-floor study's own convention).
#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub mean_us: f64,
    pub stddev_us: f64,
    pub min_us: f64,
    pub max_us: f64,
    pub p95_us: f64,
}

impl Stats {
    fn from_samples_us(mut samples: Vec<f64>) -> Stats {
        samples.sort_by(|a, b| a.partial_cmp(b).expect("timings are never NaN"));
        let n = samples.len();
        let mean = samples.iter().sum::<f64>() / n as f64;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n as f64;
        let p95_idx = ((n as f64) * 0.95).ceil() as usize;
        Stats {
            mean_us: mean,
            stddev_us: var.sqrt(),
            min_us: samples[0],
            max_us: samples[n - 1],
            p95_us: samples[p95_idx.saturating_sub(1).min(n - 1)],
        }
    }

    /// Relative standard deviation as a percentage — the number
    /// `docs/perf/noise-floor.md`'s own Pillar-1 rule
    /// (`max(3*rel_stddev, 10%)`) is built from.
    pub fn rel_stddev_pct(&self) -> f64 {
        if self.mean_us > 0.0 {
            self.stddev_us / self.mean_us * 100.0
        } else {
            0.0
        }
    }
}

/// Times `f` `reps` times after two discarded warm-up calls, returning
/// the resulting [`Stats`]. The two warm-up calls exist so a callee's
/// own one-time setup (allocator warm-up, filesystem cache) doesn't
/// bias the first timed sample the way it would a true first call —
/// see [`measure_template_setup`] for the one measurement in this
/// module that deliberately wants the *un*-warmed-up cost instead.
pub fn measure<T>(reps: u32, mut f: impl FnMut() -> T) -> Stats {
    let _ = f();
    let _ = f();
    let mut samples = Vec::with_capacity(reps as usize);
    for _ in 0..reps {
        let t0 = Instant::now();
        let _ = f();
        samples.push(t0.elapsed().as_secs_f64() * 1e6);
    }
    Stats::from_samples_us(samples)
}

/// One named phase's timing, e.g. `("sema::analyze", <stats>)`.
#[derive(Debug, Clone, Serialize)]
pub struct PhaseMeasurement {
    pub metric: String,
    pub stats: Stats,
}

/// One backend's generation cost and output shape for one example.
#[derive(Debug, Clone, Serialize)]
pub struct BackendGeneration {
    pub backend: String,
    pub steady_state: Stats,
    pub file_count: usize,
    pub total_bytes: usize,
    pub largest_file_path: String,
    pub largest_file_bytes: usize,
}

/// One backend's process-wide, measured-once template-environment
/// setup cost (`31UpdatePlan.md` Pillar 3: "first-call minus
/// steady-state `generate()`" — the cost the M4-era `OnceLock`
/// template cache structurally cannot amortize inside a single CLI
/// invocation, since a `ciac build` process calls `generate()` exactly
/// once). Reported once per `ciac-bench` process, not once per
/// example: every backend's own `static OnceLock<Environment>` is a
/// process-wide static (see e.g. `ciac-backend-python/src/lib.rs`), so
/// only the very first `generate()` call for a given backend anywhere
/// in the process observes the cold-template cost — every call after
/// that, on any example, is already warm. Measuring this per example
/// would silently report zero setup cost for every example after the
/// first.
#[derive(Debug, Clone, Serialize)]
pub struct TemplateSetup {
    pub backend: String,
    pub first_call_us: f64,
    pub steady_state_us: f64,
    pub setup_us: f64,
}

/// The full phase report for one `.ciac` example.
#[derive(Debug, Clone, Serialize)]
pub struct ExampleReport {
    pub example: String,
    pub phases: Vec<PhaseMeasurement>,
    pub backends: Vec<BackendGeneration>,
}

/// The repetition count actually used for one backend's `generate()`
/// timing, derived from the caller's requested `reps`. Python, Rust
/// and TypeScript render straight from in-process templates and cost
/// microseconds, so they run at the full requested count. Go and Java
/// both shell out to a real external formatter per `generate()` call
/// (`gofmt`, and a JVM invocation for `google-java-format`) — costing
/// tens of milliseconds and several hundred milliseconds respectively,
/// confirmed live in this arc's own M1 sweep and in `30UpdatePlan.md`'s
/// own baseline — so both are scaled down, mirroring the exact
/// reduction M1's own throwaway measurement tool used for Go
/// (`(reps/4).max(5)`) and extending the same reasoning to Java, which
/// M1's own phase table never included at full cost for the same
/// reason. This keeps `ciac-bench`'s default run in the tens of
/// seconds rather than minutes without changing what full-cost
/// backends report, and without silently dropping Java from coverage
/// the way M1's own phase table did (M1 measured Java's generation
/// cost separately, through the existing `perf_budget.rs`/
/// `bench-codegen.sh` instruments, not through this reduction).
fn effective_reps(backend_id: &str, reps: u32) -> u32 {
    match backend_id {
        "go" => (reps / 4).max(5),
        "java" => (reps / 8).max(3),
        _ => reps,
    }
}

/// Measures every backend's one-time template-setup cost, in process
/// order, before anything else touches a backend's `generate()`. Must
/// run first and exactly once per `ciac-bench` invocation — calling it
/// twice, or calling any backend's `generate()` beforehand, would
/// observe an already-warmed static and silently report near-zero
/// setup cost. `reps` controls only the steady-state measurement that
/// follows each backend's single uninstrumented first call, scaled per
/// backend by [`effective_reps`].
pub fn measure_template_setup(
    ir: &NormalizedIr,
    opts: &GenOptions,
    backends: &[Box<dyn Backend>],
    reps: u32,
) -> Vec<TemplateSetup> {
    backends
        .iter()
        .filter(|b| ciac_codegen::check_support(b.as_ref(), ir).is_ok())
        .map(|b| {
            let t0 = Instant::now();
            let _ = b.generate(ir, opts).expect("generates");
            let first_call_us = t0.elapsed().as_secs_f64() * 1e6;
            let steady = measure(effective_reps(b.id(), reps), || {
                b.generate(ir, opts).expect("generates")
            });
            TemplateSetup {
                backend: b.id().to_owned(),
                first_call_us,
                steady_state_us: steady.mean_us,
                setup_us: (first_call_us - steady.mean_us).max(0.0),
            }
        })
        .collect()
}

/// Loads and analyzes `path` once, for use as the fixture every
/// per-example phase measurement below re-derives from. Panics on an
/// invalid program — `31UpdatePlan.md`'s own examples corpus is
/// known-good, matching `ciac_integration_tests::compile_file`'s own
/// convention.
fn load(path: &Path) -> (Program, NormalizedIr) {
    let mut sources = SourceMap::new();
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::load(path, &mut sources, &mut diags)
        .unwrap_or_else(|err| panic!("cannot read {}: {err}", path.display()));
    let ir = ciac_sema::analyze(&program, &mut diags)
        .unwrap_or_else(|| panic!("{} failed to compile: {:?}", path.display(), diags.codes()));
    (program, ir)
}

/// Runs the full phase sweep for one example against already-warmed
/// backends (see [`measure_template_setup`] for why warm-up must
/// happen first, once, outside this function). Every phase
/// `31UpdatePlan.md` Pillar 3 names is measured except LSP-overlay
/// diagnostics harvesting/formatting: [`ciac_syntax::module::load_with_overlay`]
/// plus [`ciac_sema::analyze`] together are measured (the dominant
/// cost, and the part the known `sema::analyze` quadratic actually
/// hits), but `ciac`'s own `harvest`/`to_lsp_diagnostics` are
/// `pub(crate)` to the `ciac` binary and are not reachable from here —
/// disclosed rather than silently omitted, matching Pillar 3's own
/// requirement.
pub fn measure_example(path: &Path, reps: u32, backends: &[Box<dyn Backend>]) -> ExampleReport {
    let example = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_owned();

    let (_, ir) = load(path);
    let opts = GenOptions::default();
    let mut phases = Vec::new();

    phases.push(PhaseMeasurement {
        metric: "syntax::load".to_owned(),
        stats: measure(reps, || {
            let mut sources = SourceMap::new();
            let mut diags = Diagnostics::new();
            ciac_syntax::load(path, &mut sources, &mut diags).expect("known-good example")
        }),
    });

    let program = {
        let mut sources = SourceMap::new();
        let mut diags = Diagnostics::new();
        ciac_syntax::load(path, &mut sources, &mut diags).expect("known-good example")
    };
    phases.push(PhaseMeasurement {
        metric: "sema::analyze".to_owned(),
        stats: measure(reps, || {
            let mut diags = Diagnostics::new();
            ciac_sema::analyze(&program, &mut diags).expect("known-good example")
        }),
    });

    // LSP overlay path: same front end, sourced from an in-memory
    // buffer instead of disk, as `ciac lsp`'s own debounced revalidate
    // does. The buffer is just the file's own bytes -- there is no
    // real "unsaved edit" here, only the code path a real edit takes.
    let overlay_text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    phases.push(PhaseMeasurement {
        metric: "lsp::load_with_overlay+analyze".to_owned(),
        stats: measure(reps, || {
            let mut sources = SourceMap::new();
            let mut diags = Diagnostics::new();
            let program = ciac_syntax::module::load_with_overlay(
                path,
                &overlay_text,
                &mut sources,
                &mut diags,
            )
            .expect("known-good example");
            ciac_sema::analyze(&program, &mut diags).expect("known-good example")
        }),
    });

    phases.push(PhaseMeasurement {
        metric: "codegen::model::build_system".to_owned(),
        stats: measure(reps, || ciac_codegen::model::build_system(&ir, &opts)),
    });

    phases.push(PhaseMeasurement {
        metric: "semantic_model::from_ir".to_owned(),
        stats: measure(reps, || SemanticModel::from_ir(&ir)),
    });

    let sm = SemanticModel::from_ir(&ir);
    phases.push(PhaseMeasurement {
        metric: "semantic_model::semantic_hash".to_owned(),
        stats: measure(reps, || sm.semantic_hash()),
    });

    // Regeneration plan, cold: an empty scratch directory, no prior
    // manifest -- every file lands `New`. `plan_regeneration` never
    // writes anything itself, so calling it repeatedly against the
    // same still-empty directory is safe and representative.
    let project = backends
        .iter()
        .find(|b| ciac_codegen::check_support(b.as_ref(), &ir).is_ok())
        .unwrap_or_else(|| panic!("{example}: no backend supports this example"))
        .generate(&ir, &opts)
        .expect("generates");
    let cold_dir = scratch_dir(&format!("cold-{example}"));
    std::fs::create_dir_all(&cold_dir).expect("creating scratch dir");
    phases.push(PhaseMeasurement {
        metric: "regen::plan_regeneration [cold]".to_owned(),
        stats: measure(reps, || {
            plan_regeneration(&project, &cold_dir, None, RegenMode::Normal)
                .expect("plan_regeneration succeeds against an empty dir")
        }),
    });
    std::fs::remove_dir_all(&cold_dir).ok();

    // Regeneration plan, warm: the project already written to disk,
    // with the manifest `build_manifest` would have written after
    // that first build -- every file lands `Unchanged`, matching a
    // real no-op `ciac build` rerun.
    let warm_dir = scratch_dir(&format!("warm-{example}"));
    std::fs::create_dir_all(&warm_dir).expect("creating scratch dir");
    project
        .write_to(&warm_dir)
        .expect("writing fixture project");
    let manifest = build_manifest(
        &project,
        "0.0.0-bench",
        "0.0.0-bench",
        "bench-source-hash",
        "python",
    );
    phases.push(PhaseMeasurement {
        metric: "regen::plan_regeneration [warm]".to_owned(),
        stats: measure(reps, || {
            plan_regeneration(&project, &warm_dir, Some(&manifest), RegenMode::Normal)
                .expect("plan_regeneration succeeds against a matching dir")
        }),
    });

    phases.push(PhaseMeasurement {
        metric: "manifest::build_manifest".to_owned(),
        stats: measure(reps, || {
            build_manifest(
                &project,
                "0.0.0-bench",
                "0.0.0-bench",
                "bench-source-hash",
                "python",
            )
        }),
    });

    let write_dir = scratch_dir(&format!("write-{example}"));
    phases.push(PhaseMeasurement {
        metric: "GeneratedProject::write_to".to_owned(),
        stats: measure(reps, || {
            std::fs::remove_dir_all(&write_dir).ok();
            project
                .write_to(&write_dir)
                .expect("writing fixture project")
        }),
    });
    std::fs::remove_dir_all(&write_dir).ok();
    std::fs::remove_dir_all(&warm_dir).ok();

    let backend_reports = backends
        .iter()
        .filter(|b| ciac_codegen::check_support(b.as_ref(), &ir).is_ok())
        .map(|b| {
            let steady = measure(effective_reps(b.id(), reps), || {
                b.generate(&ir, &opts).expect("generates")
            });
            let p = b.generate(&ir, &opts).expect("generates");
            let (largest_path, largest_bytes) = p
                .files()
                .map(|(path, content)| (path.to_owned(), content.len()))
                .max_by_key(|(_, len)| *len)
                .unwrap_or_default();
            BackendGeneration {
                backend: b.id().to_owned(),
                steady_state: steady,
                file_count: p.len(),
                total_bytes: p.files().map(|(_, c)| c.len()).sum(),
                largest_file_path: largest_path,
                largest_file_bytes: largest_bytes,
            }
        })
        .collect();

    ExampleReport {
        example,
        phases,
        backends: backend_reports,
    }
}

/// A scratch directory unique to this process and label, mirroring
/// `ciac_codegen::format_batch`'s own `scratch_dir` convention (unique
/// per call, safe under concurrent test runs).
fn scratch_dir(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ciac-bench-{label}-{}", std::process::id()))
}
