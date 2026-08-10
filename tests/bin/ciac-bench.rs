//! `31UpdatePlan.md` M2/M4: the CLI over `ciac_integration_tests::bench`.
//!
//! ```text
//! cargo run -q -p ciac-integration-tests --bin ciac-bench -- [--format=table|json] [--reps=N] [--update-baseline] [--compare=PATH] [EXAMPLE...]
//! ```
//!
//! `EXAMPLE...` names files under `examples/` without the `.ciac`
//! extension; omitted, it defaults to the same four-example corpus
//! `docs/perf/noise-floor.md`'s M1 sweep used
//! (`ping`, `order-system`, `domain-orders`, `sim-three-service`), so
//! this binary's own output is directly comparable to that document
//! without extra flags.
//!
//! `--update-baseline` writes the current run to `docs/perf/baseline.json`
//! (with a `baseline_version`/environment stamp — see
//! `ciac_integration_tests::bench::Baseline`), overwriting whatever was
//! there. Per `31UpdatePlan.md`'s own discipline ("`--update-baseline`
//! requires a justification in the commit message"), this binary does
//! not enforce that itself — it is a commit-review discipline, not
//! something a CLI flag can check.
//!
//! `--compare=PATH` reads a previously-written baseline from `PATH` and
//! prints a per-metric delta table against the current run, via
//! `ciac_integration_tests::bench::compare_baselines` — a pure function,
//! unit-tested against synthetic regressions in `bench.rs` itself, not
//! duplicated here.
//!
//! `--with-scaling` (`31UpdatePlan.md` M7) runs the asymptotic-guard
//! corpus at N=100/200/400 and prints `sema::analyze`/`generate()`
//! timings plus the growth ratio each doubling produced, via
//! `ciac_integration_tests::bench::{measure_scaling, growth_ratio}` —
//! the same functions `tests/tests/perf_scaling.rs`'s own gate calls,
//! per that milestone's "one generator, shared" rule (open question 4).
//! This data is not written to `docs/perf/baseline.json`: the guard
//! compares two measurements from the same run, so unlike M6's
//! instruction counts it has no cross-run baseline to store.

use ciac_integration_tests::bench::{
    capture_environment, compare_baselines, growth_ratio, measure_example,
    measure_instruction_counts, measure_scaling, measure_template_setup, Baseline, ExampleReport,
    ScalingPoint, TemplateSetup, BASELINE_VERSION,
};
use ciac_integration_tests::{backends, compile_file, examples_dir};
use std::path::{Path, PathBuf};

/// `31UpdatePlan.md` M7's own corpus sizes, matching
/// `tests/tests/perf_scaling.rs`'s gate exactly, so `--with-scaling`'s
/// output is directly comparable to that test's own.
const SCALING_SIZES: &[usize] = &[100, 200, 400];
const SCALING_REPS: u32 = 20;

const DEFAULT_EXAMPLES: &[&str] = &["ping", "order-system", "domain-orders", "sim-three-service"];

/// `31UpdatePlan.md` M5's checkpoint: the corpus M6's callgrind gate
/// actually covers. Deliberately narrower than [`DEFAULT_EXAMPLES`] —
/// callgrind's own 10-50x slowdown means the gated corpus stays small
/// and fixed, not the full sweep (`docs/perf/noise-floor.md`).
const CALLGRIND_EXAMPLES: &[&str] = &["ping", "order-system"];

/// `docs/perf/baseline.json`, resolved relative to this crate's own
/// manifest directory the same way `ciac_integration_tests::examples_dir`
/// resolves `examples/` — works regardless of the caller's own working
/// directory (`cargo run` from the repo root, from `tests/`, or from
/// CI's checkout root all resolve the same file).
fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/perf/baseline.json")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut format = "table".to_owned();
    let mut reps: u32 = 40;
    let mut update_baseline = false;
    let mut with_callgrind = false;
    let mut with_scaling = false;
    let mut compare_path: Option<PathBuf> = None;
    let mut names: Vec<String> = Vec::new();
    for arg in args {
        if let Some(v) = arg.strip_prefix("--format=") {
            format = v.to_owned();
        } else if let Some(v) = arg.strip_prefix("--reps=") {
            reps = v
                .parse()
                .unwrap_or_else(|_| panic!("--reps expects an integer, got {v}"));
        } else if arg == "--update-baseline" {
            update_baseline = true;
        } else if arg == "--with-callgrind" {
            with_callgrind = true;
        } else if arg == "--with-scaling" {
            with_scaling = true;
        } else if let Some(v) = arg.strip_prefix("--compare=") {
            compare_path = Some(PathBuf::from(v));
        } else if arg.starts_with("--") {
            panic!("unknown flag {arg}");
        } else {
            names.push(arg);
        }
    }
    if names.is_empty() {
        names = DEFAULT_EXAMPLES.iter().map(|s| s.to_string()).collect();
    }

    let paths: Vec<PathBuf> = names
        .iter()
        .map(|n| examples_dir().join(format!("{n}.ciac")))
        .collect();
    for p in &paths {
        if !p.is_file() {
            panic!("no such example: {}", p.display());
        }
    }

    // Template-setup measurement must run first, exactly once, before
    // any other `generate()` call touches a backend's own process-wide
    // template cache -- see `bench::measure_template_setup`'s own doc
    // comment for why. Uses the first named example as its fixture.
    let ir = compile_file(&paths[0]);
    let setup = measure_template_setup(&ir, &Default::default(), &backends(), reps);

    let reports: Vec<ExampleReport> = paths
        .iter()
        .map(|p| measure_example(p, reps, &backends()))
        .collect();

    match format.as_str() {
        "json" => print_json(&setup, &reports),
        "table" => print_table(&setup, &reports),
        other => panic!("unknown --format {other}; expected table or json"),
    }

    if with_scaling {
        eprintln!("measuring the asymptotic-guard corpus at N=100/200/400 (this takes a while)...");
        let points: Vec<ScalingPoint> = SCALING_SIZES
            .iter()
            .map(|&n| measure_scaling(n, SCALING_REPS))
            .collect();
        print_scaling(&points);
    }

    if update_baseline || compare_path.is_some() {
        // `--with-callgrind` measures fresh; otherwise carry forward
        // whatever instruction counts the existing baseline.json
        // already has (if any) rather than silently erasing M6's own
        // data on every routine wall-clock-only `--update-baseline`
        // run from a later milestone.
        let instruction_counts = if with_callgrind {
            eprintln!("measuring instruction counts under valgrind (this is slow)...");
            measure_instruction_counts(CALLGRIND_EXAMPLES)
        } else {
            existing_baseline(&baseline_path())
                .map(|b| b.instruction_counts)
                .unwrap_or_default()
        };

        let current = Baseline {
            baseline_version: BASELINE_VERSION,
            environment: capture_environment(),
            template_setup: setup,
            examples: reports,
            instruction_counts,
        };

        if let Some(path) = &compare_path {
            let text = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("reading baseline {}: {e}", path.display()));
            let previous: Baseline = serde_json::from_str(&text)
                .unwrap_or_else(|e| panic!("parsing baseline {}: {e}", path.display()));
            print_comparison(&previous, &current);
        }

        if update_baseline {
            let path = baseline_path();
            let json = serde_json::to_string_pretty(&current).expect("Baseline serializes");
            std::fs::write(&path, format!("{json}\n"))
                .unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
            eprintln!("wrote {}", path.display());
        }
    }
}

