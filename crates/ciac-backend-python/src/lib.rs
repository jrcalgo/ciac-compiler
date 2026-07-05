//! Python (FastAPI ecosystem) code-generation backend.
//!
//! Maps the CIaC ontology onto production-standard Python components:
//!
//! | CIaC          | Python                                  |
//! |---------------|-----------------------------------------|
//! | API           | FastAPI `APIRouter`                     |
//! | Service       | plain async service class               |
//! | Worker        | NATS subscription in an asyncio task    |
//! | Database      | SQLAlchemy (async) + asyncpg            |
//! | Cache         | redis-py (asyncio)                      |
//! | Queue         | nats-py                                 |
//! | Auth (JWT)    | FastAPI dependency + PyJWT              |
//! | Logging       | structlog                               |
//! | Metrics       | prometheus-client `/metrics` endpoint   |
//!
//! The generated project is import-safe without any infrastructure
//! running: database, cache, and queue clients are created lazily, so the
//! generated smoke tests pass on a bare checkout.

use ciac_codegen::model as context;
use ciac_codegen::{Backend, BackendError, GenOptions, GeneratedProject};
use ciac_ir::{Component, NormalizedIr};
use include_dir::{include_dir, Dir};
use minijinja::context;

static TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

#[derive(Debug, Default)]
pub struct PythonBackend;

impl Backend for PythonBackend {
    fn id(&self) -> &'static str {
        "python"
    }

    fn description(&self) -> &'static str {
        "Python 3.11+ project using FastAPI, SQLAlchemy, redis-py, and nats-py"
    }

    fn supports(&self, component: &Component) -> bool {
        // Kafka has no generator yet, and neither does a v0.7 typed
        // handler signature (`extern` or inline body) — the typed HIR
        // exists (v0.7 M2) but no emitter walks it yet (M3/M4).
        !matches!(
            component,
            Component::Queue {
                engine: ciac_ir::QueueEngine::Kafka,
                ..
            } | Component::Service {
                signature: Some(_),
                ..
            }
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
                "run the API with `uv sync && uv run uvicorn app.main:app`, \
                 or `docker compose up` for the full stack"
                    .to_owned(),
            );
            let ctx = &model.services[0];
            if !ctx.workers.is_empty() || !ctx.jobs.is_empty() || !ctx.consumers.is_empty() {
                project
                    .notes
                    .push("start workers/jobs with `uv run python -m app.workers`".to_owned());
            }
        }
        Ok(project)
    }
}

