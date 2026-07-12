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

mod lower;

use ciac_codegen::model as context;
use ciac_codegen::{Backend, BackendError, GenOptions, GeneratedProject};
use ciac_ir::{Component, NormalizedIr};
use include_dir::{include_dir, Dir};
use minijinja::context;

static TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

/// This backend's two compose-file divergences (v0.9 M1): SQLx wants
/// the plain `postgres` URL scheme, and workers start as the binary's
/// `workers` subcommand. Everything else in the compose files is
/// shared — see `ciac_codegen::compose`.
const COMPOSE_OPTS: ciac_codegen::compose::BackendComposeOpts =
    ciac_codegen::compose::BackendComposeOpts {
        db_url_scheme: "postgres",
        workers_command: r#"["workers"]"#,
        mysql_url_scheme: "mysql",
        sqlite_url_prefix: "sqlite://data/",
        sqlite_url_suffix: "?mode=rwc",
        data_mount: "/data",
    };

#[derive(Debug, Default)]
pub struct RustBackend;

impl Backend for RustBackend {
    fn id(&self) -> &'static str {
        "rust"
    }

    fn description(&self) -> &'static str {
        "Rust project using Axum, SQLx, redis, and async-nats/rdkafka"
    }

    fn supports(&self, component: &Component) -> bool {
        // Full provider parity since v0.13: MySQL graduated in M1
        // (per-engine sqlx pools + placeholder styles), Kafka in M2
        // (rdkafka vendored; same topics/groups as the Python
        // backend). Typed handler signatures graduated in v0.7 M4.
        let _ = component;
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

    // v0.7 typed handlers (M4): the class shape (`class_name`, a
    // `db`/`cache`/extras-borrowing `<'a>` constructor,
    // `async fn handle(..) -> anyhow::Result<T>`) mirrors classic
    // handlers so pipeline call sites need no changes beyond importing
    // from the right package (see `HandlerRef::handler_package`).
    // Inline bodies lower straight from the HIR and are compiler-owned;
    // `extern` gets a stub in `src/services/`, the same "implement this
    // yourself" contract classic handlers already have.
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
    let needs_uuid_crate = typed_handlers
        .iter()
        .any(|(_, hir)| lower::scan(ir, hir).uuid);
    let needs_chrono_crate = typed_handlers
        .iter()
        .any(|(_, hir)| lower::scan(ir, hir).datetime);
    let needs_thiserror = ctx.records.iter().any(|r| r.is_error);

    project.add_file(
        at("Cargo.toml"),
        render(
            "Cargo.toml.j2",
            context! { needs_uuid_crate, needs_chrono_crate, needs_thiserror },
        )?,
    );
    project.add_file(at("README.md"), render("README.md.j2", empty())?);
    project.add_file(at("Dockerfile"), render("Dockerfile.j2", empty())?);
    if !multi {
        project.add_file(
            at("docker-compose.yml"),
            ciac_codegen::compose::render_service_compose(ctx, &COMPOSE_OPTS)?,
        );
    }
    project.add_file(at(".gitignore"), "/target\n");
    // Docker doesn't read .gitignore — without this, a `target/` left
    // behind by a native `cargo build`/`cargo test` run against this
    // project (as `ciac verify` does before handing off to `docker
    // compose`) becomes part of the build context, multiplying a
    // multi-hundred-MB debug build into every image layer transfer.
    project.add_file(at(".dockerignore"), "/target\n");
    project.add_file(
        at("src/lib.rs"),
        render(
            "lib.rs.j2",
            context! { has_inline_handler, has_extern_handler },
        )?,
    );
    project.add_file(at("src/main.rs"), render("main.rs.j2", empty())?);
    project.add_file(at("src/config.rs"), render("config.rs.j2", empty())?);
    project.add_file(at("src/state.rs"), render("state.rs.j2", empty())?);
    project.add_file(at("src/error.rs"), render("error.rs.j2", empty())?);
    if ctx.has_queue {
        project.add_file(at("src/queue.rs"), render("queue.rs.j2", empty())?);
    }
    project.add_file(
        at("src/observability.rs"),
        render("observability.rs.j2", empty())?,
    );
    if ctx.has_auth {
        project.add_file(at("src/auth.rs"), render("auth.rs.j2", empty())?);
    }
    if !ctx.resources.is_empty() || !ctx.tables.is_empty() {
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
    for channel in &ctx.channels {
        project.add_file(
            at(&format!("src/routes/channel_{}.rs", channel.snake)),
            render("channel.rs.j2", context! { channel => channel })?,
        );
    }
    for resource in &ctx.resources {
        project.add_file(
            at(&format!("src/routes/{}.rs", resource.snake)),
            render("route_resource.rs.j2", context! { resource => resource })?,
        );
    }

    // Render each typed handler's file once, splitting into extern
    // (seeded stub under `src/services/`) vs. inline (compiler-owned,
    // `src/logic/`) so both `mod.rs` files can list exactly the right
    // modules — Rust, unlike Python, needs every submodule declared
    // explicitly.
    let mut typed_handler_files: Vec<(String, bool, String)> = Vec::new();
    for (name, hir) in &typed_handlers {
        let handler = lower::render(ir, name, hir);
        let content = render(
            "logic.rs.j2",
            context! { handler => minijinja::Value::from_serialize(&handler) },
        )?;
        typed_handler_files.push((handler.module.clone(), hir.body.is_none(), content));
    }
    let extern_modules: Vec<&str> = typed_handler_files
        .iter()
        .filter(|(_, is_extern, _)| *is_extern)
        .map(|(m, _, _)| m.as_str())
        .collect();
    let inline_modules: Vec<&str> = typed_handler_files
        .iter()
        .filter(|(_, is_extern, _)| !*is_extern)
        .map(|(m, _, _)| m.as_str())
        .collect();

    if !ctx.services.is_empty() || !ctx.resources.is_empty() || has_extern_handler {
        project.add_file(
            at("src/services/mod.rs"),
            render(
                "services_mod.rs.j2",
                context! { extern_handler_modules => extern_modules },
            )?,
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
    if has_inline_handler {
        project.add_file(
            at("src/logic/mod.rs"),
            render(
                "logic_mod.rs.j2",
                context! { inline_handler_modules => inline_modules },
            )?,
        );
    }
    for (module, is_extern, content) in typed_handler_files {
        if is_extern {
            project.add_seeded_file(at(&format!("src/services/{module}.rs")), content);
        } else {
            project.add_file(at(&format!("src/logic/{module}.rs")), content);
        }
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
