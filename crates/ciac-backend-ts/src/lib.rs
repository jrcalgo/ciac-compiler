//! TypeScript (Fastify ecosystem) code-generation backend.
//!
//! `23UpdatePlan.md` M1 — skeleton to ping-parity: `TargetInfo` and
//! enough of the generated-project shape (config/state/observability/
//! main + one route per plain `api`) to build, type-check, lint, and
//! test `examples/single-service/ping.ciac` through the real npm toolchain. Every
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
    sim: SimSupport::Narrow {
        unsupported: unsupported_sim_capabilities,
    },
    // 27UpdatePlan.md M1: see ciac-backend-rust's identical comment —
    // depth and replay-tape support are decoupled fields on purpose.
    sim_replay: false,
};

/// Human-readable, closed list of reasons `ciac sim --target typescript`
/// cannot yet simulate `ir`, empty when it can. Always empty as of
/// 27UpdatePlan.md M6 -- every verb `lower::scan`'s `unguarded_verbs`
/// tracks now has a `world.ts` guard leaf (`db.get`/`update`/`delete`/
/// `query`/`count`/`delete_where`, `cache.*`, `object_store.*`,
/// `email.send`, `search.*`, `http.call`) and `auth` is guarded via
/// `state.world.authVerify` in `auth.ts.j2` (claims-lookup, matching
/// Python's `FakeAuth`), retiring the blanket auth refusal this
/// function used to carry.
///
/// This backend stays `SimSupport::Narrow` (never flips to `Full`) --
/// `crates/ciac/src/commands.rs`'s `sim_inner` dispatch hardcodes
/// `SimSupport::Full => sim_drive_python(..)`, so flipping the enum
/// variant would silently misroute TypeScript-generated projects
/// through Python's driver (the same structural finding Rust's own M4
/// made and corrected; docs/targets.json record the behavioral
/// "full" state precisely rather than flip a JSON field that would
/// then contradict the code).
///
/// One real, disclosed (not modeled here) gap: `crud <Name>: <Record>`
/// resources (`resource_store.ts.j2`) never read `this.state.world` at
/// all -- but confirmed unreachable through `ciac sim`, the same
/// finding Rust's M4 made: a scenario's `request` step can only
/// address `c.apis`, built from nodes with an attached `Pipeline`,
/// which a crud resource's synthesized api node never has (this
/// backend and Rust's share the same `ciac-codegen` `c.apis` builder,
/// so the finding transfers without needing to be re-proven per
/// target). See `docs/simulation.md`.
pub fn unsupported_sim_capabilities(_ir: &NormalizedIr) -> Vec<String> {
    Vec::new()
}

/// Template-facing counterpart of `ciac-sim`'s `WorldReference` --
/// `sim_runner.ts.j2` renders each of these as a `WorldReference`
/// object literal (27UpdatePlan.md M6). `on_delete` is spelled
/// `"cascade"`/`"restrict"`, matching `world.ts`'s own
/// `WorldRefAction` string-union type exactly.
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

/// Builds the schema `sim_runner.ts.j2` passes to `new SimWorld(..)`
/// (27UpdatePlan.md M6) -- without it, `SimWorld` falls back to an
/// empty schema and every reference/unique/cascade check silently
/// becomes a no-op, the same gap Rust's own M4 caught live against
/// `domain-orders.ciac`. Reuses `ciac_codegen::migrations::snapshot_schema`
/// -- the same reference/unique-column facts the migration DDL itself
/// is built from, so this can never drift from what the real schema
/// actually enforces. Mirrors `ciac-backend-rust::sim_world_tables`
/// exactly, modulo the lowercase `on_delete` spelling.
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
                        "cascade"
                    } else {
                        "restrict"
                    },
                })
                .collect();
            SimWorldTableCtx { name, references }
        })
        .collect()
}

