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
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct PhaseMeasurement {
    pub metric: String,
    pub stats: Stats,
}

/// One backend's generation cost and output shape for one example.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
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
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct TemplateSetup {
    pub backend: String,
    pub first_call_us: f64,
    pub steady_state_us: f64,
    pub setup_us: f64,
}

/// The full phase report for one `.ciac` example.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
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

// ---------------------------------------------------------------------
// `31UpdatePlan.md` M4: the committed baseline -- schema, environment
// capture, and comparison. Built on the measurement primitives above,
// not a duplicate of them: a `Baseline` is exactly one `Vec<TemplateSetup>`
// plus one `Vec<ExampleReport>` plus an [`Environment`] stamp and a
// schema version.
// ---------------------------------------------------------------------

/// Bumped whenever [`Baseline`]'s shape changes in a way that isn't
/// purely additive — a reader comparing two baselines across a version
/// bump should not silently misinterpret a reshaped field. No such
/// bump has happened yet; this is the first schema.
pub const BASELINE_VERSION: u32 = 1;

/// The machine and toolchain a [`Baseline`] was captured on.
///
/// Not optional metadata: this session found that the presence or
/// absence of a single CPU feature (SHA-NI) moves SHA-256's own share
/// of a build from a few percent to 39.79% of it — same code, same
/// input, entirely different profile (`docs/perf/codegen-baseline.md`).
/// A baseline without this stamp is not merely incomplete, it actively
/// misleads whoever reads it next on different hardware. Every
/// toolchain field beyond Rust itself is `Option`: this session's own
/// investigation found the `test` CI job installs only Rust and Go, no
/// JDK/Node/Python, so a baseline captured there must be able to say
/// "not installed in this job" rather than fail to serialize at all.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Environment {
    pub cpu_model: String,
    pub logical_cpus: usize,
    pub sha_ni: bool,
    pub ram_mb: Option<u64>,
    pub kernel: String,
    pub rustc_version: String,
    pub cargo_version: String,
    pub jdk_version: Option<String>,
    pub node_version: Option<String>,
    pub go_version: Option<String>,
    pub python_version: Option<String>,
    /// `"debug"` or `"release"` — `ciac-bench` itself, via
    /// `cfg!(debug_assertions)`, not a claim about anything it
    /// measured (every measurement in this module runs whatever
    /// backend code the workspace's own build profile produced).
    pub profile: String,
    pub git_sha: String,
}

/// Captures the current machine and toolchain. Every external command
/// (`rustc`, `cargo`, `java`, `node`, `go`, `python3`, `git`) is
/// probed independently — one missing tool degrades its own field to
/// `None`/a placeholder string, never the whole capture.
pub fn capture_environment() -> Environment {
    Environment {
        cpu_model: cpuinfo_field("model name").unwrap_or_else(|| "unknown".to_owned()),
        logical_cpus: std::thread::available_parallelism().map_or(0, |n| n.get()),
        sha_ni: cpuinfo_flags_contain("sha_ni"),
        ram_mb: meminfo_total_mb(),
        kernel: command_stdout("uname", &["-r"]).unwrap_or_else(|| "unknown".to_owned()),
        rustc_version: command_stdout("rustc", &["--version"])
            .unwrap_or_else(|| "unknown".to_owned()),
        cargo_version: command_stdout("cargo", &["--version"])
            .unwrap_or_else(|| "unknown".to_owned()),
        jdk_version: command_stderr_first_line("java", &["-version"]),
        node_version: command_stdout("node", &["--version"]),
        go_version: command_stdout("go", &["version"]),
        python_version: command_stdout("python3", &["--version"]),
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
        .to_owned(),
        git_sha: command_stdout("git", &["rev-parse", "HEAD"])
            .unwrap_or_else(|| "unknown".to_owned()),
    }
}

fn cpuinfo_field(key: &str) -> Option<String> {
    let text = std::fs::read_to_string("/proc/cpuinfo").ok()?;
    text.lines()
        .find(|l| l.starts_with(key))
        .and_then(|l| l.split(':').nth(1))
        .map(|v| v.trim().to_owned())
}