/// Emits one deployable project (today's single-service layout) under
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

    project.add_file(at("pyproject.toml"), render("pyproject.toml.j2", empty())?);
    project.add_file(at("README.md"), render("README.md.j2", empty())?);
    project.add_file(at("Dockerfile"), render("Dockerfile.j2", empty())?);
    if !multi {
        project.add_file(
            at("docker-compose.yml"),
            render("docker-compose.yml.j2", empty())?,
        );
    }
    project.add_file(at("conftest.py"), render("conftest.py.j2", empty())?);
    project.add_file(
        at("app/__init__.py"),
        format!(
            "\"\"\"{} — generated by CIaC. Structure is compiler-owned;\nbusiness logic belongs in `app/services/`.\"\"\"\n",
            ctx.service_name
        ),
    );
    project.add_file(at("app/main.py"), render("main.py.j2", empty())?);
    project.add_file(at("app/config.py"), render("config.py.j2", empty())?);
    if ctx.has_auth {
        project.add_file(at("app/auth.py"), render("auth.py.j2", empty())?);
    }
    if ctx.has_db {
        project.add_file(at("app/db.py"), render("db.py.j2", empty())?);
    }
    if ctx.has_cache {
        project.add_file(at("app/cache.py"), render("cache.py.j2", empty())?);
    }
    if ctx.has_queue {
        project.add_file(at("app/queue.py"), render("queue.py.j2", empty())?);
    }
    if ctx.has_object_store {
        project.add_file(
            at("app/object_store.py"),
            render("object_store.py.j2", empty())?,
        );
    }
    if ctx.has_email {
        project.add_file(at("app/email.py"), render("email.py.j2", empty())?);
    }
    if ctx.has_search {
        project.add_file(at("app/search.py"), render("search.py.j2", empty())?);
    }
    if ctx.has_external_http {
        project.add_file(
            at("app/http_clients.py"),
            render("http_clients.py.j2", empty())?,
        );
    }
    if !ctx.call_targets.is_empty() {
        project.add_file(
            at("app/clients/__init__.py"),
            "\"\"\"Typed HTTP clients for the services this service calls.\"\"\"\n",
        );
        for target in &ctx.call_targets {
            project.add_file(
                at(&format!("app/clients/{}.py", target.module)),
                render("client.py.j2", context! { t => target })?,
            );
        }
    }
    if ctx.has_logging || ctx.has_metrics {
        project.add_file(
            at("app/observability.py"),
            render("observability.py.j2", empty())?,
        );
    }
    if !ctx.records.is_empty() {
        project.add_file(at("app/schemas.py"), render("schemas.py.j2", empty())?);
    }
    if !ctx.resources.is_empty() {
        project.add_file(at("app/models.py"), render("models.py.j2", empty())?);
    }

    if !ctx.apis.is_empty() || !ctx.channels.is_empty() || !ctx.resources.is_empty() {
        project.add_file(at("app/api/__init__.py"), "\"\"\"HTTP routers.\"\"\"\n");
    }
    for api in &ctx.apis {
        project.add_file(
            at(&format!("app/api/{}.py", api.snake)),
            render("api.py.j2", context! { api => api })?,
        );
    }
    for channel in &ctx.channels {
        project.add_file(
            at(&format!("app/api/channel_{}.py", channel.snake)),
            render("channel.py.j2", context! { channel => channel })?,
        );
    }
    for resource in &ctx.resources {
        project.add_file(
            at(&format!("app/api/{}.py", resource.snake)),
            render("resource_api.py.j2", context! { resource => resource })?,
        );
    }

    if !ctx.services.is_empty() || !ctx.resources.is_empty() {
        project.add_file(
            at("app/services/__init__.py"),
            "\"\"\"Business-logic handlers. This is where your code goes.\"\"\"\n",
        );
    }
    for service in &ctx.services {
        project.add_seeded_file(
            at(&format!("app/services/{}.py", service.module)),
            render("service.py.j2", context! { service => service })?,
        );
    }
    for resource in &ctx.resources {
        project.add_file(
            at(&format!("app/services/{}.py", resource.store_module)),
            render("resource_store.py.j2", context! { resource => resource })?,
        );
    }

    if !ctx.workers.is_empty() || !ctx.jobs.is_empty() || !ctx.consumers.is_empty() {
        project.add_file(
            at("app/workers/__init__.py"),
            "\"\"\"Queue-driven workers and scheduled jobs. Run them all with `python -m app.workers`.\"\"\"\n",
        );
        project.add_file(
            at("app/workers/__main__.py"),
            render("workers_main.py.j2", empty())?,
        );
    }
    for worker in &ctx.workers {
        project.add_file(
            at(&format!("app/workers/{}.py", worker.snake)),
            render("worker.py.j2", context! { worker => worker })?,
        );
    }
    for job in &ctx.jobs {
        project.add_file(
            at(&format!("app/workers/{}.py", job.snake)),
            render("job.py.j2", context! { job => job })?,
        );
    }
    for consumer in &ctx.consumers {
        project.add_file(
            at(&format!("app/workers/{}.py", consumer.snake)),
            render("consumer.py.j2", context! { consumer => consumer })?,
        );
    }

    project.add_file(
        at("tests/test_smoke.py"),
        render("test_smoke.py.j2", empty())?,
    );
    Ok(())
}