/// Multi-service counterpart of [`sim_world_tables`], namespacing every
/// table name (and FK `targetTable`) `"{service}::{table}"` the same way
/// `lower.rs`'s `world_table_key` composes them at typed-handler
/// lowering time -- mirrors `ciac-backend-rust::sim_world_tables_multi`
/// exactly (see that function's own doc comment for the full rationale;
/// `heck`'s `to_snake_case` mirrors `ciac_codegen::migrations`'s private
/// `physical_table_name` the same way that one does).
fn sim_world_tables_multi(ir: &NormalizedIr) -> Vec<SimWorldTableCtx> {
    use heck::ToSnakeCase;

    let mut owner_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (_, table) in ir.tables() {
        if let Some(sid) = table.service {
            owner_of.insert(table.name.to_snake_case(), ir.service(sid).name.clone());
        }
    }
    let namespace = |physical: &str| -> String {
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
                        "cascade"
                    } else {
                        "restrict"
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

/// The `sim-shared` npm package's own fixed files (28UpdatePlan.md M7):
/// mirrors Rust's M6b `sim-shared` crate -- TypeScript's `SimWorld`
/// class declares `private` fields, and TypeScript's structural typing
/// treats two independently-declared classes with private members as
/// mutually incompatible even when textually identical, so today's
/// per-service emission (each service rendering its own byte-identical
/// copy of `world.ts.j2`) hits the exact same nominal-type-identity
/// problem Rust's M6b found, just via TS's private-member rule instead
/// of Rust's per-crate type identity. One canonical `world.ts`, built
/// once and depended on by every service (and the system-runner) via a
/// `file:../sim-shared` npm dependency, fixes it the same way. Real
/// `package-lock.json` content (verified live against `npm ci`/`npm run
/// build` producing `dist/world.js`+`.d.ts`) -- not hand-guessed, since
/// npm's lockfile format requires exact integrity hashes for its
/// (`typescript`, the package's only dependency) resolved packages.
const SIM_SHARED_PACKAGE_JSON: &str = r#"{
  "name": "sim-shared",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "main": "dist/world.js",
  "types": "dist/world.d.ts",
  "scripts": {
    "build": "tsc -p tsconfig.build.json"
  },
  "devDependencies": {
    "@types/node": "22.20.1",
    "typescript": "5.9.3"
  }
}
"#;

const SIM_SHARED_PACKAGE_LOCK_JSON: &str = r#"{
  "name": "sim-shared",
  "version": "0.1.0",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {
    "": {
      "name": "sim-shared",
      "version": "0.1.0",
      "devDependencies": {
        "@types/node": "22.20.1",
        "typescript": "5.9.3"
      }
    },
    "node_modules/@types/node": {
      "version": "22.20.1",
      "resolved": "https://registry.npmjs.org/@types/node/-/node-22.20.1.tgz",
      "integrity": "sha512-EANqOCF9QFyra+4pfxUcX9STKJpCLjMbObVzljIJomAWSnuSIEAvyzEU53GaajbXJEgdh0iEcPL+DGvpUd4k1Q==",
      "dev": true,
      "license": "MIT",
      "dependencies": {
        "undici-types": "~6.21.0"
      }
    },
    "node_modules/typescript": {
      "version": "5.9.3",
      "resolved": "https://registry.npmjs.org/typescript/-/typescript-5.9.3.tgz",
      "integrity": "sha512-jl1vZzPDinLr9eUt3J/t7V6FgNEw9QjvBPdysz9KfQDD41fQrC2Y4vKQdiaUpFT4bXlb1RHhLpp8wtm6M5TgSw==",
      "dev": true,
      "license": "Apache-2.0",
      "bin": {
        "tsc": "bin/tsc",
        "tsserver": "bin/tsserver"
      },
      "engines": {
        "node": ">=14.17"
      }
    },
    "node_modules/undici-types": {
      "version": "6.21.0",
      "resolved": "https://registry.npmjs.org/undici-types/-/undici-types-6.21.0.tgz",
      "integrity": "sha512-iwDZqg0QAGrg9Rav5H4n0M64c3mkR59cJ6wQp+7C4nI0gsmExaedaYLNO44eT4AtBBwjbTiGPMlt2Md0T9H9JQ==",
      "dev": true,
      "license": "MIT"
    }
  }
}
"#;

const SIM_SHARED_TSCONFIG_JSON: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "NodeNext",
    "moduleResolution": "NodeNext",
    "lib": ["ES2022"],
    "outDir": "dist",
    "strict": true,
    "esModuleInterop": true,
    "skipLibCheck": true,
    "forceConsistentCasingInFileNames": true,
    "resolveJsonModule": true,
    "declaration": true,
    "sourceMap": false,
    "noEmit": true
  },
  "include": ["src"]
}
"#;

