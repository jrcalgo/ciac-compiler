//! TypeScript (Fastify ecosystem) code-generation backend.
//!
//! `23UpdatePlan.md` M1 — skeleton to ping-parity: `TargetInfo` and
//! enough of the generated-project shape (config/state/observability/
//! main + one route per plain `api`) to build, type-check, lint, and
//! test `examples/ping.ciac` through the real npm toolchain. Every
//! other construct (`db`, `cache`, `queue`, typed handlers, auth, ...)
//! stays refused by [`TsBackend::supports`] until its own milestone
//! lands — see `23UpdatePlan.md`'s milestone list for the order.
//!
//! Maps the CIaC ontology onto production-standard TypeScript
//! components, matching Python's/Rust's own doc-comment table:
//!
//! | CIaC          | TypeScript                       |
//! |---------------|-----------------------------------|
//! | API           | Fastify plugin (one per api)     |
//! | Service       | plain async class (`.handle`)    |
//! | Logging       | pino (via Fastify's own logger)  |

use ciac_codegen::model as context;
use ciac_codegen::{
    Backend, BackendError, DevCommands, GenOptions, GeneratedProject, RestartStyle, SimSupport,
    TargetInfo, ValidateStep,
};
use ciac_ir::{Component, NormalizedIr};
use include_dir::{include_dir, Dir};
use minijinja::context;

mod filters;
mod lower;

static TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

/// This backend's compose-file parameterization (v0.23 M1, per
/// `23UpdatePlan.md` Pillar 1's own `TargetInfo` sketch) — not yet
/// exercised by any generated compose file (no `db`/`cache`/`queue`
/// support until M2/M3), but declared now, matching Python's/Rust's
/// own precedent of a single, correct-from-the-start value rather
/// than a placeholder revisited later.
const COMPOSE_OPTS: ciac_codegen::compose::BackendComposeOpts =
    ciac_codegen::compose::BackendComposeOpts {
        db_url_scheme: "postgres",
        workers_command: r#"["node", "dist/workers.js"]"#,
        mysql_url_scheme: "mysql",
        sqlite_url_prefix: "file:data/",
        sqlite_url_suffix: "",
        data_mount: "/app/data",
    };

/// The generated CI workflow's `test` job steps: `setup-node` with the
/// npm cache, then the same `validate` sequence `ciac verify` runs
/// locally.
const CI_TEST_STEPS: &str = "      - uses: actions/setup-node@v4\n        with:\n          node-version: 22\n          cache: npm\n      - run: npm ci\n      - run: npx tsc --noEmit\n      - run: npx eslint .\n      - run: npx vitest run\n";

/// This target's whole CLI/CI/compose/dev-loop/sim integration surface
/// (v0.23 M1, following `22UpdatePlan.md` M1's `TargetInfo` pattern).
/// `validate` mirrors the CI steps above exactly (install, typecheck,
/// lint, test — the uv-sync/ruff/pytest and cargo-check/test sequences'
/// npm analog).
static TARGET_INFO: TargetInfo = TargetInfo {
    project_marker: "package.json",
    migrations_dir: "migrations",
    migration_filename: |seq, _slug| format!("{seq:04}_migration.sql"),
    validate: &[
        ValidateStep {
            program: "npm",
            args: &["ci"],
            env: &[],
            purpose: "install dependencies from the checked-in lockfile",
        },
        ValidateStep {
            program: "npx",
            args: &["tsc", "--noEmit"],
            env: &[],
            purpose: "type-checks",
        },
        ValidateStep {
            program: "npx",
            args: &["eslint", "."],
            env: &[],
            purpose: "lint",
        },
        ValidateStep {
            program: "npx",
            args: &["vitest", "run"],
            env: &[],
            purpose: "test",
        },
    ],
    ci_test_steps: CI_TEST_STEPS,
    compose: COMPOSE_OPTS,
    dev: DevCommands {
        rebuild: &[ValidateStep {
            program: "npm",
            args: &["run", "build"],
            env: &[],
            purpose: "rebuild before restart",
        }],
        restart: RestartStyle::Restart,
    },
    source_extension: "ts",
    sim: SimSupport::None {
        reason: "TypeScript simulation support lands in 23UpdatePlan.md M9 (gated bet)",
    },
};

#[derive(Debug, Default)]
pub struct TsBackend;

