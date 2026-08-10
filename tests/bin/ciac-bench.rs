//! `31UpdatePlan.md` M2: the CLI over `ciac_integration_tests::bench`.
//!
//! ```text
//! cargo run -q -p ciac-integration-tests --bin ciac-bench -- [--format=table|json] [--reps N] [EXAMPLE...]
//! ```
//!
//! `EXAMPLE...` names files under `examples/` without the `.ciac`
//! extension; omitted, it defaults to the same four-example corpus
//! `docs/perf/noise-floor.md`'s M1 sweep used
//! (`ping`, `order-system`, `domain-orders`, `sim-three-service`), so
//! this binary's own output is directly comparable to that document
//! without extra flags. No gate, no baseline read or write here —
//! `31UpdatePlan.md` M4 builds `--update-baseline` on top of this
//! binary's own measurement calls, not by duplicating them.

use ciac_integration_tests::bench::{
    measure_example, measure_template_setup, ExampleReport, TemplateSetup,
};
use ciac_integration_tests::{backends, compile_file, examples_dir};

const DEFAULT_EXAMPLES: &[&str] = &["ping", "order-system", "domain-orders", "sim-three-service"];

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut format = "table".to_owned();
    let mut reps: u32 = 40;
    let mut names: Vec<String> = Vec::new();
    for arg in args {
        if let Some(v) = arg.strip_prefix("--format=") {
            format = v.to_owned();
        } else if let Some(v) = arg.strip_prefix("--reps=") {
            reps = v
                .parse()
                .unwrap_or_else(|_| panic!("--reps expects an integer, got {v}"));
        } else if arg.starts_with("--") {
            panic!("unknown flag {arg}");
        } else {
            names.push(arg);
        }
    }
    if names.is_empty() {
        names = DEFAULT_EXAMPLES.iter().map(|s| s.to_string()).collect();
    }

    let paths: Vec<std::path::PathBuf> = names
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