const SIM_SHARED_TSCONFIG_BUILD_JSON: &str = r#"{
  "extends": "./tsconfig.json",
  "compilerOptions": {
    "noEmit": false,
    "rootDir": "src"
  },
  "include": ["src"]
}
"#;

const SIM_SHARED_GITIGNORE: &str = "/node_modules\n/dist\n";

/// The `system-runner` npm package's own fixed `tsconfig.json`/
/// `tsconfig.build.json`/`.gitignore` (28UpdatePlan.md M7a) -- identical
/// in shape to `sim-shared`'s own (see that const's doc comment); no
/// `declaration` output is needed here since nothing depends on
/// `system-runner`'s own types.
const SYSTEM_RUNNER_TSCONFIG_JSON: &str = SIM_SHARED_TSCONFIG_JSON;
const SYSTEM_RUNNER_TSCONFIG_BUILD_JSON: &str = SIM_SHARED_TSCONFIG_BUILD_JSON;
const SYSTEM_RUNNER_GITIGNORE: &str = SIM_SHARED_GITIGNORE;

/// The `system-runner` package's own `package.json` (28UpdatePlan.md
/// M7a): a plain dependency list on `sim-shared` and every service
/// package by name, plus `croner` (the system-runner's own `dueInstants`
/// helper needs it directly, same as every generated service). Verified
/// live (real `npm install`/`npm run build`/`node` run across a
/// sim-shared + service + system-runner trio in the scratchpad) that
/// `system-runner` needs no direct dependency on `fastify`/`pg`/etc: a
/// `file:` dependency's own transitive dependencies are never hoisted
/// into the depending package's `node_modules` (confirmed against real
/// `npm install --package-lock-only` output) -- Node resolves a bare
/// specifier reached through a `file:` symlink from *that* target's own
/// real directory (and its own already-`npm ci`'d `node_modules`), not
/// from the depender's. `system_sim_runner.ts.j2`'s own doc comment
/// records the same finding for why no shared Fastify-typed `dispatch`
/// helper is used either.
fn system_runner_package_json(model: &context::SystemModel) -> Result<String, BackendError> {
    let mut dependencies = serde_json::Map::new();
    dependencies.insert(
        "sim-shared".to_owned(),
        serde_json::Value::String("file:../sim-shared".to_owned()),
    );
    dependencies.insert(
        "croner".to_owned(),
        serde_json::Value::String("10.0.1".to_owned()),
    );
    for ctx in &model.services {
        dependencies.insert(
            ctx.package.clone(),
            serde_json::Value::String(format!("file:../{}", ctx.dir)),
        );
    }
    let value = serde_json::json!({
        "name": "system-runner",
        "version": "0.1.0",
        "private": true,
        "type": "module",
        "scripts": {
            "build": "tsc -p tsconfig.build.json",
            "start": "node dist/sim_runner.js"
        },
        "dependencies": dependencies,
        "devDependencies": {
            "@types/node": "22.20.1",
            "typescript": "5.9.3"
        }
    });
    serde_json::to_string_pretty(&value)
        .map(|s| s + "\n")
        .map_err(|e| BackendError::Other(e.to_string()))
}

/// Every service's own `package.json` (`package.json.j2`) renders this
/// exact fixed dependency/devDependency map, unconditionally, for every
/// service -- the only per-service variable is the `name` field and the
/// `sim-shared` line (always present here since every entry point is a
/// multi-service system). Reused both to build the `system-runner`
/// lockfile's own `"../<dir>"` informational entries and as the
/// canonical value [`assert_no_dependency_skew`] compares every real
/// rendered service `package.json` against.
fn canonical_service_dependencies() -> serde_json::Value {
    serde_json::json!({
        "sim-shared": "file:../sim-shared",
        "@aws-sdk/client-s3": "3.1090.0",
        "@fastify/otel": "0.20.1",
        "@fastify/websocket": "11.3.0",
        "@grpc/grpc-js": "1.14.4",
        "@nats-io/transport-node": "3.4.0",
        "@opensearch-project/opensearch": "3.6.0",
        "@opentelemetry/api": "1.9.1",
        "@opentelemetry/exporter-trace-otlp-grpc": "0.220.0",
        "@opentelemetry/instrumentation": "0.220.0",
        "@opentelemetry/instrumentation-http": "0.220.0",
        "@opentelemetry/instrumentation-pg": "0.72.0",
        "@opentelemetry/instrumentation-undici": "0.30.0",
        "@opentelemetry/resources": "2.9.0",
        "@opentelemetry/sdk-trace-base": "2.9.0",
        "@opentelemetry/sdk-trace-node": "2.9.0",
        "@opentelemetry/semantic-conventions": "1.43.0",
        "better-sqlite3": "12.11.1",
        "croner": "10.0.1",
        "drizzle-orm": "0.45.2",
        "fastify": "5.10.0",
        "ioredis": "5.11.1",
        "jose": "6.2.3",
        "kafkajs": "2.2.4",
        "mysql2": "3.23.0",
        "nodemailer": "9.0.3",
        "pg": "8.22.0",
        "pino": "10.3.1",
        "prom-client": "15.1.3",
        "zod": "3.25.76"
    })
}

