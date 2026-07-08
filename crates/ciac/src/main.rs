//! The `ciac` command-line interface.

mod commands;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "ciac",
    version,
    about = "Compile declarative backend architectures into runnable systems"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse and validate a CIaC program, reporting diagnostics.
    Check {
        /// Path to the `.ciac` source file.
        file: PathBuf,
    },
    /// Compile a CIaC program into a backend project.
    Build {
        /// Path to the `.ciac` source file.
        file: PathBuf,
        /// Code-generation target.
        #[arg(short, long)]
        target: String,
        /// Output directory for the generated project.
        #[arg(short, long)]
        out: PathBuf,
        /// Allow writing into a non-empty output directory.
        #[arg(long)]
        force: bool,
        /// Adopt a pre-v0.6 output tree by preserving existing files and
        /// writing sidecars for generated content that would replace them.
        #[arg(long)]
        adopt: bool,
        /// Also emit Kubernetes manifests under `k8s/` (only accepted
        /// value today: `k8s`). Compose remains the dev default; this
        /// is additive production deployment posture.
        #[arg(long, value_name = "TARGET")]
        deploy: Option<String>,
        /// Image name prefix for `--deploy k8s` manifests (default: the
        /// project name). Build and push `{prefix}[-<service>]:<tag>`
        /// from the generated `Dockerfile` before applying the
        /// manifests — `ciac` emits the deployment shape, not a
        /// registry pipeline.
        #[arg(long)]
        image_prefix: Option<String>,
        /// Image tag for `--deploy k8s` manifests.
        #[arg(long, default_value = "latest")]
        image_tag: String,
        /// Override the generated project's name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Show what regeneration would change without writing files.
    Diff {
        /// Path to the `.ciac` source file.
        file: PathBuf,
        /// Code-generation target.
        #[arg(short, long)]
        target: String,
        /// Output directory to compare against.
        #[arg(short, long)]
        out: PathBuf,
        /// Print unified diffs for changed/conflicting files.
        #[arg(long)]
        patch: bool,
        /// Override the generated project's name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Verify an existing generated project still matches its CIaC source.
    Verify {
        /// Path to the `.ciac` source file.
        file: PathBuf,
        /// Code-generation target.
        #[arg(short, long)]
        target: String,
        /// Output directory containing the generated project.
        #[arg(short, long)]
        out: PathBuf,
        /// Boot the generated service and probe `/health` (experimental).
        #[arg(long)]
        live: bool,
        /// Run the generated `tests/system/` suite over a `docker
        /// compose`-booted stack, proving cross-service edges (calls,
        /// broker delivery, channels) actually work. Requires Docker;
        /// a no-op when the program has no whole-system edges.
        #[arg(long)]
        system: bool,
        /// Override the generated project's name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Dump the validated system graph.
    Graph {
        /// Path to the `.ciac` source file.
        file: PathBuf,
        /// Output format.
        #[arg(long, default_value = "json", value_parser = ["json", "dot"])]
        format: String,
    },
    /// Dump the external-backend wire contract for a target, without
    /// running any backend — a `ciac-backend-<name>` executable (not
    /// yet implemented by any target) would receive this same JSON on
    /// stdin.
    CodegenRequest {
        /// Path to the `.ciac` source file.
        file: PathBuf,
        /// Code-generation target this request would be built for.
        #[arg(short, long)]
        target: String,
        /// Override the generated project's name.
        #[arg(long)]
        name: Option<String>,
    },
    /// Explain an error code, e.g. `ciac explain CIAC0006`.
    Explain {
        /// The error code to explain.
        code: String,
    },
    /// List available code-generation targets.
    Targets,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Check { file } => commands::check(&file),
        Command::Build {
            file,
            target,
            out,
            force,
            adopt,
            deploy,
            image_prefix,
            image_tag,
            name,
        } => commands::build(
            &file,
            &target,
            &out,
            force,
            adopt,
            commands::DeployOpts {
                deploy,
                image_prefix,
                image_tag,
            },
            name,
        ),
        Command::Diff {
            file,
            target,
            out,
            patch,
            name,
        } => commands::diff(&file, &target, &out, patch, name),
        Command::Verify {
            file,
            target,
            out,
            live,
            system,
            name,
        } => commands::verify(&file, &target, &out, live, system, name),
        Command::Graph { file, format } => commands::graph(&file, &format),
        Command::CodegenRequest { file, target, name } => {
            commands::codegen_request(&file, &target, name)
        }
        Command::Explain { code } => commands::explain(&code),
        Command::Targets => commands::targets(),
    }
}