impl Backend for TsBackend {
    fn id(&self) -> &'static str {
        "typescript"
    }

    fn description(&self) -> &'static str {
        "TypeScript (Node 20+) project using Fastify (v0.23 M1: plain api routes only)"
    }

    fn supports(&self, component: &Component) -> bool {
        // v0.23 M1: a plain `api` (any method/path, typed or untyped
        // body, no pipeline steps beyond the implicit `Return`) —
        // matches `examples/ping.ciac`. v0.23 M2 adds: `db` on any of
        // the 3 engines (Postgres/MySQL/SQLite), `cache Redis`, and a
        // classic binding-style `service` (`signature: None`) — CRUD
        // resources and keyed-document stores compile to exactly this
        // component shape (`Database`, `Api`, `Service { signature:
        // None }`; see `23UpdatePlan.md` M2's own investigation note).
        // v0.23 M3 adds: `queue` on either broker (NATS/Kafka), `stream`
        // (the named subject a worker/channel relays), `worker` (both a
        // full pipeline-bearing worker and the bare `events X;`
        // consumer shape — both lower to `Component::Worker`, only
        // distinguished downstream in `ciac-codegen::model`), `job`
        // (cron) plus the `scheduler jobs Cron` capability declaration
        // that gates it, and `channel` (WebSocket/SSE realtime relay)
        // plus the `realtime live WebSocket`/`Sse` capability
        // declaration that gates it. v0.23 M4 adds: a *typed* `service`
        // (`signature: Some(..)`) — `crates/ciac-backend-ts/src/lower.rs`
        // implements every `HostSyntax` leaf Pillar 4's verb table
        // names, including `object_store`/`email`/`search`/
        // `external_http` (so the trait compiles completely, no
        // `unimplemented!()` leaves reachable), but the *component*
        // kinds that actually request those capabilities
        // (`Component::ObjectStore`/`Email`/`Search`/`ExternalHttp`)
        // stay refused here: 23UpdatePlan.md's own capability-parity
        // checklist places their wrapper clients at M7, not M4 (`db`/
        // `cache` already un-gated since M2 are the only capabilities
        // a typed handler can actually reach this milestone). This is
        // a disclosed scope boundary, not an oversight: `typed-
        // handlers.ciac` (needs `object_store`) and `extras-verbs.ciac`
        // (needs the M7 ontology wrappers) stay `CIAC0011`-refused,
        // matching the exact disclosed-deviation pattern M2 used for
        // `crud-notes.ciac` and M3 used for traceparent —
        // `domain-orders.ciac`/`query-verbs.ciac` (db-only) were M4's
        // proving examples instead. v0.23 M6 adds: `Component::Auth`
        // (JWT and OAuth2 — `jose` verifies both; only JWT gets the
        // no-infrastructure `tests/scope.test.ts` suite, OAuth2
        // excluded for the same live reason Rust's own scope-test gate
        // discloses: real RS256 verification needs a real issuer's
        // JWKS regardless of how lazily it's fetched). `typed-
        // video.ciac` (needs `auth`) un-gates this milestone;
        // `order-system.ciac` (JWT, the full scope-enforced surface)
        // and `oauth-echo.ciac` (OAuth2, static-only per the disclosed
        // gap above) are this milestone's proving examples.
        matches!(
            component,
            Component::Api { .. }
                | Component::Database { .. }
                | Component::Cache { .. }
                | Component::Service { .. }
                | Component::Queue { .. }
                | Component::Stream { .. }
                | Component::Worker { .. }
                | Component::Job { .. }
                | Component::Scheduler { .. }
                | Component::Channel { .. }
                | Component::Realtime { .. }
                | Component::Auth { .. }
        )
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
        env.add_filter("ts_type", filters::ts_type);
        env.add_filter("zod_schema", filters::zod_schema);
        env.add_filter("drizzle_column", filters::drizzle_column);
        env.add_filter("sql_ddl_type", filters::sql_ddl_type);
        env.add_function("id_ddl_type", filters::id_ddl_type);
        env.add_filter("reassigns_result", filters::reassigns_result);

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
                "openapi.json",
                serde_json::to_string_pretty(&ciac_codegen::openapi::build_index(&model))
                    .map_err(|e| BackendError::Other(e.to_string()))?,
            );
            let _ = m;
            project.notes.push(
                "multi-service system: each directory is a complete project; \
                 `docker compose up` runs them all together"
                    .to_owned(),
            );
        } else {
            project
                .notes
                .push("run with `npm ci && npm run build && npm start`, or `docker compose up --build` for the full stack".to_owned());
        }
        Ok(project)
    }
}

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

    project.add_file(at("package.json"), render("package.json.j2", empty())?);
    // Compiler-owned, not seeded: fully deterministic from the
    // declared dependency set (fixed for now; per-instance-conditional
    // once M2 adds db/cache drivers), so it is regenerated every
    // build like any other owned file, never hand-editable.
    project.add_file(
        at("package-lock.json"),
        render("package-lock.json.j2", empty())?,
    );
    project.add_file(at("tsconfig.json"), render("tsconfig.json.j2", empty())?);
    project.add_file(
        at("tsconfig.build.json"),
        render("tsconfig.build.json.j2", empty())?,
    );
    project.add_file(
        at("eslint.config.js"),
        render("eslint.config.js.j2", empty())?,
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
    }
    project.add_file(at(".gitignore"), "/node_modules\n/dist\n");
    // Docker doesn't read .gitignore -- without this, host-side
    // `node_modules`/`dist` from a local `npm ci`/`npm run build`
    // (as `ciac verify` runs before handing off to `docker compose`)
    // become part of the build context.
    project.add_file(at(".dockerignore"), "/node_modules\n/dist\n");

    project.add_file(at("src/main.ts"), render("main.ts.j2", empty())?);
    project.add_file(at("src/config.ts"), render("config.ts.j2", empty())?);
    project.add_file(at("src/state.ts"), render("state.ts.j2", empty())?);
    project.add_file(
        at("src/observability.ts"),
        render("observability.ts.j2", empty())?,
    );
    if ctx.queue_engine.is_some() {
        project.add_file(at("src/queue.ts"), render("queue.ts.j2", empty())?);
    }
    if ctx.has_auth {
        project.add_file(at("src/auth.ts"), render("auth.ts.j2", empty())?);
    }
    for api in &ctx.apis {
        project.add_file(
            at(&format!("src/routes/{}.ts", api.snake)),
            render("route_api.ts.j2", context! { api => api })?,
        );
    }
    for channel in &ctx.channels {
        project.add_file(
            at(&format!("src/routes/channel_{}.ts", channel.snake)),
            render("channel.ts.j2", context! { channel => channel })?,
        );
    }
    if !ctx.records.is_empty() {
        project.add_file(at("src/schemas.ts"), render("schemas.ts.j2", empty())?);
    }
    if !ctx.resources.is_empty() || !ctx.tables.is_empty() {
        project.add_file(at("src/models.ts"), render("models.ts.j2", empty())?);
        project.add_file(at("src/db.ts"), render("db.ts.j2", empty())?);
    }
    for resource in &ctx.resources {
        project.add_file(
            at(&format!("src/stores/{}.ts", resource.store_module)),
            render("resource_store.ts.j2", context! { resource => resource })?,
        );
        project.add_file(
            at(&format!("src/routes/{}.ts", resource.snake)),
            render("route_resource.ts.j2", context! { resource => resource })?,
        );
    }
    for service in &ctx.services {
        project.add_seeded_file(
            at(&format!("src/services/{}.ts", service.module)),
            render("service.ts.j2", context! { service => service })?,
        );
    }
    // v0.23 M4: typed handlers (`Component::Service { signature: Some(hir), .. }`).
    // Mirrors `ciac-backend-python`'s own `typed_handlers` dispatch:
    // inline bodies lower straight from the HIR and are compiler-owned
    // (`src/logic/`); `extern` gets a typed stub in `src/services/`
    // like classic handlers, since it's the same "implement this
    // yourself" contract.
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
    for (name, hir) in &typed_handlers {
        let handler = lower::render(ir, name, hir);
        let content = render(
            "logic.ts.j2",
            context! { handler => minijinja::Value::from_serialize(&handler) },
        )?;
        if hir.body.is_some() {
            project.add_file(at(&format!("src/logic/{}.ts", handler.module)), content);
        } else {
            project.add_seeded_file(at(&format!("src/services/{}.ts", handler.module)), content);
        }
    }
    for target in &ctx.call_targets {
        project.add_file(
            at(&format!("src/clients/{}.ts", target.module)),
            render("client.ts.j2", context! { t => target })?,
        );
    }
    if !ctx.workers.is_empty() || !ctx.jobs.is_empty() || !ctx.consumers.is_empty() {
        project.add_file(at("src/workers.ts"), render("workers_main.ts.j2", empty())?);
    }
    for worker in &ctx.workers {
        project.add_file(
            at(&format!("src/workers/{}.ts", worker.snake)),
            render("worker.ts.j2", context! { worker => worker })?,
        );
    }
    for job in &ctx.jobs {
        project.add_file(
            at(&format!("src/workers/{}.ts", job.snake)),
            render("job.ts.j2", context! { job => job })?,
        );
    }
    for consumer in &ctx.consumers {
        project.add_file(
            at(&format!("src/workers/{}.ts", consumer.snake)),
            render("consumer.ts.j2", context! { consumer => consumer })?,
        );
    }
    project.add_file(
        at("tests/state.test.ts"),
        render("state.test.ts.j2", empty())?,
    );
    // v0.23 M6: scope-enforcement behavioral test, JWT-only. OAuth2 is
    // excluded from this no-infrastructure suite for the same live
    // reason Rust's own `scope_tests.rs` gate discloses: real RS256
    // verification needs a real issuer's JWKS regardless of how
    // lazily it's fetched — a lazy `createRemoteJWKSet` just moves
    // *when* that network call happens (construction to first
    // request), it doesn't remove the need for it. A no-infrastructure
    // scope proof for OAuth2 needs an actual fake auth adapter, real,
    // disclosed future work this milestone didn't build.
    if ctx.auth_scheme == "jwt" && !ctx.scopes.is_empty() {
        project.add_file(
            at("tests/scope.test.ts"),
            render("scope.test.ts.j2", empty())?,
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(src: &str) -> NormalizedIr {
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = ciac_diagnostics::Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()))
    }

    const PING_SRC: &str = "service Ping;\n\nrecord Message {\n    id: Uuid;\n    text: String;\n}\n\napi Echo: Message {\n    method: POST;\n    path: \"/echo\";\n}\n\npipeline Echo: Return;\n";

    #[test]
    fn supports_v0_23_m6_scope() {
        let backend = TsBackend;
        assert!(backend.supports(&Component::Api {
            name: "X".to_owned(),
            request: None,
            config: ciac_ir::ApiConfig {
                method: ciac_ir::HttpMethod::Get,
                path: Some("/x".to_owned()),
                scope: None,
            },
        }));
        for engine in [
            ciac_ir::DbEngine::Postgres,
            ciac_ir::DbEngine::MySql,
            ciac_ir::DbEngine::Sqlite,
        ] {
            assert!(backend.supports(&Component::Database {
                name: "db".to_owned(),
                engine,
            }));
        }
        assert!(backend.supports(&Component::Cache {
            name: "cache".to_owned(),
            engine: ciac_ir::CacheEngine::Redis,
        }));
        assert!(backend.supports(&Component::Service {
            name: "svc".to_owned(),
            signature: None,
        }));
        for engine in [ciac_ir::QueueEngine::Nats, ciac_ir::QueueEngine::Kafka] {
            assert!(backend.supports(&Component::Queue {
                name: "queue".to_owned(),
                engine,
            }));
        }
        assert!(backend.supports(&Component::Worker {
            name: "w".to_owned(),
            config: ciac_ir::WorkerConfig::default(),
        }));
        assert!(backend.supports(&Component::Job {
            name: "j".to_owned(),
            config: ciac_ir::JobConfig {
                schedule: "0 3 * * *".to_owned(),
                catch_up: false,
            },
        }));
        assert!(backend.supports(&Component::Channel {
            name: "ch".to_owned(),
            config: ciac_ir::ChannelConfig {
                path: "/ch".to_owned(),
            },
        }));
        // v0.23 M6: Auth is now supported, both schemes.
        for scheme in [ciac_ir::AuthScheme::Jwt, ciac_ir::AuthScheme::OAuth2] {
            assert!(backend.supports(&Component::Auth {
                name: "auth".to_owned(),
                scheme,
                issuer: None,
                audience: None,
            }));
        }
    }

    #[test]
    fn generates_ping_project_files() {
        let ir = compile(PING_SRC);
        let backend = TsBackend;
        let project = backend
            .generate(&ir, &GenOptions::default())
            .expect("ping generates");
        let paths: Vec<&str> = project.files().map(|(p, _)| p).collect();
        assert!(paths.contains(&"package.json"), "{paths:?}");
        assert!(paths.contains(&"package-lock.json"), "{paths:?}");
        assert!(paths.contains(&"src/main.ts"), "{paths:?}");
        assert!(paths.contains(&"src/routes/echo.ts"), "{paths:?}");
        assert!(paths.contains(&"tests/state.test.ts"), "{paths:?}");

        let (_, main_ts) = project.files().find(|(p, _)| *p == "src/main.ts").unwrap();
        assert!(main_ts.contains("echoRoute"), "{main_ts}");
    }

    #[test]
    fn target_info_matches_the_validate_sequence() {
        let backend = TsBackend;
        let info = backend.target_info();
        assert_eq!(info.project_marker, "package.json");
        assert_eq!(info.source_extension, "ts");
        assert!(matches!(info.sim, SimSupport::None { .. }));
        let programs: Vec<&str> = info.validate.iter().map(|s| s.program).collect();
        assert_eq!(programs, vec!["npm", "npx", "npx", "npx"]);
    }
}