fn existing_baseline(path: &Path) -> Option<Baseline> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn print_comparison(previous: &Baseline, current: &Baseline) {
    let deltas = compare_baselines(previous, current);
    println!(
        "### Comparison against baseline (git {})\n",
        previous.environment.git_sha
    );
    println!("| Example | Metric | Old (us) | New (us) | Change |");
    println!("|---|---|---|---|---|");
    for d in &deltas {
        println!(
            "| {} | {} | {:.1} | {:.1} | {:+.1}% |",
            d.example, d.metric, d.old_mean_us, d.new_mean_us, d.pct_change
        );
    }
    println!();
}

fn print_json(setup: &[TemplateSetup], reports: &[ExampleReport]) {
    #[derive(serde::Serialize)]
    struct Output<'a> {
        template_setup: &'a [TemplateSetup],
        examples: &'a [ExampleReport],
    }
    let out = Output {
        template_setup: setup,
        examples: reports,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&out).expect("serializes")
    );
}

fn print_table(setup: &[TemplateSetup], reports: &[ExampleReport]) {
    println!("### Template setup (measured once, first example only)\n");
    println!("| Backend | first call (us) | steady state (us) | setup (us) |");
    println!("|---|---|---|---|");
    for s in setup {
        println!(
            "| {:<12} | {:>14.1} | {:>16.1} | {:>10.1} |",
            s.backend, s.first_call_us, s.steady_state_us, s.setup_us
        );
    }
    println!();

    for r in reports {
        println!("### {}\n", r.example);
        println!("| Metric | mean (us) | stddev | rel stddev | min | max | p95 |");
        println!("|---|---|---|---|---|---|---|");
        for p in &r.phases {
            println!(
                "| {:<32} | {:>10.1} | {:>9.1} | {:>7.1}% | {:>10.1} | {:>10.1} | {:>10.1} |",
                p.metric,
                p.stats.mean_us,
                p.stats.stddev_us,
                p.stats.rel_stddev_pct(),
                p.stats.min_us,
                p.stats.max_us,
                p.stats.p95_us
            );
        }
        for b in &r.backends {
            println!(
                "| {:<32} | {:>10.1} | {:>9.1} | {:>7.1}% | {:>10.1} | {:>10.1} | {:>10.1} |",
                format!("generate() [{}]", b.backend),
                b.steady_state.mean_us,
                b.steady_state.stddev_us,
                b.steady_state.rel_stddev_pct(),
                b.steady_state.min_us,
                b.steady_state.max_us,
                b.steady_state.p95_us
            );
        }
        println!();
        println!("output shape:");
        for b in &r.backends {
            println!(
                "  {:<12} {} files, {} bytes, largest {} ({} bytes)",
                b.backend, b.file_count, b.total_bytes, b.largest_file_path, b.largest_file_bytes
            );
        }
        println!();
    }
}

fn print_scaling(points: &[ScalingPoint]) {
    println!("### Asymptotic guard (synthetic corpus, 31UpdatePlan.md M7)\n");
    println!("| N | sema::analyze (us) | growth | generate() [python] (us) | growth |");
    println!("|---|---|---|---|---|");
    for (i, p) in points.iter().enumerate() {
        let (analyze_growth, generate_growth) = match points.get(i.wrapping_sub(1)) {
            Some(prev) if i > 0 => (
                growth_ratio(prev.analyze.mean_us, p.analyze.mean_us),
                growth_ratio(prev.generate.mean_us, p.generate.mean_us),
            ),
            _ => (0.0, 0.0),
        };
        if i == 0 {
            println!(
                "| {:<5} | {:>18.1} | {:<6} | {:>24.1} | {:<6} |",
                p.n, p.analyze.mean_us, "--", p.generate.mean_us, "--"
            );
        } else {
            println!(
                "| {:<5} | {:>18.1} | {:>5.2}x | {:>24.1} | {:>5.2}x |",
                p.n, p.analyze.mean_us, analyze_growth, p.generate.mean_us, generate_growth
            );
        }
    }
    println!();
}