fn cpuinfo_flags_contain(flag: &str) -> bool {
    std::fs::read_to_string("/proc/cpuinfo")
        .map(|text| {
            text.lines()
                .find(|l| l.starts_with("flags"))
                .is_some_and(|l| l.split_whitespace().any(|f| f == flag))
        })
        .unwrap_or(false)
}

fn meminfo_total_mb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let kb: u64 = text
        .lines()
        .find(|l| l.starts_with("MemTotal:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some(kb / 1024)
}

fn command_stdout(program: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
}

/// `java -version` writes to stderr, not stdout — and the *first*
/// stderr line is not reliably the version line. Found live capturing
/// this session's own first baseline: this sandbox sets
/// `JAVA_TOOL_OPTIONS` (a proxy configuration, unrelated to the JVM's
/// own identity), and every `java` invocation here prints `"Picked up
/// JAVA_TOOL_OPTIONS: ..."` to stderr *before* the real `openjdk
/// version "21.0.10" ...` line — so naively taking the first stderr
/// line would have committed that proxy noise into
/// `docs/perf/baseline.json` as this environment's "JDK version,"
/// exactly the kind of misleading environment stamp Pillar 4 exists to
/// prevent. Finds the first line containing `"version"` instead
/// (case-sensitive; both `openjdk version` and `java version` output
/// shapes match, `JAVA_TOOL_OPTIONS` does not) — a real JDK's output
/// always has such a line, and if this environment's `java` ever
/// stops producing one, this correctly falls through to `None` rather
/// than silently reporting an unrelated line as the version.
fn command_stderr_first_line(program: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(program)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8_lossy(&o.stderr)
                .lines()
                .find(|l| l.contains("version"))
                .map(|l| l.trim().to_owned())
        })
}

/// The full committed measurement: schema version, environment, and
/// every phase/backend measurement `ciac-bench` produced. `docs/perf/baseline.json`
/// holds one of these; `--update-baseline` is the only path that
/// writes it (`tests/bin/ciac-bench.rs`).
///
/// `instruction_counts` is additive (`31UpdatePlan.md` M6) — present
/// only when `--update-baseline` ran with `--with-callgrind`, absent
/// (empty) otherwise, and `#[serde(default)]` so a `baseline.json`
/// written before M6 still deserializes. No `baseline_version` bump:
/// adding a field is exactly the "purely additive" case the version
/// field's own doc comment carves out.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct Baseline {
    pub baseline_version: u32,
    pub environment: Environment,
    pub template_setup: Vec<TemplateSetup>,
    pub examples: Vec<ExampleReport>,
    #[serde(default)]
    pub instruction_counts: Vec<InstructionCount>,
}

/// One metric's delta between two baselines, e.g.
/// `("order-system", "generate() [python]", 1276.2, 1300.5, 1.9)`.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct MetricDelta {
    pub example: String,
    pub metric: String,
    pub old_mean_us: f64,
    pub new_mean_us: f64,
    pub pct_change: f64,
}

/// Compares every metric two baselines have in common (by example and
/// metric name — a metric present in only one baseline, e.g. after a
/// milestone adds a new phase, is silently skipped rather than
/// reported as an infinite delta). Pure: no I/O, no timing, exactly
/// the split `tests/tests/perf_budget.rs`'s own `check_budget`/
/// `measure_all` already establishes, so this can be unit-tested with
/// synthetic `Baseline` values instead of paying for a real
/// measurement run every time the comparison arithmetic itself needs
/// proving.
pub fn compare_baselines(old: &Baseline, new: &Baseline) -> Vec<MetricDelta> {
    let mut deltas = Vec::new();
    for new_ex in &new.examples {
        let Some(old_ex) = old.examples.iter().find(|e| e.example == new_ex.example) else {
            continue;
        };
        for new_phase in &new_ex.phases {
            let Some(old_phase) = old_ex.phases.iter().find(|p| p.metric == new_phase.metric)
            else {
                continue;
            };
            deltas.push(metric_delta(
                &new_ex.example,
                &new_phase.metric,
                old_phase.stats.mean_us,
                new_phase.stats.mean_us,
            ));
        }
        for new_b in &new_ex.backends {
            let Some(old_b) = old_ex.backends.iter().find(|b| b.backend == new_b.backend) else {
                continue;
            };
            deltas.push(metric_delta(
                &new_ex.example,
                &format!("generate() [{}]", new_b.backend),
                old_b.steady_state.mean_us,
                new_b.steady_state.mean_us,
            ));
        }
    }
    deltas
}

