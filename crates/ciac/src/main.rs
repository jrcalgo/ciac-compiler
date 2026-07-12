//! The `ciac` command-line interface.

mod commands;
mod describe;
mod dev;
mod json_out;
mod lsp;
mod mcp;
mod scaffold;
mod vocab;

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
    /// Scaffold a new CIaC project directory from an embedded template.
    New {
        /// Directory to create (must not exist or must be empty).
        dir: PathBuf,
        /// Project template. Each is a checked-in example the test
        /// suite already compiles: `crud` (typed CRUD service),
        /// `multi-service` (cross-service call + verifiable
        /// capabilities), `kafka` (event ingestion), `minimal`
        /// (one api, no capabilities).
        #[arg(
            long,
            default_value = "crud",
            value_parser = ["crud", "multi-service", "kafka", "minimal"]
        )]
        template: String,
    },
    /// Parse and validate a CIaC program, reporting diagnostics.
    Check {
        /// Path to the `.ciac` source file.
        file: PathBuf,
        /// Emit one machine-readable JSON document on stdout
        /// (diagnostics with resolved file/line/column, plus success)
        /// instead of only human-readable text; human narration stays
        /// on stderr.
        #[arg(long)]
        json: bool,
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
        /// Also emit deployment artifacts: `k8s` (manifests under
        /// `k8s/`) and/or `terraform` (AWS modules for stateful
        /// capabilities under `terraform/`). Repeatable. Compose
        /// remains the dev default; these are additive production
        /// posture.
        #[arg(long, value_name = "TARGET")]
        deploy: Vec<String>,
        /// Sizing profile for `--deploy` artifacts (k8s replicas and
        /// resources, Terraform instance classes): dev, staging, prod.
        #[arg(long, default_value = "dev")]
        profile: String,
        /// With `--deploy k8s`: move secret-shaped env values
        /// (JWT_SECRET) out of the ConfigMap into a generated Secret
        /// manifest wired via secretRef. Placeholder values --
        /// override before applying.
        #[arg(long)]
        secrets: bool,
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
        /// Emit one machine-readable JSON document on stdout
        /// (diagnostics with resolved file/line/column, plus success)
        /// instead of only human-readable text; human narration stays
        /// on stderr.
        #[arg(long)]
        json: bool,
    },
    /// Watch the program's source files and keep a regenerated, live
    /// compose stack in sync: on save, recompile (compile errors keep
    /// the last good stack running), regenerate through the same
    /// sidecar-safe path `ciac build` uses, restart the stack, and
    /// re-probe every service's /health route.
    Dev {
        /// Path to the `.ciac` source file.
        file: PathBuf,
        /// Code-generation target.
        #[arg(short, long)]
        target: String,
        /// Output directory for the generated project.
        #[arg(short, long)]
        out: PathBuf,
        /// Leave the compose stack running on exit instead of tearing
        /// it down.
        #[arg(long)]
        keep: bool,
        /// Watch + regenerate only: never touch Docker. For pairing
        /// with a hand-run process or zero-container (SQLite) programs.
        #[arg(long)]
        no_docker: bool,
        /// Use filesystem polling instead of native change events
        /// (for filesystems where inotify/fsevents misbehave).
        #[arg(long)]
        poll: bool,
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
        /// Emit the regeneration plan as one machine-readable JSON
        /// document on stdout (with `--patch`, including unified diff
        /// text per changed entry).
        #[arg(long)]
        json: bool,
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
        /// Boot the generated compose stack and probe every service's
        /// `/health` route until it answers (bounded backoff),
        /// reporting per-service up/down. Requires Docker.
        #[arg(long)]
        live: bool,
        /// Run the generated `tests/system/` suite over a `docker
        /// compose`-booted stack, proving cross-service edges (calls,
        /// broker delivery, channels) and capability round-trips
        /// actually work. Requires Docker; a no-op when the program
        /// has no whole-system edges or verifiable capabilities.
        #[arg(long)]
        system: bool,
        /// Leave the compose stack running after a green `--system` or
        /// `--live` run instead of tearing it down, for local poking.
        /// A failing run always tears down.
        #[arg(long)]
        keep: bool,
        /// Override the generated project's name.
        #[arg(long)]
        name: Option<String>,
        /// Emit one machine-readable JSON document on stdout
        /// (diagnostics with resolved file/line/column, plus success)
        /// instead of only human-readable text; human narration stays
        /// on stderr.
        #[arg(long)]
        json: bool,
    },
    /// Run a Language Server Protocol server over stdio. Configure
    /// `ciac lsp` as the language server command for `.ciac` files;
    /// it publishes the same diagnostics `ciac check` reports (on
    /// open and save), plus hover and completion for the language's
    /// keywords, capabilities, providers, and the file's own
    /// declarations.
    Lsp,
    /// Dump the validated system graph.
    Graph {
        /// Path to the `.ciac` source file.
        file: PathBuf,
        /// Output format.
        #[arg(long, default_value = "json", value_parser = ["json", "dot"])]
        format: String,
    },
    /// Print the JSON Schema for the external-backend wire contract
    /// (`CodegenRequest` on the child's stdin, `CodegenResponse` on
    /// its stdout) — derived from the same types that serialize the
    /// real payloads. `docs/protocol-schema.json` is this output,
    /// checked in.
    CodegenSchema,
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
    /// Print one versioned JSON document naming everything the
    /// language and CLI expose: capabilities, providers (with
    /// per-target support), field types, builtin pipeline steps,
    /// declaration kinds, error codes, and scaffold templates. The
    /// machine-facing counterpart to `ciac lsp`'s hover/completion —
    /// both render from the same tables.
    Describe,
    /// Run a Model Context Protocol server over stdio (newline-
    /// delimited JSON-RPC): exposes `check`, `build`, `diff`,
    /// `verify` (no `--system`/`--live`), `graph`, `explain`, and
    /// `describe` as MCP tools for an agent client to call.
    Mcp,
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
        Command::New { dir, template } => scaffold::new_project(&dir, &template),
        Command::Check { file, json } => commands::check(&file, json),
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
            json,
            profile,
            secrets,
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
                profile,
                secrets,
            },
            name,
            json,
        ),
        Command::Dev {
            file,
            target,
            out,
            keep,
            no_docker,
            poll,
            name,
        } => dev::run(&file, &target, &out, name, keep, no_docker, poll),
        Command::Diff {
            file,
            target,
            out,
            patch,
            name,
            json,
        } => commands::diff(&file, &target, &out, patch, name, json),
        Command::Verify {
            file,
            target,
            out,
            live,
            system,
            keep,
            name,
            json,
        } => commands::verify(&file, &target, &out, live, system, keep, name, json),
        Command::Lsp => lsp::run(),
        Command::Graph { file, format } => commands::graph(&file, &format),
        Command::CodegenSchema => commands::codegen_schema(),
        Command::CodegenRequest { file, target, name } => {
            commands::codegen_request(&file, &target, name)
        }
        Command::Explain { code } => commands::explain(&code),
        Command::Targets => commands::targets(),
        Command::Describe => describe::run(),
        Command::Mcp => mcp::run(),
    }
}