fn canonical_service_dev_dependencies() -> serde_json::Value {
    serde_json::json!({
        "@eslint/js": "10.0.1",
        "@types/better-sqlite3": "7.6.13",
        "@types/node": "22.20.1",
        "@types/nodemailer": "8.0.1",
        "@types/pg": "8.20.0",
        "eslint": "10.7.0",
        "typescript": "5.9.3",
        "typescript-eslint": "8.64.0",
        "vitest": "4.1.10"
    })
}

/// The `system-runner` package's own `package-lock.json` -- shaped
/// exactly like the real `npm install`-produced lockfile verified live
/// in the scratchpad for a sim-shared + service + system-runner trio:
/// the root `""` entry lists this package's own manifest, one `"../
/// <dir>"` informational entry per linked package (mirroring what that
/// package's own real `package.json` declares -- confirmed live that
/// `npm ci` does not actually validate this field against the target's
/// real manifest, but it is kept accurate here rather than relying on
/// that leniency), and `node_modules/*` entries: one `link: true` entry
/// per linked package (`sim-shared` + every service), plus the three
/// ordinary registry packages `system-runner` itself directly depends
/// on (`croner`, `typescript`, `@types/node` and its own `undici-types`
/// dependency) -- integrity hashes copied from the already-live-verified
/// `package-lock.json.j2`/`SIM_SHARED_PACKAGE_LOCK_JSON` entries for the
/// same pinned versions.
fn system_runner_package_lock_json(model: &context::SystemModel) -> Result<String, BackendError> {
    let mut root_dependencies = serde_json::Map::new();
    root_dependencies.insert(
        "sim-shared".to_owned(),
        serde_json::Value::String("file:../sim-shared".to_owned()),
    );
    root_dependencies.insert(
        "croner".to_owned(),
        serde_json::Value::String("10.0.1".to_owned()),
    );
    let mut packages = serde_json::Map::new();
    for ctx in &model.services {
        root_dependencies.insert(
            ctx.package.clone(),
            serde_json::Value::String(format!("file:../{}", ctx.dir)),
        );
        packages.insert(
            format!("../{}", ctx.dir),
            serde_json::json!({
                "version": "0.1.0",
                "dependencies": canonical_service_dependencies(),
                "devDependencies": canonical_service_dev_dependencies(),
            }),
        );
        packages.insert(
            format!("node_modules/{}", ctx.package),
            serde_json::json!({ "resolved": format!("../{}", ctx.dir), "link": true }),
        );
    }
    packages.insert(
        "".to_owned(),
        serde_json::json!({
            "name": "system-runner",
            "version": "0.1.0",
            "dependencies": root_dependencies,
            "devDependencies": {
                "@types/node": "22.20.1",
                "typescript": "5.9.3"
            }
        }),
    );
    packages.insert(
        "../sim-shared".to_owned(),
        serde_json::json!({
            "version": "0.1.0",
            "devDependencies": {
                "@types/node": "22.20.1",
                "typescript": "5.9.3"
            }
        }),
    );
    packages.insert(
        "node_modules/sim-shared".to_owned(),
        serde_json::json!({ "resolved": "../sim-shared", "link": true }),
    );
    packages.insert(
        "node_modules/@types/node".to_owned(),
        serde_json::json!({
            "version": "22.20.1",
            "resolved": "https://registry.npmjs.org/@types/node/-/node-22.20.1.tgz",
            "integrity": "sha512-EANqOCF9QFyra+4pfxUcX9STKJpCLjMbObVzljIJomAWSnuSIEAvyzEU53GaajbXJEgdh0iEcPL+DGvpUd4k1Q==",
            "dev": true,
            "license": "MIT",
            "dependencies": { "undici-types": "~6.21.0" }
        }),
    );
    packages.insert(
        "node_modules/undici-types".to_owned(),
        serde_json::json!({
            "version": "6.21.0",
            "resolved": "https://registry.npmjs.org/undici-types/-/undici-types-6.21.0.tgz",
            "integrity": "sha512-iwDZqg0QAGrg9Rav5H4n0M64c3mkR59cJ6wQp+7C4nI0gsmExaedaYLNO44eT4AtBBwjbTiGPMlt2Md0T9H9JQ==",
            "dev": true,
            "license": "MIT"
        }),
    );
    packages.insert(
        "node_modules/typescript".to_owned(),
        serde_json::json!({
            "version": "5.9.3",
            "resolved": "https://registry.npmjs.org/typescript/-/typescript-5.9.3.tgz",
            "integrity": "sha512-jl1vZzPDinLr9eUt3J/t7V6FgNEw9QjvBPdysz9KfQDD41fQrC2Y4vKQdiaUpFT4bXlb1RHhLpp8wtm6M5TgSw==",
            "dev": true,
            "license": "Apache-2.0",
            "bin": { "tsc": "bin/tsc", "tsserver": "bin/tsserver" },
            "engines": { "node": ">=14.17" }
        }),
    );
    packages.insert(
        "node_modules/croner".to_owned(),
        serde_json::json!({
            "version": "10.0.1",
            "resolved": "https://registry.npmjs.org/croner/-/croner-10.0.1.tgz",
            "integrity": "sha512-ixNtAJndqh173VQ4KodSdJEI6nuioBWI0V1ITNKhZZsO0pEMoDxz539T4FTTbSZ/xIOSuDnzxLVRqBVSvPNE2g==",
            "funding": [
                { "type": "other", "url": "https://paypal.me/hexagonpp" },
                { "type": "github", "url": "https://github.com/sponsors/hexagon" }
            ],
            "license": "MIT",
            "engines": { "node": ">=18.0" }
        }),
    );

    let value = serde_json::json!({
        "name": "system-runner",
        "version": "0.1.0",
        "lockfileVersion": 3,
        "requires": true,
        "packages": packages,
    });
    serde_json::to_string_pretty(&value)
        .map(|s| s + "\n")
        .map_err(|e| BackendError::Other(e.to_string()))
}