fn metric_delta(example: &str, metric: &str, old_mean_us: f64, new_mean_us: f64) -> MetricDelta {
    let pct_change = if old_mean_us > 0.0 {
        (new_mean_us - old_mean_us) / old_mean_us * 100.0
    } else {
        0.0
    };
    MetricDelta {
        example: example.to_owned(),
        metric: metric.to_owned(),
        old_mean_us,
        new_mean_us,
        pct_change,
    }
}

#[cfg(test)]
mod baseline_tests {
    use super::*;

    fn stats(mean_us: f64) -> Stats {
        Stats {
            mean_us,
            stddev_us: 0.0,
            min_us: mean_us,
            max_us: mean_us,
            p95_us: mean_us,
        }
    }

    fn example(name: &str, metric: &str, mean_us: f64) -> ExampleReport {
        ExampleReport {
            example: name.to_owned(),
            phases: vec![PhaseMeasurement {
                metric: metric.to_owned(),
                stats: stats(mean_us),
            }],
            backends: vec![BackendGeneration {
                backend: "python".to_owned(),
                steady_state: stats(mean_us * 2.0),
                file_count: 10,
                total_bytes: 1000,
                largest_file_path: "app/main.py".to_owned(),
                largest_file_bytes: 100,
            }],
        }
    }

    fn baseline(examples: Vec<ExampleReport>) -> Baseline {
        Baseline {
            baseline_version: BASELINE_VERSION,
            environment: capture_environment(),
            template_setup: Vec::new(),
            examples,
            instruction_counts: Vec::new(),
        }
    }

    /// The harness-can-fail proof this milestone's own exit checklist
    /// requires, mirroring `perf_budget.rs`'s
    /// `fails_when_a_backend_is_artificially_slowed`: a synthetically
    /// regressed metric must produce a correctly-signed, correctly-
    /// computed percentage delta, not merely "some delta."
    #[test]
    fn flags_a_synthetic_regression_with_the_right_sign_and_magnitude() {
        let old = baseline(vec![example("ping", "sema::analyze", 100.0)]);
        let new = baseline(vec![example("ping", "sema::analyze", 150.0)]);
        let deltas = compare_baselines(&old, &new);
        let d = deltas
            .iter()
            .find(|d| d.example == "ping" && d.metric == "sema::analyze")
            .expect("the shared metric is reported");
        assert_eq!(d.old_mean_us, 100.0);
        assert_eq!(d.new_mean_us, 150.0);
        assert!(
            (d.pct_change - 50.0).abs() < 1e-9,
            "expected +50% for 100->150, got {}",
            d.pct_change
        );
    }

    #[test]
    fn flags_a_synthetic_improvement_as_negative() {
        let old = baseline(vec![example("ping", "sema::analyze", 200.0)]);
        let new = baseline(vec![example("ping", "sema::analyze", 100.0)]);
        let deltas = compare_baselines(&old, &new);
        let d = &deltas[0];
        assert!(
            (d.pct_change - (-50.0)).abs() < 1e-9,
            "expected -50% for 200->100, got {}",
            d.pct_change
        );
    }

    #[test]
    fn generate_delta_uses_backend_id_in_its_metric_name() {
        let old = baseline(vec![example("ping", "sema::analyze", 100.0)]);
        let new = baseline(vec![example("ping", "sema::analyze", 100.0)]);
        let deltas = compare_baselines(&old, &new);
        assert!(deltas
            .iter()
            .any(|d| d.metric == "generate() [python]" && d.example == "ping"));
    }

