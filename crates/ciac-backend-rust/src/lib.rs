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
use ciac_ir::{Component, NormalizedIr};
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
// (`crate::failure`, `crate::cron`, ...) for a single-service project --
// no extra Cargo.toml, no dependency-resolution story for a crate that
// isn't published. `plan.rs`/`replay.rs` (the two modules that do need
// `ciac-ir`) are deliberately not vendored: a generated project builds
// its `SimWorld` directly (no `SimPlan` JSON to load), and Rust replay
// support is real, disclosed future work. 27UpdatePlan.md M2's own
// schema-aware `world.rs` deepening is written to this same constraint
// -- it defines its own self-contained `WorldTable`/`WorldReference`
// schema-description types rather than importing `plan::SimTable`,
// precisely so it stays vendorable without dragging `ciac-ir` in;
// `clock.rs` newly joins the vendored set here since `world.rs` now
// wires `VirtualClock`/`Entropy` through directly.
//
// 28UpdatePlan.md M6b: a *multi*-service system is the one case where
// "no separate path dependency" stops being free -- N services each
// vendoring their own private copy gives N nominally distinct `SimWorld`
// types (same source text, different crates), and a system-runner crate
// depending on every service as a library needs exactly one type they
// all share. `emit_service` skips these files entirely when `multi`;
// `RustBackend::generate` emits them once instead, as the `sim-shared`
// crate every service (and, from M6c, the system-runner) depends on by
// path -- single-service projects are completely unaffected.
//
// The five files below are read from `vendor/ciac-sim/`, a physical
// copy checked into this crate's own directory, not from `ciac-sim`'s
// own `src/` directly -- found live, the hard way, via a real `cargo
// publish` failure: `cargo package`/`publish` only bundles files inside
// a crate's own directory, so a `../../ciac-sim/src/...` `include_str!`
// (reaching into a sibling crate) doesn't exist in the package tarball
// and the verify-build fails with "No such file or directory". This
// mirrors `ciac-backend-java`'s own `vendor/` directory (its jar and
// `mvnw` scripts, one level up, never crossing the crate boundary) --
// the same fix, for source text instead of a jar. Unlike that jar,
// `ciac-sim`'s source is actively developed, so a copy here can drift:
// run `scripts/sync-vendored-sim.sh` after any change to `ciac-sim/src/
// {clock,cron,failure,scenario,world}.rs`, and see this module's own
// `vendored_sim_matches_source` test, which fails loudly in a normal
// workspace build (never in a build from a published crate, where
// `ciac-sim/src` isn't reachable at all) if the two fall out of sync.
const VENDORED_SIM_CLOCK: &str = include_str!("../vendor/ciac-sim/clock.rs");
const VENDORED_SIM_CRON: &str = include_str!("../vendor/ciac-sim/cron.rs");
const VENDORED_SIM_FAILURE: &str = include_str!("../vendor/ciac-sim/failure.rs");
const VENDORED_SIM_SCENARIO: &str = include_str!("../vendor/ciac-sim/scenario.rs");
const VENDORED_SIM_WORLD: &str = include_str!("../vendor/ciac-sim/world.rs");

const SIM_SHARED_CARGO_TOML: &str = r#"[package]
name = "sim-shared"
version = "0.1.0"
edition = "2021"
description = "Generated by CIaC: vendored simulation modules shared by every service in this system"

[dependencies]
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
chrono = { version = "0.4", features = ["serde"] }
"#;

const SIM_SHARED_LIB_RS: &str = r#"//! Vendored simulation modules, shared by every service crate in this
//! system (28UpdatePlan.md M6b) so `SimWorld` is one nominal type they
//! all agree on, not one private copy per service. Generated by CIaC.

pub mod clock;
pub mod cron;
pub mod failure;
pub mod scenario;
pub mod world;
"#;