/// 28UpdatePlan.md M7a's own composition matrix names this target's one
/// sharp edge as "dependency-version skew across the N generated
/// `package.json`s (identical by construction today -- asserted)" --
/// this is that assertion, checked for real against the actually
/// rendered files rather than only assumed: every service's own
/// `package.json.j2` renders the exact same fixed dependency/
/// devDependency map regardless of what that service declares (the only
/// per-service variables are the `name` field and, uniformly here, the
/// `sim-shared` line), so this should always hold -- it exists to catch
/// a future edit to that template that made a dependency version
/// conditional on something service-specific, not because any skew is
/// expected today.
fn assert_no_dependency_skew(
    project: &GeneratedProject,
    model: &context::SystemModel,
) -> Result<(), BackendError> {
    let mut canonical: Option<(&str, serde_json::Value, serde_json::Value)> = None;
    for ctx in &model.services {
        let path = format!("{}/package.json", ctx.dir);
        let content = project
            .get(&path)
            .ok_or_else(|| BackendError::Other(format!("expected {path} to already be emitted")))?;
        let parsed: serde_json::Value = serde_json::from_str(content)
            .map_err(|e| BackendError::Other(format!("parsing {path}: {e}")))?;
        let deps = parsed.get("dependencies").cloned().unwrap_or_default();
        let dev_deps = parsed.get("devDependencies").cloned().unwrap_or_default();
        match &canonical {
            None => canonical = Some((ctx.service_name.as_str(), deps, dev_deps)),
            Some((first_service, canonical_deps, canonical_dev_deps)) => {
                if &deps != canonical_deps || &dev_deps != canonical_dev_deps {
                    return Err(BackendError::Other(format!(
                        "dependency-version skew detected: service {:?}'s package.json \
                         dependencies diverge from service {first_service:?}'s -- \
                         28UpdatePlan.md M7a's system-runner assumes every service's \
                         dependency set is identical (see `assert_no_dependency_skew`'s own \
                         doc comment)",
                        ctx.service_name
                    )));
                }
            }
        }
    }
    Ok(())
}

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
        // Full provider parity as of v0.23 M8, built up milestone by
        // milestone (M1: a plain `api`; M2: `db`/`cache`/classic
        // `service`; M3: `queue`/`stream`/`worker`/`job`/`scheduler`/
        // `channel`/`realtime`; M4: typed `service` bodies and every
        // `HostSyntax` leaf Pillar 4's verb table names; M6: `auth`
        // JWT/OAuth2; M7: `object_store`/`email`/`search`/
        // `external_http` wrapper clients and `metrics`/`tracing`/
        // `logging`/`users`) — every `Component` variant reaches this
        // backend now, so, like Rust's own `supports`, there is
        // nothing left to gate on. See `23UpdatePlan.md`'s own
        // milestone-by-milestone shipped-notes for the disclosed scope
        // boundaries each earlier milestone held (and closed) along
        // the way.
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
                env.add_filter("ts_type", filters::ts_type);
                env.add_filter("zod_schema", filters::zod_schema);
                env.add_filter("drizzle_column", filters::drizzle_column);
                env.add_filter("sql_ddl_type", filters::sql_ddl_type);
                env.add_function("id_ddl_type", filters::id_ddl_type);
                env.add_filter("reassigns_result", filters::reassigns_result);
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
            // 28UpdatePlan.md M7a: one `sim-shared` npm package per
            // system -- see `SIM_SHARED_PACKAGE_JSON`'s own doc comment
            // for why TS needs this despite structural typing (private
            // class members break the structural-compatibility
            // shortcut). Only emitted when at least one service
            // actually needs the simulation world, mirroring Rust's
            // identical gate.
            if model.services.iter().any(|ctx| {
                ctx.has_db
                    || ctx.queue_engine.is_some()
                    || ctx.has_cache
                    || ctx.has_object_store
                    || ctx.has_email
                    || ctx.has_search
                    || ctx.has_external_http
                    || ctx.has_auth
                    || !ctx.call_targets.is_empty()
            }) {
                project.add_file("sim-shared/package.json", SIM_SHARED_PACKAGE_JSON);
                project.add_file("sim-shared/package-lock.json", SIM_SHARED_PACKAGE_LOCK_JSON);
                project.add_file("sim-shared/tsconfig.json", SIM_SHARED_TSCONFIG_JSON);
                project.add_file(
                    "sim-shared/tsconfig.build.json",
                    SIM_SHARED_TSCONFIG_BUILD_JSON,
                );
                project.add_file("sim-shared/.gitignore", SIM_SHARED_GITIGNORE);
                project.add_file(
                    "sim-shared/src/world.ts",
                    env.get_template("world.ts.j2")?.render(context! {})?,
                );

                // 28UpdatePlan.md M7a: the `system-runner` package --
                // `sim_drive_typescript`'s eventual multi-service
                // counterpart to driving a single service's own `src/
                // sim_runner.ts` (see `system_sim_runner.ts.j2`'s own doc
                // comment for the full architecture). Gated on the same
                // condition as `sim-shared` itself since it depends on
                // that package unconditionally and has nothing to drive
                // without it.
                assert_no_dependency_skew(&project, &model)?;
                project.add_file(
                    "system-runner/package.json",
                    system_runner_package_json(&model)?,
                );
                project.add_file(
                    "system-runner/package-lock.json",
                    system_runner_package_lock_json(&model)?,
                );
                project.add_file("system-runner/tsconfig.json", SYSTEM_RUNNER_TSCONFIG_JSON);
                project.add_file(
                    "system-runner/tsconfig.build.json",
                    SYSTEM_RUNNER_TSCONFIG_BUILD_JSON,
                );
                project.add_file("system-runner/.gitignore", SYSTEM_RUNNER_GITIGNORE);
                let services = minijinja::Value::from_serialize(&model.services);
                let sim_world_tables = sim_world_tables_multi(ir);
                project.add_file(
                    "system-runner/src/sim_runner.ts",
                    env.get_template("system_sim_runner.ts.j2")?
                        .render(context! { services, sim_world_tables })?,
                );
            }
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
            .render(context! { c => base, multi, ..extra })?)
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
    // 27UpdatePlan.md M6: the simulation world -- for any program with
    // something it can fake (db, queue, cache, object_store, email,
    // search, external_http, or auth), the same broadened gate Rust's
    // own `world.rs`/`sim_runner.rs` emission (27UpdatePlan.md M4)
    // uses -- a program with only a peripheral capability and no
    // db/queue (e.g. a cache-only or auth-only program) still needs
    // `src/world.ts` to exist.
    if ctx.has_db
        || ctx.queue_engine.is_some()
        || ctx.has_cache
        || ctx.has_object_store
        || ctx.has_email
        || ctx.has_search
        || ctx.has_external_http
        || ctx.has_auth
        || !ctx.call_targets.is_empty()
    {
        // 28UpdatePlan.md M7a: multi-service systems get one shared
        // `sim-shared/src/world.ts` instead (see `SIM_SHARED_PACKAGE_
        // JSON`'s own doc comment) -- `state.ts.j2`/`queue.ts.j2`/
        // `sim_runner.ts.j2`'s own `SimWorld` import already switches
        // to the bare `"sim-shared"` specifier when `multi` (this
        // `render` closure passes `multi` into every template).
        if !multi {
            project.add_file(at("src/world.ts"), render("world.ts.j2", empty())?);
        }
        project.add_file(
            at("src/sim_runner.ts"),
            render(
                "sim_runner.ts.j2",
                context! { sim_world_tables => sim_world_tables(ir) },
            )?,
        );
    }
    if ctx.has_auth {
        project.add_file(at("src/auth.ts"), render("auth.ts.j2", empty())?);
    }
    // v0.23 M7: one shared wrapper module per ontology capability
    // *kind* (not per named instance) -- `state.ts`'s per-instance
    // loop constructs as many clients as declared, all from the same
    // class.
    if ctx.has_object_store {
        project.add_file(
            at("src/object_store.ts"),
            render("object_store.ts.j2", empty())?,
        );
    }
    if ctx.has_email {
        project.add_file(at("src/email.ts"), render("email.ts.j2", empty())?);
    }
    if ctx.has_search {
        project.add_file(at("src/search.ts"), render("search.ts.j2", empty())?);
    }
    if ctx.has_external_http {
        project.add_file(
            at("src/http_clients.ts"),
            render("http_clients.ts.j2", empty())?,
        );
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
    let service_for_sim = multi.then_some(ctx.service_name.as_str());
    for (name, hir) in &typed_handlers {
        let handler = lower::render(ir, name, hir, service_for_sim);
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
    // v0.23 M6: scope-enforcement behavioral test. `26UpdatePlan.md`
    // M5 widened this from JWT-only to both schemes: the file's own
    // `no_token`/`malformed_token` cases are scheme-agnostic (gated
    // inside the template on `has_auth_step`/`has_auth`), and its
    // JWT-only `bearer`/`bearerExp` helpers plus their wrong_scope/
    // correct_scope/expired_token blocks stay gated on
    // `c.auth_scheme == "jwt"` inside the template. OAuth2 gets the
    // scheme-specific equivalent via the real-RS256 rig below --
    // closing the "future work" this file's v0.23 M6 comment used to
    // disclose (real RS256 verification needing a real issuer's JWKS
    // is no longer a blocker once the JWKS server itself is an
    // in-process stub, not a live IdP).
    if !ctx.scopes.is_empty() {
        project.add_file(
            at("tests/scope.test.ts"),
            render("scope.test.ts.j2", empty())?,
        );
    }
    // The no-infra OAuth2 rig (`26UpdatePlan.md` M5): real RS256
    // signing against an in-process JWKS stub, gated the same way the
    // scope suite is gated on `c.scopes` above.
    if ctx.auth_scheme == "oauth2" && !ctx.scopes.is_empty() {
        project.add_file(
            at("tests/oauth-stub.ts"),
            render("oauth_stub.ts.j2", empty())?,
        );
        project.add_file(
            at("tests/oauth-rig.test.ts"),
            render("oauth_rig.test.ts.j2", empty())?,
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
    fn supports_full_provider_parity() {
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
        // v0.23 M9: TypeScript's own gated-bet simulation slice, same
        // scope shape as Rust's (v0.17 M11) -- narrow, not absent.
        assert!(matches!(info.sim, SimSupport::Narrow { .. }));
        let programs: Vec<&str> = info.validate.iter().map(|s| s.program).collect();
        assert_eq!(programs, vec!["npm", "npx", "npx", "npx"]);
    }
}