    /// A metric present in only one baseline (e.g. a phase a later
    /// milestone adds) must not appear as a delta at all -- there is
    /// nothing to compare it against, and reporting it as a 100%/−100%
    /// change would be actively misleading.
    #[test]
    fn a_metric_only_present_in_one_baseline_is_skipped_not_reported() {
        let mut new_ex = example("ping", "sema::analyze", 100.0);
        new_ex.phases.push(PhaseMeasurement {
            metric: "brand::new::phase".to_owned(),
            stats: stats(999.0),
        });
        let old = baseline(vec![example("ping", "sema::analyze", 100.0)]);
        let new = baseline(vec![new_ex]);
        let deltas = compare_baselines(&old, &new);
        assert!(!deltas.iter().any(|d| d.metric == "brand::new::phase"));
        assert_eq!(deltas.len(), 2, "sema::analyze + generate() [python] only");
    }

    /// An example present in only one baseline (a corpus addition or
    /// removal) must not crash the comparison and must contribute no
    /// deltas.
    #[test]
    fn an_example_only_present_in_one_baseline_is_skipped_not_reported() {
        let old = baseline(vec![example("ping", "sema::analyze", 100.0)]);
        let new = baseline(vec![
            example("ping", "sema::analyze", 100.0),
            example("brand-new-example", "sema::analyze", 500.0),
        ]);
        let deltas = compare_baselines(&old, &new);
        assert!(!deltas.iter().any(|d| d.example == "brand-new-example"));
    }
}

// ---------------------------------------------------------------------
// `31UpdatePlan.md` M6: the uniform-regression gate. Callgrind
// instruction counts, not wall clock — `docs/perf/noise-floor.md`'s
// own M1 finding is that every wall-clock phase metric this arc
// measured fails the 25% wall-clock-gateable ceiling, so the gate that
// actually catches a shared-path regression (item 1/9 on the
// motivating investigation's inventory: lazy template loading,
// `build_system` deduplication) has to be instruction counts instead.
// ---------------------------------------------------------------------

/// One example's measured instruction count for the gated corpus.
/// Always the python target — the corpus `31UpdatePlan.md` M5's
/// checkpoint fixed (`ping`, `order-system`), on the one target M1's
/// own callgrind determinism study measured.
#[derive(Debug, Clone, Serialize, serde::Deserialize, PartialEq)]
pub struct InstructionCount {
    pub example: String,
    pub instructions: u64,
}

/// The band `31UpdatePlan.md` M5's checkpoint set: 1% of the measured
/// instruction count, ~150x the worst drift M1 observed across two
/// unrelated machines (0.00653%) — see `docs/perf/noise-floor.md`'s
/// own "Gate decisions" section for the full reasoning, including why
/// this replaces the pre-registered `max(3sigma, 10%)` rule's floor
/// term for instruction-count metrics specifically rather than
/// applying it literally.
pub const INSTRUCTION_COUNT_BAND_PCT: f64 = 1.0;

/// Locates the sibling `ciac` release binary, relative to this crate's
/// own manifest directory (mirroring `examples_dir`'s and
/// `baseline_path`'s own `CARGO_MANIFEST_DIR`-relative resolution).
/// Panics with an actionable message rather than a bare "file not
/// found" — this is meant to be run by a human or a CI job that just
/// forgot the build step, not silently skipped.
pub fn resolve_ciac_release_binary() -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../target/release/ciac");
    if !path.is_file() {
        panic!(
            "{} does not exist -- run `cargo build --release -p ciac` first",
            path.display()
        );
    }
    path
}