/// The system-runner crate's own `Cargo.toml` (28UpdatePlan.md M6c):
/// path dependencies on `sim-shared` and every service crate in this
/// system, by their real package names -- generated as a plain `String`
/// rather than a `.j2` template since the dependency list itself is the
/// only per-system variable part and a `format!` is simpler than a loop
/// construct for one line per service.
fn system_runner_cargo_toml(model: &context::SystemModel) -> String {
    let mut deps = String::from("sim-shared = { path = \"../sim-shared\" }\n");
    for ctx in &model.services {
        deps.push_str(&format!(
            "{} = {{ path = \"../{}\" }}\n",
            ctx.package, ctx.dir
        ));
    }
    format!(
        r#"[package]
name = "system-runner"
version = "0.1.0"
edition = "2021"
description = "Generated by CIaC: drives ciac_sim scenarios across every service in this system through one shared world"

[dependencies]
anyhow = "1"
axum = "0.8"
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
tokio = {{ version = "1", features = ["full"] }}
tracing = "0.1"
chrono = {{ version = "0.4", features = ["serde"] }}
base64 = "0.22"
tower = {{ version = "0.5", features = ["util"] }}
{deps}"#
    )
}

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
        static ENV: std::sync::OnceLock<minijinja::Environment<'static>> =
            std::sync::OnceLock::new();
        let env = ciac_codegen::template::cached_environment(
            &ENV,
            TEMPLATES.files().map(|f| {
                (
                    f.path().to_str().expect("template names are utf-8"),
                    f.contents_utf8().expect("templates are utf-8"),
                )
            }),
            |env| {
                env.add_filter("rust_type", filters::rust_type);
                env.add_filter("db_rust_type", filters::db_rust_type);
            },
        );

        let mut project = GeneratedProject::new();
        for ctx in &model.services {
            let prefix = if model.multi {
                format!("{}/", ctx.dir)
            } else {
                String::new()
            };
            emit_service(env, ir, ctx, model.multi, &prefix, &mut project)?;
        }

        if model.multi {
            // 28UpdatePlan.md M6b: one `sim-shared` crate per system,
            // depended on by path from every service that would
            // otherwise vendor its own private copy of the simulation
            // modules -- see `emit_service`'s own doc comment on the
            // `multi` branch of that condition for why this matters
            // (N nominally distinct `SimWorld` types vs one shared type
            // the eventual system-runner crate can hold). Only emitted
            // when at least one service actually needs it; a system
            // with no db/queue/cache/etc. and no call edges anywhere
            // has nothing that would ever reference it.
            if model.services.iter().any(|ctx| {
                ctx.has_db
                    || ctx.has_queue
                    || ctx.has_cache
                    || ctx.has_object_store
                    || ctx.has_email
                    || ctx.has_search
                    || ctx.has_external_http
                    || ctx.has_auth
                    || !ctx.call_targets.is_empty()
            }) {
                project.add_file("sim-shared/Cargo.toml", SIM_SHARED_CARGO_TOML);
                project.add_file("sim-shared/src/lib.rs", SIM_SHARED_LIB_RS);
                project.add_file("sim-shared/src/clock.rs", VENDORED_SIM_CLOCK);
                project.add_file("sim-shared/src/failure.rs", VENDORED_SIM_FAILURE);
                project.add_file("sim-shared/src/scenario.rs", VENDORED_SIM_SCENARIO);
                project.add_file("sim-shared/src/world.rs", VENDORED_SIM_WORLD);
                project.add_file("sim-shared/src/cron.rs", VENDORED_SIM_CRON);

                // 28UpdatePlan.md M6c: the system-runner crate --
                // `sim_drive_rust`'s multi-service counterpart to
                // driving a single service's own `src/bin/sim_runner.rs`
                // (see `system_sim_runner.rs.j2`'s own doc comment for
                // the full architecture). Gated on the same condition as
                // `sim-shared` itself since it depends on that crate
                // unconditionally and has nothing to drive without it.
                project.add_file("system-runner/Cargo.toml", system_runner_cargo_toml(&model));
                let has_drain_workers = model
                    .services
                    .iter()
                    .any(|ctx| ctx.workers.iter().any(|w| !w.steps.is_empty()));
                let services = minijinja::Value::from_serialize(&model.services);
                let sim_world_tables = sim_world_tables_multi(ir);
                project.add_file(
                    "system-runner/src/main.rs",
                    env.get_template("system_sim_runner.rs.j2")?
                        .render(context! { services, sim_world_tables, has_drain_workers })?,
                );
            }
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
/// cannot yet simulate `ir`, empty when it can. As of 27UpdatePlan.md
/// M4, `SimWorld` (`ciac-sim/src/world.rs`) fakes every verb
/// [`lower::scan`]'s shared `Needs::unguarded_verbs` tracks (db/cache/
/// object_store/email/search/http) and `auth` (claims-lookup, not real
/// JWT/JWKS crypto, matching Python's `FakeAuth`), and every *typed-
/// handler* call site world-guards accordingly -- so unlike TypeScript/
/// Go/Java (still gated on that shared list and a hard `NodeKind::Auth`
/// refusal, pending their own M6-M8 restatements), Rust's own version
/// of this function no longer refuses anything -- it always returns
/// empty.
///
/// `crud <Name>: <Record>` resources (`resource_store.rs.j2`) never
/// read `self.world` at all -- a real gap, investigated this
/// milestone (`sim-broker-slice.ciac`'s `crud Widget: Widget`) -- but
/// it is not a *reachable* one: `sim_runner.rs`'s `request()` step
/// dispatches only against `c.apis`, built from `NodeKind::Api` nodes
/// with an attached `Pipeline` (`ciac-codegen/src/model.rs`'s `apis`
/// builder), which a `crud` resource's synthesized api node never has
/// -- confirmed by inspecting a generated `sim_runner.rs`'s own match
/// arms, which list every typed/classic-pipeline api but never a crud
/// route. No scenario step can address one, so its missing guard can
/// never be exercised; disclosed here rather than modeled as a
/// refusal reason, so a future capability that *does* make crud
/// routes reachable (e.g. scenario-level crud invocation) has this
/// comment as the pointer to what still needs guarding first.
pub fn unsupported_sim_capabilities(_ir: &NormalizedIr) -> Vec<String> {
    Vec::new()
}

/// Template-facing counterpart of `ciac-sim`'s `WorldReference` --
/// `sim_runner.rs.j2` renders each of these as a
/// `crate::world::WorldReference` struct literal (27UpdatePlan.md M4).
/// `on_delete` is spelled `"Cascade"`/`"Restrict"` so the template can
/// splice it straight after `crate::world::WorldRefAction::` without a
/// filter.
#[derive(serde::Serialize)]
struct SimWorldReferenceCtx {
    field_name: String,
    target_table: Option<String>,
    on_delete: &'static str,
    unique: bool,
}

/// Template-facing counterpart of `ciac-sim`'s `WorldTable`.
#[derive(serde::Serialize)]
struct SimWorldTableCtx {
    name: String,
    references: Vec<SimWorldReferenceCtx>,
}

/// Builds the schema `sim_runner.rs.j2` passes to
/// `SimWorld::with_schema` (27UpdatePlan.md M4) -- without it, `SimWorld`
/// falls back to `with_schema`'s empty-schema equivalent (`SimWorld::
/// new`) and every reference/unique/cascade check silently becomes a
/// no-op, exactly the gap that let `db_delete_checked` skip cascading
/// `line_items` away with its `orders` row until this milestone's live
/// corpus run against `domain-orders.ciac` caught it. Reuses
/// `ciac_codegen::migrations::snapshot_schema` -- the same reference/
/// unique-column facts the migration DDL itself is built from, so this
/// can never drift from what the real schema actually enforces.
fn sim_world_tables(ir: &NormalizedIr) -> Vec<SimWorldTableCtx> {
    ciac_codegen::migrations::snapshot_schema(ir)
        .into_iter()
        .map(|(name, schema)| {
            let unique_columns = schema.unique_columns;
            let references = schema
                .foreign_keys
                .into_iter()
                .map(|fk| SimWorldReferenceCtx {
                    unique: unique_columns.contains(&fk.column),
                    field_name: fk.column,
                    target_table: Some(fk.target_table),
                    on_delete: if fk.on_delete == "CASCADE" {
                        "Cascade"
                    } else {
                        "Restrict"
                    },
                })
                .collect();
            SimWorldTableCtx { name, references }
        })
        .collect()
}

/// Multi-service counterpart of [`sim_world_tables`]: the system-runner's
/// one shared [`ciac_sim::world::SimWorld`] (28UpdatePlan.md M6c) needs
/// every table name -- and every foreign key's `target_table` -- spelled
/// the same namespaced way `lower.rs`'s `world_table_key` composes them
/// at typed-handler lowering time (`"{service}::{table}"`, via
/// `SimWorld::namespaced_table_key`), or a reference/uniqueness check
/// would silently look up a table the world never registered. Builds a
/// `physical table name -> owning service` map from `ir.tables()`
/// directly (heck's `to_snake_case` mirrors `ciac_codegen::migrations`'s
/// private `physical_table_name` exactly -- that helper isn't `pub`, and
/// duplicating the one-line transform here is simpler than changing its
/// visibility for a single caller), then remaps `snapshot_schema`'s own
/// output onto it. A compiler-owned link table (`orders__line_items`)
/// carries no `Table` node of its own; its prefix (`orders`) is always
/// its source table's physical name, so the same map resolves it too.
fn sim_world_tables_multi(ir: &NormalizedIr) -> Vec<SimWorldTableCtx> {
    use heck::ToSnakeCase;

    let mut owner_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (_, table) in ir.tables() {
        if let Some(sid) = table.service {
            owner_of.insert(table.name.to_snake_case(), ir.service(sid).name.clone());
        }
    }
    let namespace = |physical: &str| -> String {
        // A link table's own prefix (before `__`) is its source table's
        // physical name; anything else looks itself up directly. Either
        // way, a table this system's sema never assigned an owning
        // service to (should not happen for a real compiled program --
        // every `table`/`crud` declaration lives in some `service`
        // block) degrades to the bare name rather than panicking.
        let key = physical
            .split_once("__")
            .map_or(physical, |(prefix, _)| prefix);
        match owner_of.get(key) {
            Some(service) => format!("{service}::{physical}"),
            None => physical.to_owned(),
        }
    };

    ciac_codegen::migrations::snapshot_schema(ir)
        .into_iter()
        .map(|(name, schema)| {
            let unique_columns = schema.unique_columns;
            let references = schema
                .foreign_keys
                .into_iter()
                .map(|fk| SimWorldReferenceCtx {
                    unique: unique_columns.contains(&fk.column),
                    field_name: fk.column,
                    target_table: Some(namespace(&fk.target_table)),
                    on_delete: if fk.on_delete == "CASCADE" {
                        "Cascade"
                    } else {
                        "Restrict"
                    },
                })
                .collect();
            SimWorldTableCtx {
                name: namespace(&name),
                references,
            }
        })
        .collect()
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
            context! { needs_uuid_crate, needs_chrono_crate, needs_thiserror, multi },
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
            context! { has_inline_handler, has_extern_handler, multi },
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
    if ctx.has_db
        || ctx.has_queue
        || ctx.has_cache
        || ctx.has_object_store
        || ctx.has_email
        || ctx.has_search
        || ctx.has_external_http
        || ctx.has_auth
        || !ctx.call_targets.is_empty()
    {
        // 28UpdatePlan.md M6b: a multi-service system's own services
        // depend on the shared `sim-shared` crate (emitted once per
        // system by `RustBackend::generate`, below) instead of each
        // vendoring a private copy -- N private copies of the identical
        // source text are still N *nominally distinct* Rust types, and a
        // system-runner crate depending on every service crate as a
        // library needs exactly one `SimWorld` type all of them share.
        // Single-service projects are untouched: still self-contained,
        // still vendored via `include_str!`, no `sim-shared` dependency.
        if !multi {
            project.add_file(at("src/clock.rs"), VENDORED_SIM_CLOCK);
            project.add_file(at("src/failure.rs"), VENDORED_SIM_FAILURE);
            project.add_file(at("src/scenario.rs"), VENDORED_SIM_SCENARIO);
            project.add_file(at("src/world.rs"), VENDORED_SIM_WORLD);
            if !ctx.jobs.is_empty() {
                project.add_file(at("src/cron.rs"), VENDORED_SIM_CRON);
            }
        }
        // v0.17 M11: `cargo run --bin sim_runner -- <scenario.json>` --
        // see the template's own doc comment for exactly what this does
        // and does not cover. `has_drain_workers` tells the template
        // whether any worker match arm exists at all, so it can name the
        // drained-payload binding `_raw` instead of `raw` when none do
        // (an empty match has nothing to deserialize `raw` into).
        let has_drain_workers = ctx.workers.iter().any(|w| !w.steps.is_empty());
        let sim_world_tables = sim_world_tables(ir);
        project.add_file(
            at("src/bin/sim_runner.rs"),
            render(
                "sim_runner.rs.j2",
                context! { has_drain_workers, sim_world_tables },
            )?,
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
                render(
                    "client.rs.j2",
                    context! { t => target, caller => ctx.service_name },
                )?,
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
    let service_for_sim = multi.then_some(ctx.service_name.as_str());
    for (name, hir) in &typed_handlers {
        let handler = lower::render(ir, name, hir, service_for_sim);
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

#[cfg(test)]
mod tests {
    /// Guards `vendor/ciac-sim/`'s own reason for existing: the
    /// `VENDORED_SIM_*` constants must stay byte-identical to
    /// `ciac-sim/src/{clock,cron,failure,scenario,world}.rs`. Runs
    /// only inside the workspace, where the sibling crate's source is
    /// reachable relative to `CARGO_MANIFEST_DIR` -- never true when
    /// building from a published crate's own package tarball, which
    /// contains only the vendored copy this test would have nothing to
    /// compare it against. If this fails, run
    /// `scripts/sync-vendored-sim.sh` and re-vendor.
    #[test]
    fn vendored_sim_matches_source() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        let sim_src = std::path::Path::new(manifest_dir).join("../ciac-sim/src");
        if !sim_src.is_dir() {
            return;
        }
        for name in ["clock", "cron", "failure", "scenario", "world"] {
            let source = std::fs::read_to_string(sim_src.join(format!("{name}.rs")))
                .unwrap_or_else(|e| panic!("reading ciac-sim/src/{name}.rs: {e}"));
            let vendored = std::fs::read_to_string(
                std::path::Path::new(manifest_dir).join(format!("vendor/ciac-sim/{name}.rs")),
            )
            .unwrap_or_else(|e| panic!("reading vendor/ciac-sim/{name}.rs: {e}"));
            assert_eq!(
                source, vendored,
                "vendor/ciac-sim/{name}.rs has drifted from ciac-sim/src/{name}.rs -- \
                 run scripts/sync-vendored-sim.sh"
            );
        }
    }
}
