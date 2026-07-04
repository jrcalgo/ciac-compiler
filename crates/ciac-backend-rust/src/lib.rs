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
        // Kafka has no generator yet; realtime waits for its v0.6 language
        // construct (`ciac check` accepts it, build gates).
        !matches!(
            component,
            Component::Queue {
                engine: ciac_ir::QueueEngine::Kafka,
                ..
            } | Component::Realtime { .. }
        )
    }

    fn generate(
        &self,
        ir: &NormalizedIr,
        opts: &GenOptions,
    ) -> Result<GeneratedProject, BackendError> {
        let model = context::build_system(ir, opts);
        let env = ciac_codegen::template::environment(TEMPLATES.files().map(|f| {
            (
                f.path().to_str().expect("template names are utf-8"),
                f.contents_utf8().expect("templates are utf-8"),
            )
        }))?;

        let mut project = GeneratedProject::new();
        for ctx in &model.services {
            let prefix = if model.multi {
                format!("{}/", ctx.dir)
            } else {
                String::new()
            };
            emit_service(&env, ctx, model.multi, &prefix, &mut project)?;
        }

        if model.multi {
            let m = minijinja::Value::from_serialize(&model);
            project.add_file(
                "docker-compose.yml",
                env.get_template("system-compose.yml.j2")?
                    .render(context! { m => m })?,
            );
            project.add_file(
                "README.md",
                env.get_template("system-README.md.j2")?
                    .render(context! { m => m })?,
            );
            project.notes.push(
                "multi-service system: each directory is a complete project; \
                 `docker compose up` runs them all together"
                    .to_owned(),
            );
        } else {
            project.notes.push(
                "run the API with `cargo run`, or `docker compose up` for the full stack"
                    .to_owned(),
            );
            let ctx = &model.services[0];
            if !ctx.workers.is_empty() || !ctx.jobs.is_empty() || !ctx.consumers.is_empty() {
                project
                    .notes
                    .push("start workers/jobs with `cargo run --bin workers`".to_owned());
            }
        }
        Ok(project)
    }
}

/// Emits one deployable crate (today's single-service layout) under
/// `prefix`. Multi-service systems get their compose file at the root
/// instead of per service.
fn emit_service(
    env: &minijinja::Environment<'_>,
    ctx: &context::Ctx,
    multi: bool,
    prefix: &str,
    project: &mut GeneratedProject,
) -> Result<(), BackendError> {
    let base = minijinja::Value::from_serialize(ctx);
    let render = |name: &str, extra: minijinja::Value| -> Result<String, BackendError> {
        Ok(env
            .get_template(name)?
            .render(context! { c => base, ..extra })?)
    };
    let empty = || context! {};
    let at = |path: &str| format!("{prefix}{path}");

    project.add_file(at("Cargo.toml"), render("Cargo.toml.j2", empty())?);
    project.add_file(at("README.md"), render("README.md.j2", empty())?);
    project.add_file(at("Dockerfile"), render("Dockerfile.j2", empty())?);
    if !multi {
        project.add_file(
            at("docker-compose.yml"),
            render("docker-compose.yml.j2", empty())?,
        );
    }
    project.add_file(at(".gitignore"), "/target\n");
    project.add_file(at("src/lib.rs"), render("lib.rs.j2", empty())?);
    project.add_file(at("src/main.rs"), render("main.rs.j2", empty())?);
    project.add_file(at("src/config.rs"), render("config.rs.j2", empty())?);
    project.add_file(at("src/state.rs"), render("state.rs.j2", empty())?);
    project.add_file(at("src/error.rs"), render("error.rs.j2", empty())?);
    project.add_file(
        at("src/observability.rs"),
        render("observability.rs.j2", empty())?,
    );
    if ctx.has_auth {
        project.add_file(at("src/auth.rs"), render("auth.rs.j2", empty())?);
    }
    if !ctx.resources.is_empty() {
        project.add_file(at("src/db.rs"), render("db.rs.j2", empty())?);
        project.add_file(at("src/models.rs"), render("models.rs.j2", empty())?);
    }
    if !ctx.records.is_empty() {
        project.add_file(at("src/schemas.rs"), render("schemas.rs.j2", empty())?);
    }
    if ctx.has_object_store {
        project.add_file(
            at("src/object_store.rs"),
            render("object_store.rs.j2", empty())?,
        );
    }
    if ctx.has_email {
        project.add_file(at("src/email.rs"), render("email.rs.j2", empty())?);
    }
    if ctx.has_search {
        project.add_file(at("src/search.rs"), render("search.rs.j2", empty())?);
    }
    if ctx.has_external_http {
        project.add_file(
            at("src/http_clients.rs"),
            render("http_clients.rs.j2", empty())?,
        );
    }
    if !ctx.call_targets.is_empty() {
        project.add_file(
            at("src/clients/mod.rs"),
            render("clients_mod.rs.j2", empty())?,
        );
        for target in &ctx.call_targets {
            project.add_file(
                at(&format!("src/clients/{}.rs", target.module)),
                render("client.rs.j2", context! { t => target })?,
            );
        }
    }

    project.add_file(
        at("src/routes/mod.rs"),
        render("routes_mod.rs.j2", empty())?,
    );
    for api in &ctx.apis {
        project.add_file(
            at(&format!("src/routes/{}.rs", api.snake)),
            render("route_api.rs.j2", context! { api => api })?,
        );
    }
    for resource in &ctx.resources {
        project.add_file(
            at(&format!("src/routes/{}.rs", resource.snake)),
            render("route_resource.rs.j2", context! { resource => resource })?,
        );
    }

    if !ctx.services.is_empty() || !ctx.resources.is_empty() {
        project.add_file(
            at("src/services/mod.rs"),
            render("services_mod.rs.j2", empty())?,
        );
    }
    for service in &ctx.services {
        project.add_seeded_file(
            at(&format!("src/services/{}.rs", service.module)),
            render("service.rs.j2", context! { service => service })?,
        );
    }
    for resource in &ctx.resources {
        project.add_file(
            at(&format!("src/services/{}.rs", resource.store_module)),
            render("resource_store.rs.j2", context! { resource => resource })?,
        );
    }

    if !ctx.workers.is_empty() || !ctx.jobs.is_empty() || !ctx.consumers.is_empty() {
        project.add_file(
            at("src/workers/mod.rs"),
            render("workers_mod.rs.j2", empty())?,
        );
        project.add_file(
            at("src/bin/workers.rs"),
            render("workers_bin.rs.j2", empty())?,
        );
    }
    for worker in &ctx.workers {
        project.add_file(
            at(&format!("src/workers/{}.rs", worker.snake)),
            render("worker.rs.j2", context! { worker => worker })?,
        );
    }
    for job in &ctx.jobs {
        project.add_file(
            at(&format!("src/workers/{}.rs", job.snake)),
            render("job.rs.j2", context! { job => job })?,
        );
    }
    for consumer in &ctx.consumers {
        project.add_file(
            at(&format!("src/workers/{}.rs", consumer.snake)),
            render("consumer.rs.j2", context! { consumer => consumer })?,
        );
    }
    Ok(())
}