/// Runs `ciac_binary build <example> --target python --out <scratch>`
/// once under `valgrind --tool=callgrind --cache-sim=no` and returns
/// the reported instruction count. `--cache-sim=no` matches M1's own
/// study: instruction count only, no cache simulation, since cache
/// behavior is itself a source of run-to-run variance this gate does
/// not need for a pass/fail count. Returns `Err` with the actual
/// stderr on any failure (valgrind missing, the build itself failing)
/// rather than panicking, so a caller measuring several examples can
/// report all of their failures rather than stopping at the first.
pub fn measure_instructions(ciac_binary: &Path, example_path: &Path) -> Result<u64, String> {
    let scratch = std::env::temp_dir().join(format!(
        "ciac-callgrind-{}-{}",
        example_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("example"),
        std::process::id()
    ));
    let cg_out = scratch.with_extension("callgrind");
    std::fs::create_dir_all(&scratch)
        .map_err(|e| format!("creating scratch dir {}: {e}", scratch.display()))?;

    let output = std::process::Command::new("valgrind")
        .arg("--tool=callgrind")
        .arg(format!("--callgrind-out-file={}", cg_out.display()))
        .arg("--cache-sim=no")
        .arg("-q")
        .arg(ciac_binary)
        .arg("build")
        .arg(example_path)
        .arg("--target")
        .arg("python")
        .arg("--out")
        .arg(&scratch)
        .output()
        .map_err(|e| format!("running valgrind (is it on PATH?): {e}"))?;

    let result = if !output.status.success() {
        Err(format!(
            "ciac build under valgrind failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    } else {
        let cg_text = std::fs::read_to_string(&cg_out)
            .map_err(|e| format!("reading callgrind output {}: {e}", cg_out.display()))?;
        cg_text
            .lines()
            .find(|l| l.starts_with("summary:"))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|n| n.parse::<u64>().ok())
            .ok_or_else(|| format!("no `summary:` line found in {}", cg_out.display()))
    };

    std::fs::remove_dir_all(&scratch).ok();
    std::fs::remove_file(&cg_out).ok();
    result
}

/// Measures every example in `examples` (by name, under `examples/`)
/// and returns one [`InstructionCount`] per example that measured
/// successfully. A single example's failure is logged to stderr and
/// skipped rather than aborting the whole sweep — matching
/// `perf_budget.rs`'s own "one backend's refusal doesn't invalidate
/// the others" posture, applied here to one example's measurement
/// instead of one backend's support.
pub fn measure_instruction_counts(examples: &[&str]) -> Vec<InstructionCount> {
    let binary = resolve_ciac_release_binary();
    examples
        .iter()
        .filter_map(|name| {
            let path = examples_path(name);
            match measure_instructions(&binary, &path) {
                Ok(instructions) => Some(InstructionCount {
                    example: (*name).to_owned(),
                    instructions,
                }),
                Err(e) => {
                    eprintln!("measuring {name}: {e}");
                    None
                }
            }
        })
        .collect()
}

fn examples_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../examples")
        .join(format!("{name}.ciac"))
}

/// Pure comparison, mirroring `tests/tests/perf_budget.rs`'s own
/// `check_budget`: kept separate from measurement so it can be
/// exercised with synthetic data — the harness-can-fail proof M6's own
/// exit checklist requires — without paying for a real valgrind run
/// every time the comparison arithmetic itself needs proving. A
/// current example absent from `baseline` (a corpus addition) is
/// skipped, not reported as a fabricated regression, mirroring
/// `compare_baselines`'s own M4 convention.
pub fn check_instruction_budget(
    current: &[InstructionCount],
    baseline: &[InstructionCount],
    band_pct: f64,
) -> Result<(), String> {
    let mut offenders = Vec::new();
    for c in current {
        let Some(b) = baseline.iter().find(|b| b.example == c.example) else {
            continue;
        };
        let pct_change = if b.instructions > 0 {
            (c.instructions as f64 - b.instructions as f64) / b.instructions as f64 * 100.0
        } else {
            0.0
        };
        if pct_change > band_pct {
            offenders.push((
                c.example.clone(),
                b.instructions,
                c.instructions,
                pct_change,
            ));
        }
    }
    if offenders.is_empty() {
        return Ok(());
    }
    let mut report = format!(
        "instruction-count budget exceeded (band +{band_pct}% over the committed baseline):\n"
    );
    for (example, baseline_ir, current_ir, pct) in &offenders {
        report.push_str(&format!(
            "  {example:<20} {baseline_ir:>12} -> {current_ir:>12}  {pct:>+7.2}%\n"
        ));
    }
    Err(report)
}

#[cfg(test)]
mod instruction_budget_tests {
    use super::*;

