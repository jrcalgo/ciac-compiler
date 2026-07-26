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

mod filters;
mod lower;

use ciac_codegen::model as context;
use ciac_codegen::{
    Backend, BackendError, DevCommands, GenOptions, GeneratedProject, RestartStyle, SimSupport,
    TargetInfo, ValidateStep,
};
use ciac_ir::{Component, NodeKind, NormalizedIr};
use include_dir::{include_dir, Dir};
use minijinja::context;

static TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

// v0.17 M11: unlike Python (which cannot depend on `ciac-sim` directly
// and must restate its primitives narrowly, see `sim/pyrunner/world.py`),
// generated Rust code is Rust -- so it vendors `ciac-sim`'s own source
// for the target-neutral pieces that have no dependency on `ciac-ir`
// (confirmed: `cron.rs`/`failure.rs`/`scenario.rs`/`clock.rs` import
// only `serde`/`chrono`/std; `world.rs` only adds `anyhow`), byte-for-
// byte, as ordinary sibling modules in the generated crate
// (`crate::failure`, `crate::cron`, ...) rather than as a separate path
// dependency -- no extra Cargo.toml, no dependency-resolution story for
// a crate that isn't published. `plan.rs`/`replay.rs` (the two modules
// that do need `ciac-ir`) are deliberately not vendored: a generated
// project builds its `SimWorld` directly (no `SimPlan` JSON to load),
// and Rust replay support is real, disclosed future work. 27UpdatePlan.md
// M2's own schema-aware `world.rs` deepening is written to this same
// constraint -- it defines its own self-contained `WorldTable`/
// `WorldReference` schema-description types rather than importing
// `plan::SimTable`, precisely so it stays vendorable without dragging
// `ciac-ir` in; `clock.rs` newly joins the vendored set here since
// `world.rs` now wires `VirtualClock`/`Entropy` through directly.
const VENDORED_SIM_CLOCK: &str = include_str!("../../ciac-sim/src/clock.rs");
const VENDORED_SIM_CRON: &str = include_str!("../../ciac-sim/src/cron.rs");
const VENDORED_SIM_FAILURE: &str = include_str!("../../ciac-sim/src/failure.rs");
const VENDORED_SIM_SCENARIO: &str = include_str!("../../ciac-sim/src/scenario.rs");
const VENDORED_SIM_WORLD: &str = include_str!("../../ciac-sim/src/world.rs");

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

/// The literal CI test-step YAML for this target (v0.22 M1, formerly
/// `ciac-codegen::ci::RUST_TEST_STEPS`).
const CI_TEST_STEPS: &str = "      - uses: dtolnay/rust-toolchain@stable\n      - run: cargo check\n        env:\n          RUSTFLAGS: \"-D warnings\"\n      - run: cargo test -q --lib\n";

