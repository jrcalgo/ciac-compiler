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
        note: Some(
            "`queue Kafka` currently generates on the Python target only; \
             the Rust target reports CIAC0011 (see docs/language.md's \
             provider support table).",
        ),
    },
    Template {
        name: "minimal",
        source: include_str!("../../../examples/ping.ciac"),
        summary: "the smallest useful program: one record, one api, one \
                  pipeline, no capabilities",
        note: None,
    },
];

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

    eprintln!(
        "scaffolded the `{}` template into {}",
        tpl.name,
        dir.display()
    );
    eprintln!("next: cd {} && ciac check main.ciac", dir.display());
    Ok(ExitCode::SUCCESS)
}

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
