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
        // v0.23 M1 scope: a plain `api` (any method/path, typed or
        // untyped body, no pipeline steps beyond the implicit
        // `Return`) is the whole gate — matches `examples/ping.ciac`
        // exactly. Every other component kind (db/cache/queue/
        // service/worker/job/channel/auth/...) stays refused
        // (`CIAC0011`) until its own milestone un-gates it.
        matches!(component, Component::Api { .. })
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
    for api in &ctx.apis {
        project.add_file(
            at(&format!("src/routes/{}.ts", api.snake)),
            render("route_api.ts.j2", context! { api => api })?,
        );
    }
    project.add_file(
        at("tests/state.test.ts"),
        render("state.test.ts.j2", empty())?,
    );

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
    fn supports_plain_apis_only() {
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
        assert!(!backend.supports(&Component::Database {
            name: "db".to_owned(),
            engine: ciac_ir::DbEngine::Postgres,
        }));
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
