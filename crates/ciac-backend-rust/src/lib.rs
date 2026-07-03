//! Rust (Axum ecosystem) code-generation backend.
//!
//! Maps the CIaC ontology onto production-standard Rust components:
//!
//! | CIaC          | Rust                                     |
//! |---------------|------------------------------------------|
//! | API           | Axum router                              |
//! | Service       | plain async service struct               |
//! | Worker        | async-nats subscription in a Tokio task  |
//! | Database      | SQLx (Postgres)                          |
//! | Cache         | redis crate (async)                      |
//! | Queue         | async-nats                               |
//! | Auth (JWT)    | extractor + jsonwebtoken                 |
//! | Logging       | tracing + tracing-subscriber             |
//! | Metrics       | metrics-exporter-prometheus              |
//!
//! The generated project mirrors the structure of the Python backend's
//! output (same shared [`ciac_codegen::model`]), so targets stay
//! comparable: routers per api, service handler stubs you own, queue-group
//! workers, and a compose file for the declared infrastructure.

use ciac_codegen::model as context;
use ciac_codegen::{Backend, BackendError, GenOptions, GeneratedProject};
use ciac_ir::{Component, NormalizedIr};
use include_dir::{include_dir, Dir};
use minijinja::context;

static TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

#[derive(Debug, Default)]
pub struct RustBackend;

impl Backend for RustBackend {
    fn id(&self) -> &'static str {
        "rust"
    }

    fn description(&self) -> &'static str {
        "Rust project using Axum, SQLx, redis, and async-nats"
    }

    fn supports(&self, component: &Component) -> bool {
        !matches!(
            component,
            Component::Queue {
                engine: ciac_ir::QueueEngine::Kafka
            }
        )
    }

    fn generate(
        &self,
        ir: &NormalizedIr,
        opts: &GenOptions,
    ) -> Result<GeneratedProject, BackendError> {
        let ctx = context::build(ir, opts);
        let env = ciac_codegen::template::environment(TEMPLATES.files().map(|f| {
            (
                f.path().to_str().expect("template names are utf-8"),
                f.contents_utf8().expect("templates are utf-8"),
            )
        }))?;
        let base = minijinja::Value::from_serialize(&ctx);
        let render = |name: &str, extra: minijinja::Value| -> Result<String, BackendError> {
            Ok(env
                .get_template(name)?
                .render(context! { c => base, ..extra })?)
        };

        let mut project = GeneratedProject::new();
        let empty = || context! {};

        project.add_file("Cargo.toml", render("Cargo.toml.j2", empty())?);
        project.add_file("README.md", render("README.md.j2", empty())?);
        project.add_file("Dockerfile", render("Dockerfile.j2", empty())?);
        project.add_file(
            "docker-compose.yml",
            render("docker-compose.yml.j2", empty())?,
        );
        project.add_file(".gitignore", "/target\n");
        project.add_file("src/lib.rs", render("lib.rs.j2", empty())?);
        project.add_file("src/main.rs", render("main.rs.j2", empty())?);
        project.add_file("src/config.rs", render("config.rs.j2", empty())?);
        project.add_file("src/state.rs", render("state.rs.j2", empty())?);
        project.add_file("src/error.rs", render("error.rs.j2", empty())?);
        project.add_file(
            "src/observability.rs",
            render("observability.rs.j2", empty())?,
        );
        if ctx.has_auth {
            project.add_file("src/auth.rs", render("auth.rs.j2", empty())?);
        }
        if !ctx.resources.is_empty() {
            project.add_file("src/db.rs", render("db.rs.j2", empty())?);
            project.add_file("src/models.rs", render("models.rs.j2", empty())?);
        }
        if !ctx.records.is_empty() {
            project.add_file("src/schemas.rs", render("schemas.rs.j2", empty())?);
        }

        project.add_file("src/routes/mod.rs", render("routes_mod.rs.j2", empty())?);
        for api in &ctx.apis {
            project.add_file(
                format!("src/routes/{}.rs", api.snake),
                render("route_api.rs.j2", context! { api => api })?,
            );
        }
        for resource in &ctx.resources {
            project.add_file(
                format!("src/routes/{}.rs", resource.snake),
                render("route_resource.rs.j2", context! { resource => resource })?,
            );
        }

        if !ctx.services.is_empty() || !ctx.resources.is_empty() {
            project.add_file(
                "src/services/mod.rs",
                render("services_mod.rs.j2", empty())?,
            );
        }
        for service in &ctx.services {
            project.add_file(
                format!("src/services/{}.rs", service.module),
                render("service.rs.j2", context! { service => service })?,
            );
        }
        for resource in &ctx.resources {
            project.add_file(
                format!("src/services/{}.rs", resource.store_module),
                render("resource_store.rs.j2", context! { resource => resource })?,
            );
        }

        if !ctx.workers.is_empty() || !ctx.consumers.is_empty() {
            project.add_file("src/workers/mod.rs", render("workers_mod.rs.j2", empty())?);
            project.add_file("src/bin/workers.rs", render("workers_bin.rs.j2", empty())?);
        }
        for worker in &ctx.workers {
            project.add_file(
                format!("src/workers/{}.rs", worker.snake),
                render("worker.rs.j2", context! { worker => worker })?,
            );
        }
        for consumer in &ctx.consumers {
            project.add_file(
                format!("src/workers/{}.rs", consumer.snake),
                render("consumer.rs.j2", context! { consumer => consumer })?,
            );
        }

        project.notes.push(
            "run the API with `cargo run`, or `docker compose up` for the full stack".to_owned(),
        );
        if !ctx.workers.is_empty() || !ctx.consumers.is_empty() {
            project
                .notes
                .push("start workers with `cargo run --bin workers`".to_owned());
        }
        Ok(project)
    }
}
