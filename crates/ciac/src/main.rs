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
            name,
        } => commands::build(&file, &target, &out, force, name),
        Command::Graph { file, format } => commands::graph(&file, &format),
        Command::Explain { code } => commands::explain(&code),
        Command::Targets => commands::targets(),
    }
}
