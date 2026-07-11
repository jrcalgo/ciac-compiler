//! `ciac new` (v0.12 M1): scaffold a fresh project directory from an
//! embedded template.
//!
//! Every template body is `include_str!`ed from a real checked-in
//! example, so a scaffold can never drift from a shape the golden and
//! CI suites already compile and verify — there is no separate
//! "starter" dialect to keep in sync.

use anyhow::{bail, Context, Result};
use std::path::Path;
use std::process::ExitCode;

struct Template {
    name: &'static str,
    source: &'static str,
    summary: &'static str,
    /// Template-specific caveat surfaced in the scaffold README.
    note: Option<&'static str>,
}

/// `--template` names, in `--help` order. Keep in sync with the
/// `value_parser` list on the `New` subcommand.
const TEMPLATES: &[Template] = &[
    Template {
        name: "crud",
        source: include_str!("../../../examples/crud-notes.ciac"),
        summary: "a single service whose `crud Note;` expands into \
                  REST API -> JWT auth -> service -> Postgres (+ Redis cache)",
        note: None,
    },
    Template {
        name: "multi-service",
        source: include_str!("../../../examples/inventory-system.ciac"),
        summary: "two services joined by a cross-service `call` edge, with a \
                  typed CRUD resource whose Postgres/Redis round-trips are \
                  system-verifiable",
        note: Some(
            "`ciac verify main.ciac --target python --out ./build --system` \
             (requires Docker) boots the generated compose stack and proves \
             the cross-service call and the capability round-trips for real.",
        ),
    },
    Template {
        name: "kafka",
        source: include_str!("../../../examples/kafka-pipeline.ciac"),
        summary: "an event-ingestion shape on Kafka: an api publishing to a \
                  stream, a worker consuming it in a consumer group",
        note: None,
    },
    Template {
        name: "minimal",
        source: include_str!("../../../examples/ping.ciac"),
        summary: "the smallest useful program: one record, one api, one \
                  pipeline, no capabilities",
        note: None,
    },
];

/// `(name, summary)` for every `--template` choice — `ciac describe`
/// (v0.13 M5) lists these without embedding the template sources
/// themselves.
pub fn template_summaries() -> Vec<(&'static str, &'static str)> {
    TEMPLATES.iter().map(|t| (t.name, t.summary)).collect()
}

/// Scaffolds `dir` from the named template: `main.ciac` (the embedded
/// example, verbatim) plus a README with the next commands to run.
/// Refuses a non-empty target directory — there is deliberately no
/// `--force` here; regeneration workflows belong to `ciac build`.
pub fn new_project(dir: &Path, template: &str) -> Result<ExitCode> {
    let tpl = TEMPLATES
        .iter()
        .find(|t| t.name == template)
        .with_context(|| format!("unknown template `{template}`"))?;

    if dir.exists() {
        let mut entries = std::fs::read_dir(dir)
            .with_context(|| format!("cannot read target directory {}", dir.display()))?;
        if entries.next().is_some() {
            bail!(
                "target directory {} is not empty; `ciac new` only scaffolds \
                 into a new or empty directory",
                dir.display()
            );
        }
    } else {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("cannot create target directory {}", dir.display()))?;
    }

    std::fs::write(dir.join("main.ciac"), tpl.source)
        .with_context(|| format!("cannot write {}", dir.join("main.ciac").display()))?;
    std::fs::write(dir.join("README.md"), readme(tpl))
        .with_context(|| format!("cannot write {}", dir.join("README.md").display()))?;
    std::fs::write(dir.join("AGENTS.md"), AGENTS_MD)
        .with_context(|| format!("cannot write {}", dir.join("AGENTS.md").display()))?;

    eprintln!(
        "scaffolded the `{}` template into {}",
        tpl.name,
        dir.display()
    );
    eprintln!("next: cd {} && ciac check main.ciac", dir.display());
    Ok(ExitCode::SUCCESS)
}

/// Emitted into every scaffolded project (v0.13 M5) — the front door
/// for an agent that opens this directory cold, before `ciac build`
/// has produced anything to read `AGENTS.md`'s generated-project
/// counterpart (see `commands::agents_md`) about.
const AGENTS_MD: &str = "\
# Agents working in this project\n\
\n\
This directory holds one file that matters: `main.ciac`, the whole\n\
architecture. Everything else (`README.md`, this file, and — once you\n\
run `ciac build` — a generated project tree) is derived from it or\n\
describes it.\n\
\n\
## Loop\n\
\n\
```sh\n\
ciac check main.ciac                                # parse + validate, fast\n\
ciac build main.ciac --target python --out ./build  # generate a runnable project\n\
ciac verify main.ciac --target python --out ./build # regenerate + run its own tests\n\
```\n\
\n\
`ciac verify`'s exit code is the truth signal: it regenerates from the\n\
current source and runs the generated project's own test suite, so a\n\
green `verify` means the source and the generated tree agree and the\n\
generated code actually works — not just that it parsed.\n\
\n\
Once `ciac build` has run, `./build/AGENTS.md` explains that tree's\n\
own owned-vs-seeded rules (which files regenerate freely and which\n\
hold code you write once).\n\
\n\
## Machine-readable output\n\
\n\
`ciac check|build|diff|verify --json` each print one JSON envelope on\n\
stdout (diagnostics resolved to file/line/column, plus success) —\n\
human narration stays on stderr, so the two never interleave.\n\
`ciac describe` prints the language's full vocabulary (capabilities,\n\
providers, field types, error codes) as one versioned JSON document.\n\
`ciac mcp` runs the same commands as a Model Context Protocol server\n\
over stdio, for a client that would rather call a tool than parse a\n\
CLI's stdout.\n\
\n\
## Learn the language\n\
\n\
<https://github.com/jrcalgo/ciac/blob/main/docs/language.md> —\n\
records, apis, pipelines, streams, workers, capabilities, and the\n\
provider support table per target.\n\
";

fn readme(tpl: &Template) -> String {
    let mut doc = format!(
        "# A CIaC project (`{name}` template)\n\
         \n\
         Scaffolded by `ciac new --template {name}`: {summary}.\n\
         \n\
         `main.ciac` is the whole architecture; everything else is\n\
         generated from it.\n\
         \n\
         ## Next steps\n\
         \n\
         ```sh\n\
         ciac check main.ciac\n\
         ciac build main.ciac --target python --out ./build\n\
         ciac verify main.ciac --target python --out ./build\n\
         ```\n\
         \n\
         `ciac build` emits a complete runnable project (app code, tests,\n\
         Dockerfile, docker-compose.yml); `ciac verify` regenerates and\n\
         runs the generated project's own test suite. `ciac targets`\n\
         lists the other code-generation targets.\n",
        name = tpl.name,
        summary = tpl.summary,
    );
    if let Some(note) = tpl.note {
        doc.push_str(&format!("\n## Note\n\n{note}\n"));
    }
    doc.push_str(
        "\n## Learn the language\n\
         \n\
         The language reference lives in the ciac repository:\n\
         <https://github.com/jrcalgo/ciac/blob/main/docs/language.md>\n\
         — records, apis, pipelines, streams, workers, capabilities,\n\
         and the provider support table per target.\n",
    );
    doc
}