    fn counts(pairs: &[(&str, u64)]) -> Vec<InstructionCount> {
        pairs
            .iter()
            .map(|(example, instructions)| InstructionCount {
                example: (*example).to_owned(),
                instructions: *instructions,
            })
            .collect()
    }

    #[test]
    fn passes_when_unchanged() {
        let baseline = counts(&[("ping", 12_972_120), ("order-system", 45_000_000)]);
        let current = baseline.clone();
        assert!(check_instruction_budget(&current, &baseline, INSTRUCTION_COUNT_BAND_PCT).is_ok());
    }

    #[test]
    fn passes_within_the_band() {
        let baseline = counts(&[("ping", 12_972_120)]);
        // +0.5%: well inside the 1% band, the kind of drift M1's own
        // cross-machine callgrind study actually observed (0.00653%
        // worst case) with a wide margin still to spare.
        let current = counts(&[("ping", 12_972_120 + 64_860)]);
        assert!(check_instruction_budget(&current, &baseline, INSTRUCTION_COUNT_BAND_PCT).is_ok());
    }

    /// The harness-can-fail proof this milestone's own exit checklist
    /// requires: a synthetically regressed instruction count must
    /// actually fail the check, by construction.
    #[test]
    fn fails_when_a_shared_path_regresses() {
        let baseline = counts(&[("ping", 12_972_120), ("order-system", 45_000_000)]);
        // +5%: the shape of a real regression (e.g. eager template
        // parsing reintroduced), not noise -- M1 measured worst-case
        // drift of 0.00653%, three orders of magnitude smaller.
        let current = counts(&[("ping", 12_972_120 + 648_606), ("order-system", 45_000_000)]);
        let err = check_instruction_budget(&current, &baseline, INSTRUCTION_COUNT_BAND_PCT)
            .expect_err("a +5% regression must trip the budget");
        assert!(
            err.contains("ping"),
            "report should name the offending example: {err}"
        );
        assert!(
            !err.contains("order-system"),
            "an unregressed example must not be reported: {err}"
        );
    }

    #[test]
    fn an_example_only_in_current_is_skipped_not_reported() {
        let baseline = counts(&[("ping", 12_972_120)]);
        let current = counts(&[("ping", 12_972_120), ("brand-new-example", 999_999_999)]);
        assert!(check_instruction_budget(&current, &baseline, INSTRUCTION_COUNT_BAND_PCT).is_ok());
    }

    #[test]
    fn an_improvement_never_fails_the_budget() {
        let baseline = counts(&[("ping", 12_972_120)]);
        let current = counts(&[("ping", 10_000_000)]);
        assert!(check_instruction_budget(&current, &baseline, INSTRUCTION_COUNT_BAND_PCT).is_ok());
    }
}

// ---------------------------------------------------------------------
// `31UpdatePlan.md` M7: the asymptotic guard. A synthetic corpus at N,
// 2N and 4N, shared here between `ciac-bench` and
// `tests/tests/perf_scaling.rs` (open question 4: the corpus generator
// lives in exactly one place so a gate can never grow a private copy
// that quietly drifts from what a human inspects by hand). Unlike the
// M6 gate, this one is runner-independent by construction -- it
// compares two measurements from the *same* run on the *same*
// machine, so it needs no noise-floor calibration at all.
// ---------------------------------------------------------------------

