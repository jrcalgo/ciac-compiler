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

mod lower;

use ciac_codegen::model as context;
use ciac_codegen::{Backend, BackendError, GenOptions, GeneratedProject};
use ciac_ir::{Component, NormalizedIr};
use include_dir::{include_dir, Dir};
use minijinja::context;

static TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

/// This backend's two compose-file divergences (v0.9 M1): SQLAlchemy
/// wants the asyncpg driver in the URL scheme, and workers start as a
/// Python module. Everything else in the compose files is shared —
/// see `ciac_codegen::compose`.
const COMPOSE_OPTS: ciac_codegen::compose::BackendComposeOpts =
    ciac_codegen::compose::BackendComposeOpts {
        db_url_scheme: "postgresql+asyncpg",
        workers_command: r#"["python", "-m", "app.workers"]"#,
        mysql_url_scheme: "mysql+aiomysql",
        sqlite_url_prefix: "sqlite+aiosqlite:///data/",
        sqlite_url_suffix: "",
        data_mount: "/app/data",
    };

#[derive(Debug, Default)]
pub struct PythonBackend;

impl Backend for PythonBackend {
    fn id(&self) -> &'static str {
        "python"
    }

    fn description(&self) -> &'static str {
        "Python 3.11+ project using FastAPI, SQLAlchemy, redis-py, and nats-py"
    }

    fn supports(&self, _component: &Component) -> bool {
        // Everything, including Kafka since v0.11 M3 (aiokafka). The
        // Rust backend still gates Kafka and MySQL.
        true
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
            emit_service(&env, ir, ctx, model.multi, &prefix, &mut project)?;
        }

        if model.multi {
            let m = minijinja::Value::from_serialize(&model);
            project.add_file(
                "docker-compose.yml",
                ciac_codegen::compose::render_system_compose(&model, &COMPOSE_OPTS)?,
            );
            project.add_file(
                "README.md",
                env.get_template("system-README.md.j2")?
                    .render(context! { m => m })?,
            );
            project.add_file(
                "openapi.json",
                serde_json::to_string_pretty(&ciac_codegen::openapi::build_index(&model))
                    .map_err(|e| BackendError::Other(e.to_string()))?,
            );
            if model.has_tracing {
                project.add_file(
                    "otel-collector-config.yaml",
                    ciac_codegen::compose::OTEL_COLLECTOR_CONFIG,
                );
            }
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
    ir: &NormalizedIr,
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
    // Docker doesn't read .gitignore — without this, a `.venv/` left
    // behind by a native `uv sync` run against this project (as `ciac
    // verify` does before handing off to `docker compose`) becomes
    // part of the build context on every image layer transfer.
    project.add_file(at(".dockerignore"), ".venv\n__pycache__\n.pytest_cache\n");
    if !multi {
        project.add_file(
            at("docker-compose.yml"),
            ciac_codegen::compose::render_service_compose(ctx, &COMPOSE_OPTS)?,
        );
        if ctx.has_tracing {
            project.add_file(
                at("otel-collector-config.yaml"),
                ciac_codegen::compose::OTEL_COLLECTOR_CONFIG,
            );
        }
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
    project.add_file(
        at("openapi.json"),
        serde_json::to_string_pretty(&ciac_codegen::openapi::build_document(ctx))
            .map_err(|e| BackendError::Other(e.to_string()))?,
    );
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
    if ctx.has_logging || ctx.has_metrics || ctx.has_tracing {
        project.add_file(
            at("app/observability.py"),
            render("observability.py.j2", empty())?,
        );
    }
    if !ctx.records.is_empty() {
        project.add_file(at("app/schemas.py"), render("schemas.py.j2", empty())?);
    }
    if !ctx.resources.is_empty() || !ctx.tables.is_empty() {
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

    // v0.7 typed handlers (M3): the class shape (`class_name`, a
    // `session`/`cache`/extras constructor, `async def handle(..)`)
    // mirrors classic handlers so pipeline call sites need no changes.
    // Inline bodies lower straight from the HIR and are compiler-owned;
    // `extern` gets a typed stub in `app/services/` like classic
    // handlers, since it's the same "implement this yourself" contract.
    let typed_handlers: Vec<(String, &ciac_ir::HandlerBody)> = ctx
        .typed_handlers
        .iter()
        .filter_map(|id| match &ir.node(*id).component {
            Component::Service {
                name,
                signature: Some(hir),
            } => Some((name.clone(), hir)),
            _ => None,
        })
        .collect();
    let has_extern_handler = typed_handlers.iter().any(|(_, hir)| hir.body.is_none());
    let has_inline_handler = typed_handlers.iter().any(|(_, hir)| hir.body.is_some());

    if !ctx.services.is_empty() || !ctx.resources.is_empty() || has_extern_handler {
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
    if has_inline_handler {
        project.add_file(
            at("app/logic/__init__.py"),
            "\"\"\"Compiler-owned typed-handler logic, lowered from `.ciac` handler \
             bodies. Regenerated on every build; business logic lives in the \
             `.ciac` source, not here.\"\"\"\n",
        );
    }
    for (name, hir) in &typed_handlers {
        let handler = lower::render(ir, name, hir);
        let content = render(
            "logic.py.j2",
            context! { handler => minijinja::Value::from_serialize(&handler) },
        )?;
        if hir.body.is_some() {
            project.add_file(at(&format!("app/logic/{}.py", handler.module)), content);
            if let Some(test) = lower::render_test(ir, hir, &handler) {
                project.add_file(at(&format!("tests/test_logic_{}.py", handler.module)), test);
            }
        } else {
            project.add_seeded_file(at(&format!("app/services/{}.py", handler.module)), content);
        }
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