/// This target's whole CLI/CI/compose/dev-loop/sim integration surface
/// (v0.22 M1 — `22UpdatePlan.md` Pillar 1). `validate` mirrors
/// `validate_rust_project`'s two steps exactly (`cargo check -D
/// warnings`, then `cargo test --lib --tests` so the generated
/// no-live-infra scope-enforcement suite runs as part of `ciac
/// verify`/`build`, v0.17 M11).
static TARGET_INFO: TargetInfo = TargetInfo {
    project_marker: "Cargo.toml",
    migrations_dir: "migrations",
    migration_filename: |seq, _slug| format!("{seq:04}_migration.sql"),
    validate: &[
        ValidateStep {
            program: "cargo",
            args: &["check"],
            env: &[("RUSTFLAGS", "-D warnings")],
            purpose: "type-checks (deny warnings)",
        },
        ValidateStep {
            program: "cargo",
            args: &["test", "-q", "--lib", "--tests"],
            env: &[],
            purpose: "unit and generated tests pass",
        },
    ],
    ci_test_steps: CI_TEST_STEPS,
    compose: COMPOSE_OPTS,
    dev: DevCommands {
        rebuild: &[],
        restart: RestartStyle::Restart,
    },
    source_extension: "rs",
    sim: SimSupport::Narrow {
        unsupported: unsupported_sim_capabilities,
    },
    // 27UpdatePlan.md M1: the generated runner is a plain scenario
    // interpreter with no plan/source-hash arguments and no replay
    // tape — decoupled from `sim` depth so a future `Full` flip
    // doesn't silently imply replay support this runner never grew.
    sim_replay: false,
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

    fn target_info(&self) -> &'static TargetInfo {
        &TARGET_INFO
    }

    fn generate(
        &self,
        ir: &NormalizedIr,
        opts: &GenOptions,
    ) -> Result<GeneratedProject, BackendError> {
        let model = context::build_system(ir, opts);
        let mut env = ciac_codegen::template::environment(TEMPLATES.files().map(|f| {
            (
                f.path().to_str().expect("template names are utf-8"),
                f.contents_utf8().expect("templates are utf-8"),
            )
        }))?;
        env.add_filter("rust_type", filters::rust_type);
        env.add_filter("db_rust_type", filters::db_rust_type);

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
            if model.has_users {
                for (path, content) in ciac_codegen::users::build(&model) {
                    project.add_file(path, content);
                }
            }
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

/// Human-readable, closed list of reasons `ciac sim --target rust`
/// (v0.17 M11) cannot yet simulate `ir`, empty when it can. `SimWorld`
/// (`ciac-sim/src/world.rs`) only fakes `db.insert` and broker
/// publish/consume; every other verb a typed handler calls falls
/// straight through the world-guard to real infrastructure, either
/// panicking against an unreachable service or (for verbs with no
/// guard at all, like `db.get`) reading the real, empty pool instead of
/// the seeded/inserted simulation state. Declared-but-unused capability
/// instances are not flagged here -- only verbs a handler body actually
/// calls, since an unused `cache Redis` instance never touches
/// anything and lazy construction (v0.17 M11) means it's harmless.
pub fn unsupported_sim_capabilities(ir: &NormalizedIr) -> Vec<String> {
    let mut reasons = Vec::new();
    if ir.nodes_of_kind(NodeKind::Auth).next().is_some() {
        reasons.push(
            "declares `auth` (OAuth2/JWT): validating a real signed token needs real \
             cryptography against a real issuer, which this milestone's simulation world does \
             not fake"
                .to_owned(),
        );
    }
    let mut unguarded_verbs: Vec<&'static str> = Vec::new();
    for node in ir.nodes() {
        if let Component::Service {
            signature: Some(hir),
            ..
        } = &node.component
        {
            for verb in lower::scan(ir, hir).unguarded_verbs {
                if !unguarded_verbs.contains(&verb) {
                    unguarded_verbs.push(verb);
                }
            }
        }
    }
    if !unguarded_verbs.is_empty() {
        unguarded_verbs.sort_unstable();
        reasons.push(format!(
            "calls verb(s) the simulation world does not fake: {}",
            unguarded_verbs.join(", ")
        ));
    }
    reasons
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
    project.add_file(
        at("openapi.json"),
        serde_json::to_string_pretty(&ciac_codegen::openapi::build_document(ctx))
            .map_err(|e| BackendError::Other(e.to_string()))?,
    );
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
        if ctx.has_users {
            project.add_file(
                at("keycloak-realm.json"),
                ciac_codegen::users::realm_json(&ctx.scopes),
            );
            project.add_file(
                at("scripts/token.sh"),
                ciac_codegen::users::token_script(&ctx.scopes),
            );
        }
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
    // v0.17 M11: the vendored simulation world -- only for programs with
    // something it can actually fake (`db.insert`, broker `publish`);
    // see the `VENDORED_SIM_*` doc comment above for why this is a
    // verbatim copy of `ciac-sim`'s own source, not a restatement.
    if ctx.has_db || ctx.has_queue {
        project.add_file(at("src/clock.rs"), VENDORED_SIM_CLOCK);
        project.add_file(at("src/failure.rs"), VENDORED_SIM_FAILURE);
        project.add_file(at("src/scenario.rs"), VENDORED_SIM_SCENARIO);
        project.add_file(at("src/world.rs"), VENDORED_SIM_WORLD);
        if !ctx.jobs.is_empty() {
            project.add_file(at("src/cron.rs"), VENDORED_SIM_CRON);
        }
        // v0.17 M11: `cargo run --bin sim_runner -- <scenario.json>` --
        // see the template's own doc comment for exactly what this does
        // and does not cover. `has_drain_workers` tells the template
        // whether any worker match arm exists at all, so it can name the
        // drained-payload binding `_raw` instead of `raw` when none do
        // (an empty match has nothing to deserialize `raw` into).
        let has_drain_workers = ctx.workers.iter().any(|w| !w.steps.is_empty());
        project.add_file(
            at("src/bin/sim_runner.rs"),
            render("sim_runner.rs.j2", context! { has_drain_workers })?,
        );
    }
    project.add_file(
        at("src/observability.rs"),
        render("observability.rs.j2", empty())?,
    );
    if ctx.has_auth {
        project.add_file(at("src/auth.rs"), render("auth.rs.j2", empty())?);
    }
    // v0.14 M6: scope-enforcement behavioral test. v0.17 M11 made the
    // broker client lazy (matching the db pools' `connect_lazy`), so
    // the `!has_queue` half of this gate is gone -- a queue-bearing JWT
    // service's `AppState::new` no longer touches the network either.
    // `26UpdatePlan.md` M5: this file's own `no_token`/`malformed_token`
    // cases are scheme-agnostic (gated inside the template on
    // `has_auth_step`/`has_auth`, not on scheme), so the file now
    // renders for both schemes -- an oauth2 project without it silently
    // lost that coverage, a gap M4 introduced and M5 closed. The
    // JWT-only bearer-minting helpers and their wrong_scope/
    // correct_scope/expired_token blocks stay gated on
    // `c.auth_scheme == "jwt"` inside the template; oauth2 gets the
    // equivalent via the real-RS256 rig below.
    if !ctx.scopes.is_empty() {
        project.add_file(
            at("tests/scope_tests.rs"),
            render("scope_tests.rs.j2", empty())?,
        );
    }
    // The no-infra OAuth2 rig (`26UpdatePlan.md` M4): real RS256
    // signing against an in-process JWKS stub, gated the same way the
    // JWT scope suite is gated on `c.scopes` above.
    if ctx.auth_scheme == "oauth2" && !ctx.scopes.is_empty() {
        project.add_file(
            at("tests/oauth_rig_tests.rs"),
            render("oauth_rig_tests.rs.j2", empty())?,
        );
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