/// One "unit" of synthetic load: a `crud`-managed record, a
/// `table`-managed record, a stream, two handlers, one api and one
/// worker, wired into two pipelines -- the same crud/table split
/// `examples/order-system.ciac` itself documents as forced (a `crud`
/// resource and a handler-addressed `table` can never be the same
/// record), repeated `n` times under a distinct numeric suffix so
/// every declared name in the corpus stays unique. `ciac check`
/// reports one benign `CIAC0007` ("stream has no publisher") per
/// unit -- each unit's worker only *consumes* its own stream, nothing
/// in this corpus ever publishes to it -- the same disclosed-benign
/// warning shape `order-system.ciac`'s own comments already carry for
/// an unrelated stream in that file.
pub fn synthetic_corpus(n: usize) -> String {
    let mut src =
        String::from("service SyntheticScale;\n\nuse {\n    db Postgres;\n    queue NATS;\n}\n");
    for i in 0..n {
        src.push_str(&format!(
            r#"
record Entity{i} {{
    id: Uuid;
    name: String;
    value: Int;
}}
crud Entity{i}: Entity{i};

record Widget{i} {{
    id: Uuid;
    entity_id: Uuid;
    status: enum {{ Pending, Done }};
}}
table Widgets{i}: Widget{i};
stream Events{i}: Widget{i};

handler Compute{i}(payload: Widget{i}) -> Widget{i} {{
    let updated = Widget{i} {{
        id: payload.id,
        entity_id: payload.entity_id,
        status: Done
    }};
    db.update(Widgets{i}, updated.id, updated);
    return updated;
}}

handler Notify{i}(payload: Widget{i}) -> Widget{i} {{
    return payload;
}}

api ComputeApi{i}: Widget{i} {{
    method: POST;
    path: "/widgets{i}/compute";
}}
pipeline ComputeApi{i}: Compute{i} -> Return;

worker EventWorker{i} on Events{i};
pipeline EventWorker{i}: Notify{i};
"#
        ));
    }
    src
}

/// `sema::analyze` and (python target) `generate()` timings for one
/// synthetic corpus size.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct ScalingPoint {
    pub n: usize,
    pub analyze: Stats,
    pub generate: Stats,
}

/// Measures the front end and python-target generation cost against
/// [`synthetic_corpus`]`(n)`. Writes the corpus to its own scratch
/// file first -- [`load`]'s own doc comment explains why
/// `ciac_syntax::load` needs a real path rather than an in-memory
/// buffer -- and reuses [`measure`] for the timing itself, so this
/// reports the same shape of numbers `measure_example`'s own
/// `sema::analyze` phase does, just against a corpus whose size is a
/// parameter instead of whatever one committed example happens to
/// contain.
pub fn measure_scaling(n: usize, reps: u32) -> ScalingPoint {
    let src = synthetic_corpus(n);
    let dir = scratch_dir(&format!("scaling-n{n}"));
    std::fs::create_dir_all(&dir).expect("creating scratch dir");
    let path = dir.join("synthetic.ciac");
    std::fs::write(&path, &src).expect("writing synthetic corpus");

    let (_, ir) = load(&path);
    let opts = GenOptions::default();
    let backend = ciac_backend_python::PythonBackend;

    let analyze = measure(reps, || {
        let mut sources = SourceMap::new();
        let mut diags = Diagnostics::new();
        let program = ciac_syntax::load(&path, &mut sources, &mut diags)
            .expect("synthetic corpus is well-formed by construction");
        ciac_sema::analyze(&program, &mut diags)
            .expect("synthetic corpus is well-formed by construction")
    });

    let generate = measure(reps, || {
        backend
            .generate(&ir, &opts)
            .expect("python backend supports any corpus this generator can produce")
    });

    std::fs::remove_dir_all(&dir).ok();

    ScalingPoint {
        n,
        analyze,
        generate,
    }
}

/// Ratio of two mean timings, e.g. the cost of a 2N corpus over an N
/// corpus. Pure and separate from [`measure_scaling`] for the same
/// reason `check_instruction_budget` is separate from
/// `measure_instruction_counts`: the arithmetic can be unit-tested
/// with synthetic data, without paying for two real front-end runs
/// every time the comparison itself needs proving.
pub fn growth_ratio(from_mean_us: f64, to_mean_us: f64) -> f64 {
    if from_mean_us > 0.0 {
        to_mean_us / from_mean_us
    } else {
        0.0
    }
}

/// `sema::analyze` measured on this session's own machine
/// (`31UpdatePlan.md` M7, N=50/100/200/400, 15 reps each): per-doubling
/// ratios of 1.93x, 2.80x, 2.72x -- noisy at these sample counts but
/// trending toward the higher end as N grows, consistent with a real
/// quadratic term dominating a smaller linear one. Smaller than the
/// ~3.7x/doubling the motivating investigation found on a
/// differently-shaped corpus (`docs/perf/noise-floor.md` quotes it),
/// which is expected -- a different corpus shape exercises the same
/// underlying quadratic to a different degree, not a different fact
/// about the codebase. This ceiling is set at 4.5x -- comfortably
/// above the worst doubling this measurement observed (2.80x), not a
/// pass mark and not an endorsement of the current exponent. It exists
/// to catch a *further* regression (e.g. an accidentally-quadratic
/// lookup added on top of the existing one), and is meant to be
/// ratcheted down once inventory item 10 (IR indexes) lands and the
/// known quadratic itself is fixed.
pub const SEMA_ANALYZE_GROWTH_CEILING: f64 = 4.5;

/// python `generate()` measured the same session, same corpus:
/// per-doubling ratios of 2.13x, 2.17x, 1.96x -- close to linear,
/// matching the motivating investigation's own "`generate()` is
/// linear" finding on a different corpus. Ceiling set at 3.0x --
/// enough margin for legitimate per-file overhead to grow somewhat
/// without false-firing on noise, tight enough that a regression to
/// the `sema::analyze` shape (list-in-a-loop, `O(n^2)` string
/// building) would still trip it.
pub const GENERATE_GROWTH_CEILING: f64 = 3.0;

/// Pure comparison: does `to`'s cost, relative to `from`'s, stay under
/// `ceiling`? Mirrors `check_instruction_budget`'s and `check_budget`'s
/// own shape.
pub fn check_growth_ceiling(
    from_mean_us: f64,
    to_mean_us: f64,
    ceiling: f64,
    label: &str,
) -> Result<(), String> {
    let ratio = growth_ratio(from_mean_us, to_mean_us);
    if ratio > ceiling {
        Err(format!(
            "{label}: growth ratio {ratio:.3}x exceeds the {ceiling:.3}x ceiling ({from_mean_us:.1}us -> {to_mean_us:.1}us)"
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod scaling_tests {
    use super::*;

    #[test]
    fn synthetic_corpus_is_empty_but_valid_shape_at_n_zero() {
        let src = synthetic_corpus(0);
        assert!(src.contains("service SyntheticScale;"));
        assert!(!src.contains("record Entity0"));
    }

    #[test]
    fn synthetic_corpus_grows_one_unit_per_n() {
        let src = synthetic_corpus(3);
        for i in 0..3 {
            assert!(src.contains(&format!("record Entity{i} ")));
            assert!(src.contains(&format!("record Widget{i} ")));
            assert!(src.contains(&format!("crud Entity{i}: Entity{i};")));
            assert!(src.contains(&format!("table Widgets{i}: Widget{i};")));
        }
        assert!(!src.contains("Entity3"));
    }

    #[test]
    fn growth_ratio_of_equal_costs_is_one() {
        assert_eq!(growth_ratio(1000.0, 1000.0), 1.0);
    }

    #[test]
    fn passes_when_growth_stays_under_the_ceiling() {
        // A 2x cost increase for a 2x-larger corpus: linear, well
        // under any of this milestone's ceilings.
        assert!(check_growth_ceiling(1000.0, 2000.0, SEMA_ANALYZE_GROWTH_CEILING, "test").is_ok());
    }

    /// The harness-can-fail proof this milestone's own exit checklist
    /// requires: a synthetically superlinear growth must actually fail
    /// the check, by construction.
    #[test]
    fn fails_when_growth_exceeds_the_ceiling() {
        // 10x the cost for a 2x-larger corpus -- the shape of an
        // accidentally-introduced O(n^2) lookup on top of the existing
        // one, not measurement noise.
        let err = check_growth_ceiling(
            1000.0,
            10_000.0,
            SEMA_ANALYZE_GROWTH_CEILING,
            "sema::analyze",
        )
        .expect_err("a 10x ratio must trip a 4.5x ceiling");
        assert!(err.contains("sema::analyze"));
        assert!(err.contains("10.000x"));
    }

    #[test]
    fn an_improvement_never_fails_the_ceiling() {
        assert!(check_growth_ceiling(1000.0, 500.0, SEMA_ANALYZE_GROWTH_CEILING, "test").is_ok());
    }
}
