//! Go (stdlib `net/http` + `database/sql` ecosystem) code-generation
//! backend (`24UpdatePlan.md`).
//!
//! Maps the CIaC ontology onto production-standard Go components:
//!
//! | CIaC          | Go                                       |
//! |---------------|-------------------------------------------|
//! | API           | `net/http` 1.22+ `ServeMux` handler        |
//! | Service       | plain struct with a `Handle` method        |
//! | Worker        | NATS/Kafka consumer goroutine               |
//! | Database      | `database/sql` + pgx/mysql/modernc drivers |
//! | Cache         | go-redis                                   |
//! | Queue         | nats.go / franz-go                         |
//! | Auth (JWT)    | golang-jwt + generated middleware          |
//! | Logging       | `log/slog` (stdlib, JSON handler)          |
//! | Metrics       | prometheus/client_golang `/metrics`        |
//!
//! The generated project is buildable without any infrastructure
//! running: `database/sql` pools, the go-redis client, and the broker
//! client are all lazy by construction (v0.24 M1's own no-infra
//! construction test, `goleak`-backed, proves this from day one).
//!
//! **Routing, decided at M1 (`24UpdatePlan.md`'s pre-registered open
//! question #3):** every route shape this compiler emits (static
//! paths, single-segment `{id}` params, method-first dispatch) is
//! covered by Go 1.22's `ServeMux` pattern matching alone. `chi` —
//! Pillar 1's tentative pick — is dropped: it would add a dependency
//! and an indirection (`chi.URLParam` vs. `r.PathValue`) for
//! middleware grouping this backend's own [`httpx`] helpers give for
//! free by plain function wrapping. Recorded here, not silently
//! substituted, since Pillar 1's table named chi as the pick.

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

/// This backend's two compose-file divergences (mirroring
/// `ciac-backend-python`/`-rust`/`-ts`'s own `COMPOSE_OPTS`): pgx
/// accepts a plain `postgres://` URL DSN directly, and the workers
/// binary is the second compiled binary (`cmd/workers`), started with
/// no interpreter. `mysql_url_scheme`'s value is unused by this
/// backend's own config path (the Pillar 1 DSN containment note:
/// `go-sql-driver/mysql` wants `user:pass@tcp(host:port)/db`, not a
/// URL, so `internal/config` composes that shape from the same
/// discrete env vars compose already emits for every target) — kept
/// non-empty only so the shared compose template's env-var line still
/// renders something legible for a human reading `docker-compose.yml`.
const COMPOSE_OPTS: ciac_codegen::compose::BackendComposeOpts =
    ciac_codegen::compose::BackendComposeOpts {
        db_url_scheme: "postgres",
        workers_command: r#"["/app/workers"]"#,
        mysql_url_scheme: "mysql",
        sqlite_url_prefix: "file:data/",
        sqlite_url_suffix: "",
        data_mount: "/data",
    };

/// The literal CI test-step YAML for this target (`24UpdatePlan.md`
/// Pillar 1): `setup-go@v5` with module caching, then the same
/// build/vet/gofmt/test sequence `validate` runs locally, plus
/// `staticcheck` — CI's own extra layer beyond what `ciac verify`
/// asks a developer's machine to install.
const CI_TEST_STEPS: &str = "      - uses: actions/setup-go@v5\n        with:\n          go-version: \"1.24\"\n          cache-dependency-path: go.sum\n      - run: go build ./...\n      - run: go vet ./...\n      - run: test -z \"$(gofmt -l .)\"\n      - run: go test ./...\n      - run: go run honnef.co/go/tools/cmd/staticcheck@latest ./...\n";

/// This target's whole CLI/CI/compose/dev-loop/sim integration surface
/// (`22UpdatePlan.md` Pillar 1's factory contract, consumed here
/// exactly as `ciac-backend-python`/`-rust`/`-ts` already do), reached
/// through the `Backend` trait instead of a per-call-site
/// `match target { "python" => .., .. }`.
static TARGET_INFO: TargetInfo = TargetInfo {
    project_marker: "go.mod",
    migrations_dir: "migrations",
    migration_filename: |seq, _slug| format!("{seq:04}_migration.sql"),
    validate: &[
        ValidateStep {
            program: "go",
            args: &["build", "./..."],
            env: &[("CGO_ENABLED", "0")],
            purpose: "compiles to a static binary",
        },
        ValidateStep {
            program: "go",
            args: &["vet", "./..."],
            env: &[("CGO_ENABLED", "0")],
            purpose: "lints",
        },
        ValidateStep {
            program: "gofmt",
            args: &["-l", "."],
            env: &[],
            purpose: "formatting is golden bytes, not a post-pass (empty output required)",
        },
        ValidateStep {
            program: "go",
            args: &["test", "./..."],
            env: &[("CGO_ENABLED", "0")],
            purpose: "unit tests pass",
        },
    ],
    ci_test_steps: CI_TEST_STEPS,
    compose: COMPOSE_OPTS,
    dev: DevCommands {
        rebuild: &[],
        restart: RestartStyle::Restart,
    },
    source_extension: "go",
    sim: SimSupport::Narrow {
        unsupported: unsupported_sim_capabilities,
    },
    // 27UpdatePlan.md M1: see ciac-backend-rust's identical comment —
    // depth and replay-tape support are decoupled fields on purpose.
    sim_replay: false,
};

/// Human-readable, closed list of reasons `ciac sim --target go`
/// cannot yet simulate `ir`, empty when it can — the same gate Rust's
/// own `unsupported_sim_capabilities` (v0.17 M11, full parity
/// 27UpdatePlan.md M4) and TypeScript's (v0.23 M9, full parity M6)
/// compute. 27UpdatePlan.md M7 closes Go's own gate the identical
/// way: `world.go` (`world.go.j2`) now fakes every capability a typed
/// handler can call — `db.get`/`update`/`delete`/`query`/`count`/
/// `delete_where` (in addition to the v0.24 M9 narrow `db.insert`/
/// publish), `cache`, `object_store`, `email`, `search`,
/// `external_http`, and `auth` (`World.AuthVerify`, wired into
/// `auth.go.j2`'s `VerifyToken`) all have a world-guard leaf in
/// `lower.rs` now, so `lower::scan`'s own `unguarded_verbs` list is
/// always empty and `auth` no longer needs a standalone refusal
/// either. Kept as a function (not a bare `Vec::new()` constant) for
/// the identical reason Rust's/TypeScript's own M4/M6 kept theirs:
/// `TargetInfo::sim` needs a `fn(&NormalizedIr) -> Vec<String>`
/// value, and a real function documents the empty list's own
/// reasoning inline rather than leaving a bare literal to look
/// unintentional.
///
/// **A structural note, identical to Rust's/TypeScript's own M4/M6
/// finding:** this does *not* flip `TargetInfo::sim` from
/// `SimSupport::Narrow` to `SimSupport::Full` — `commands.rs`'s
/// `sim_inner` dispatch hardcodes `SimSupport::Full =>
/// sim_drive_python(..)`, so doing that would silently misroute Go-
/// generated projects through Python's driver. Go's `TargetInfo`
/// stays `SimSupport::Narrow` with this function now always
/// returning empty — "full" in observed behavior, not in enum shape.
pub fn unsupported_sim_capabilities(_ir: &NormalizedIr) -> Vec<String> {
    Vec::new()
}

/// Template-facing counterpart of `ciac-sim`'s `WorldReference`.
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

/// Builds the schema `sim_runner.go.j2` passes to `world.New(..)`
/// (27UpdatePlan.md M7) -- without it, `World` falls back to an empty
/// schema and every reference/unique/cascade check silently becomes a
/// no-op, the same gap Rust's own M4 and TypeScript's own M6 caught
/// live against `domain-orders.ciac`. Reuses
/// `ciac_codegen::migrations::snapshot_schema` -- the same reference/
/// unique-column facts the migration DDL itself is built from, so
/// this can never drift from what the real schema actually enforces.
/// Mirrors `ciac-backend-rust`'s/`ciac-backend-ts`'s own
/// `sim_world_tables` exactly, modulo the lowercase `on_delete`
/// spelling TypeScript's own version already established.
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

/// Multi-service counterpart of [`sim_world_tables`]: the system-runner's
/// one shared `world.World` (28UpdatePlan.md M7c) needs every table
/// name -- and every foreign key's `target_table` -- spelled the same
/// namespaced way `lower.rs`'s own `world_table_key` composes them at
/// typed-handler lowering time (`"{service}::{table}"`, via
/// `world.NamespacedTableKey`), or a reference/uniqueness check would
/// silently look up a table the world never registered. Builds a
/// `physical table name -> owning service` map from `ir.tables()`
/// directly (mirrors `ciac-backend-rust`'s own `sim_world_tables_multi`
/// exactly, modulo the lowercase `on_delete` spelling this backend's
/// own `sim_world_tables` already established). A compiler-owned link
/// table (`orders__line_items`) carries no `Table` node of its own; its
/// prefix (`orders`) is always its source table's physical name, so the
/// same map resolves it too.
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

#[derive(Debug, Default)]
pub struct GoBackend;

impl Backend for GoBackend {
    fn id(&self) -> &'static str {
        "go"
    }

    fn description(&self) -> &'static str {
        "Go 1.24+ project using net/http, database/sql, go-redis, and nats.go"
    }

    fn supports(&self, component: &Component) -> bool {
        // M2 adds `Database` for every engine at once: unlike Node's
        // per-driver call-shape divergence (TS's own M2 cost, disclosed
        // at 23UpdatePlan.md M5), Go's `database/sql` interface is
        // engine-agnostic -- `db.go`'s `Open` picks the driver by name,
        // everything downstream (`*sql.DB`, `ExecContext`,
        // `QueryRowContext`) is identical code regardless of engine, so
        // there is no per-engine gating to stage here the way Rust/TS
        // needed for MySQL/Kafka.
        //
        // `crud <Name>[: <Record>];` does NOT expand into Api+Database
        // alone -- `ciac_sema::build::crud` also synthesizes a
        // `Component::Service { name: "<Name>Store", signature: None }`
        // marker node (found empirically: `examples/sqlite-notes.ciac`
        // refused on it once `Database` alone was un-gated). `signature:
        // None` is the documented "classic binding-only handler"
        // discriminant (pre-v0.7, no typed body) -- exactly the shape
        // `resource_store.go.j2`/`resource_api.go.j2`/`service.go.j2`
        // already render unconditionally, with no handler *body* to
        // lower. A typed handler (`signature: Some(_)`) is v0.24 M4's
        // own feature (HostSyntax leaf lowering, landing this
        // milestone) -- the match arm below widens from `signature:
        // None` to `Component::Service { .. }` so both shapes pass,
        // mirroring `ciac-backend-ts`'s own M4 commit exactly (read
        // directly via `git show`, not assumed): every `HostSyntax`
        // leaf is implemented for trait completeness, but the
        // *component* kinds a typed handler body can request
        // (`ObjectStore`/`Email`/`Search`/`ExternalHttp`, plus
        // `Cache`/`Auth`) stay refused below until M6/M7 add their own
        // client wrappers -- `typed-handlers.ciac`/`typed-video.ciac`/
        // `extras-verbs.ciac` stay `CIAC0011`-refused this milestone;
        // `domain-orders.ciac`/`query-verbs.ciac` (db-only) are this
        // milestone's actual proving examples, exactly as they were
        // for TS.
        //
        // M3 (this milestone) adds broker/workers/jobs/channels for
        // both queue engines at once, the same "engine-agnostic
        // component gate, per-engine branch stays inside the template"
        // shape M2 already established for `Database`: `Queue` gates
        // the capability instance declaration itself
        // (`use { queue NATS/Kafka; }`), `Stream` gates `stream <Name>:
        // <Record>;`, `Worker`/`Job`/`Channel` gate the pipeline units
        // (`worker`/`events`/`job`/`channel` declarations -- `events`
        // lowers to a `Component::Worker` node too, split into
        // `ConsumerCtx` only at the codegen model layer), and
        // `Scheduler`/`Realtime` gate the `use { scheduler jobs Cron;
        // realtime live WebSocket/SSE; }` capability instances a
        // `job`/`channel` declaration needs present.
        //
        // M6 added `Auth` for both schemes at once (golang-jwt/v5 +
        // keyfunc/v3's own JWKS caching handle HS256 and RS256 through
        // the same `jwt.ParseWithClaims` call shape, branching only on
        // which `jwt.Keyfunc` gets passed in -- `auth.go.j2`'s own
        // `c.auth_scheme` conditional, not a gate here).
        //
        // M7 (this milestone) closes out the ontology: `Cache` (a
        // lazily-parsed `*redis.Client` on `AppState`, wired straight
        // into `resource_store.go.j2`'s own read-through-cache CRUD --
        // no dedicated wrapper package needed, matching Rust's/TS's own
        // choice to keep it inline rather than its own module),
        // `ObjectStore`/`Email`/`Search`/`ExternalHttp` (one wrapper
        // package per capability *kind*, matching Rust's own module
        // shape -- `internal/objectstore`/`internal/email`/
        // `internal/search`/`internal/httpclients`), `Users` (no Go-
        // specific code at all: `ciac-codegen::model`'s own dev-issuer-
        // default computation is already target-neutral, confirmed the
        // same way TS's own M7 confirmed it), and `Logging`/`Metrics`/
        // `Tracing` (OTel end-to-end plus a `/metrics` endpoint --
        // `observability.go.j2`'s own doc comment named this milestone
        // as where that wiring would land). Every `Component` variant
        // reaches this backend now, so, like Rust's/TS's own `supports`
        // at full parity, there is nothing left to gate on.
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
        env.add_filter("go_type", filters::go_type);
        env.add_filter("go_db_type", filters::go_db_type);
        env.add_filter("go_zero", filters::go_zero);
        env.add_filter("go_validate_tag", filters::go_validate_tag);
        env.add_filter("go_pascal", filters::go_pascal);

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
            // No system-level README template -- found live: M3's own
            // wider `supports()` gate is what first makes a
            // multi-service system reachable for Go at all (M1/M2's
            // narrower gate never had one in the conformance harness's
            // supported set), and `ciac-backend-ts` already settled
            // this exact question by not emitting one either; matching
            // that precedent instead of writing a new template neither
            // other target's own users asked for.
            project.add_file(
                "docker-compose.yml",
                ciac_codegen::compose::render_system_compose(&model, &COMPOSE_OPTS)?,
            );
            project.add_file(
                "openapi.json",
                serde_json::to_string_pretty(&ciac_codegen::openapi::build_index(&model))
                    .map_err(|e| BackendError::Other(e.to_string()))?,
            );
            // 28UpdatePlan.md M7c: the `sim-shared` module (one
            // `world.World` type every service and the system-runner
            // import identically, resolved via `replace` -- see
            // `emit_service`'s own `!multi` gate and `go.mod.j2`'s own
            // `{% if multi %}` block for why this is needed at all: Go's
            // `internal/` package-visibility rule makes each service's
            // own private `internal/world` copy unimportable from
            // outside that service's own module tree). `world.go.j2`
            // has no per-service template variables (it never
            // references `c.*`), so this is the exact same rendered
            // content every service's own single-service `internal/
            // world/world.go` would have gotten.
            project.add_file("sim-shared/go.mod", SIM_SHARED_GO_MOD);
            project.add_file(
                "sim-shared/world/world.go",
                gofmt(&env.get_template("world.go.j2")?.render(context! {})?)?,
            );
            // The system-runner module: drives `ciac sim` scenarios
            // across every service in this system through the one
            // shared world above (M7d wires the actual driver).
            project.add_file("system-runner/go.mod", system_runner_go_mod(&env, &model)?);
            project.add_file("system-runner/go.sum", GO_SUM);
            let sim_needs_context = model
                .services
                .iter()
                .any(|ctx| !ctx.jobs.is_empty() || ctx.workers.iter().any(|w| !w.steps.is_empty()));
            project.add_file(
                "system-runner/main.go",
                gofmt(
                    &env.get_template("system_sim_runner.go.j2")?
                        .render(context! {
                            services => model.services,
                            sim_world_tables => sim_world_tables_multi(ir),
                            sim_needs_context => sim_needs_context,
                        })?,
                )?,
            );
            project.notes.push(
                "multi-service system: each directory is a complete project; \
                 `docker compose up` runs them all together"
                    .to_owned(),
            );
        } else {
            project.notes.push(
                "run the API with `go run ./cmd/api`, or `docker compose up` for the full stack"
                    .to_owned(),
            );
        }
        Ok(project)
    }
}

/// Emits one deployable project under `prefix`. Multi-service systems
/// get their compose file/README/index-openapi at the root instead of
/// per service (mirroring every other backend's `emit_service`).
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
    // `.go` files specifically route through the real `gofmt` binary
    // (Pillar "determinism and supply chain": "generated code is
    // emitted gofmt-canonical... formatting is golden bytes, not a
    // post-pass" — read literally, that rules out *committing*
    // unformatted bytes and hoping `ciac verify`'s own `gofmt -l`
    // check happens to agree, not the mechanism proving it. A Jinja
    // template has no notion of gofmt's struct-field column alignment
    // (which depends on the *rendered* width of every sibling field,
    // not any one template's local context) or its empty-composite-
    // literal collapse (`T{\n}` -> `T{}`); hand-simulating gofmt's own
    // algorithm in the template layer is exactly the kind of
    // "getting subtly wrong forever" risk `docs/backends.md`'s
    // determinism rules exist to avoid. Every other target's
    // formatter (`ruff format`, `rustfmt` implicitly via already-
    // canonical hand-authored templates) has an equivalent real
    // answer; Go's is that this backend leans on the Go toolchain's
    // own formatter to render into it — a disclosed, narrow
    // dependency: `gofmt` must be on `PATH` to run `--target go`
    // generation at all, not only to *validate* its output.
    let render_go = |name: &str, extra: minijinja::Value| -> Result<String, BackendError> {
        gofmt(&render(name, extra)?)
    };
    let empty = || context! {};
    let at = |path: &str| format!("{prefix}{path}");

    project.add_file(at("go.mod"), render("go.mod.j2", empty())?);
    // 28UpdatePlan.md M7c: the one non-`internal` package this service
    // exposes to the system's own system-runner module -- found live
    // (`go build`'s "use of internal package ... not allowed" refusal)
    // that Go's directory-based `internal/` visibility rule blocks a
    // *separate* module (even one reached through a `go.mod` `replace`
    // directive) from importing anything under `internal/` at all, not
    // only `internal/world` -- see `templates/simbridge.go.j2`'s own
    // doc comment. Every symbol it re-exports already exists
    // unconditionally (`config.FromEnv`/`state.New`) or is itself
    // already gated inside the template, so this is emitted
    // unconditionally in multi mode, mirroring `world.go`'s own "any
    // service might be a routed-call target" reasoning.
    if multi {
        project.add_file(
            at("simbridge/simbridge.go"),
            render_go("simbridge.go.j2", empty())?,
        );
    }
    project.add_file(at("go.sum"), GO_SUM);
    project.add_file(at("README.md"), render("README.md.j2", empty())?);
    project.add_file(at("Dockerfile"), render("Dockerfile.j2", empty())?);
    project.add_file(at(".dockerignore"), "data\n");
    if !multi {
        project.add_file(
            at("docker-compose.yml"),
            ciac_codegen::compose::render_service_compose(ctx, &COMPOSE_OPTS)?,
        );
    }
    // `go:embed` cannot reach outside the embedding file's own
    // directory (no `..` in embed patterns, unlike Rust's
    // `include_str!("../../openapi.json")`), so `cmd/api/main.go`
    // needs its own colocated copy to embed at build time. The
    // project-root `openapi.json` stays the canonical one every other
    // target and the conformance harness's C3 check (byte-identical
    // `openapi.json` content, matched by an `ends_with("openapi.json")`
    // path filter) reads — the embed copy is deliberately named
    // `apidoc.json` (does not end with the literal substring
    // "openapi.json"), so it never matches that filter and never
    // needs C3 to learn a Go-specific exception.
    // Disclosed here rather than silently worked around with a runtime
    // `os.ReadFile`, which would reintroduce a working-directory
    // dependency the distroless runtime image doesn't have.
    // Written unconditionally, multi-service included -- found live:
    // every other target (confirmed against Rust's own `lib.rs`, which
    // never gates this behind `!multi` either) emits a per-service
    // `openapi.json` even inside a multi-service system, separate from
    // the root-level *index* `generate()` writes across every service.
    // Go's own `!multi` gate here was a latent M1 gap invisible until
    // M3's wider `supports()` first made a multi-service example
    // (`audited-crud.ciac`) reachable at all -- caught by C3's
    // byte-identical-path-set conformance check, not by inspection.
    let openapi_doc = serde_json::to_string_pretty(&ciac_codegen::openapi::build_document(ctx))
        .map_err(|e| BackendError::Other(e.to_string()))?;
    project.add_file(at("openapi.json"), openapi_doc.clone());
    project.add_file(at("cmd/api/apidoc.json"), openapi_doc);

    project.add_file(
        at("internal/config/config.go"),
        render_go("config.go.j2", empty())?,
    );
    project.add_file(
        at("internal/state/state.go"),
        render_go("state.go.j2", empty())?,
    );
    project.add_file(
        at("internal/state/state_test.go"),
        render_go("state_test.go.j2", empty())?,
    );
    project.add_file(
        at("internal/observability/observability.go"),
        render_go("observability.go.j2", empty())?,
    );
    project.add_file(
        at("internal/httpx/httpx.go"),
        render_go("httpx.go.j2", empty())?,
    );
    if ctx.has_auth {
        project.add_file(
            at("internal/auth/auth.go"),
            render_go("auth.go.j2", empty())?,
        );
    }
    // v0.24 M7: one wrapper package per ontology capability *kind* (not
    // per named instance, matching Rust's own module shape) -- state.
    // go.j2's own per-instance loop resolves each named instance's
    // field against these package types.
    if ctx.has_object_store {
        project.add_file(
            at("internal/objectstore/objectstore.go"),
            render_go("object_store.go.j2", empty())?,
        );
    }
    if ctx.has_email {
        project.add_file(
            at("internal/email/email.go"),
            render_go("email.go.j2", empty())?,
        );
    }
    if ctx.has_search {
        project.add_file(
            at("internal/search/search.go"),
            render_go("search.go.j2", empty())?,
        );
    }
    if ctx.has_external_http {
        project.add_file(
            at("internal/httpclients/http_clients.go"),
            render_go("http_clients.go.j2", empty())?,
        );
    }
    // One typed HTTP client per downstream service this service
    // `call`s -- `_steps.go.j2`'s own `call` step arm already expects
    // `clients.New{ClassName}(st)`. No `mod.rs`-equivalent aggregator
    // is needed the way Rust's `src/clients/mod.rs` is: every `.go`
    // file under `internal/clients/` sharing `package clients` forms
    // the package automatically.
    for target in &ctx.call_targets {
        project.add_file(
            at(&format!("internal/clients/{}.go", target.module)),
            render_go(
                "client.go.j2",
                context! { t => target, caller => ctx.service_name },
            )?,
        );
    }

    if !ctx.records.is_empty() {
        project.add_file(
            at("internal/schemas/schemas.go"),
            render_go("schemas.go.j2", empty())?,
        );
    }

    if ctx.has_db {
        project.add_file(at("internal/db/db.go"), render_go("db.go.j2", empty())?);
    }
    if ctx.has_queue {
        project.add_file(
            at("internal/queue/queue.go"),
            render_go("queue.go.j2", empty())?,
        );
    }
    // v0.24 M9: the simulation world -- only for programs with
    // something it can actually fake, the same gate Rust's/
    // TypeScript's own `world.rs`/`world.ts` emission (v0.17 M11 /
    // v0.23 M9, broadened at 27UpdatePlan.md M4/M6) uses. 27UpdatePlan.md
    // M7 broadens this from `has_db or has_queue` to the full 8-
    // condition check, matching TypeScript's own M6 fix, now that
    // `world.go` fakes every capability, not just db.insert/publish.
    // 28UpdatePlan.md M7c: `|| !ctx.call_targets.is_empty()` closes a
    // gap found live -- a service reachable only via a routed `call`
    // from another service (no local db/queue/etc. of its own) still
    // needs a world instance to register its handlers against, the
    // same fix Rust's/TypeScript's own M6a/M7a checkpoints already
    // made to their analogous gates.
    // `sim_needs_schemas` tells the runner template whether any
    // worker's typed payload needs the `internal/schemas` import at
    // all.
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
        // 28UpdatePlan.md M7c: a multi-service system's own services
        // import the shared `sim-shared/world` module (emitted once per
        // system by `GoBackend::generate`, below) instead of each
        // vendoring a private copy under `internal/world` -- N private
        // copies of the identical source text are still N *nominally
        // distinct* Go types (Go's own `internal/` visibility rule also
        // makes a private copy unimportable across module boundaries
        // regardless), and a system-runner module depending on every
        // service needs exactly one `world.World` type all of them
        // share. Single-service projects are untouched: still self-
        // contained, still emitted at `internal/world/world.go`.
        if !multi {
            project.add_file(
                at("internal/world/world.go"),
                render_go("world.go.j2", empty())?,
            );
        }
        let sim_needs_schemas = ctx.workers.iter().any(|w| w.payload.is_some());
        // `context.Background()` only appears inside the runner's
        // `drain`/`advance` bodies, and only for a worker with a
        // non-empty pipeline (`drain`'s own `{% if worker.steps %}`
        // gate) or any job (`advance`'s per-job block is unconditional)
        // -- found live: a db/queue-only program with zero such workers
        // (e.g. every `crud`-only example that still declares a queue)
        // left `context` imported and unused, a `go vet` failure.
        let sim_needs_context =
            !ctx.jobs.is_empty() || ctx.workers.iter().any(|w| !w.steps.is_empty());
        project.add_file(
            at("cmd/sim_runner/main.go"),
            render_go(
                "sim_runner.go.j2",
                context! {
                    sim_needs_schemas => sim_needs_schemas,
                    sim_needs_context => sim_needs_context,
                    sim_world_tables => sim_world_tables(ir),
                },
            )?,
        );
    }
    if !ctx.resources.is_empty() || !ctx.tables.is_empty() {
        let resource_needs_decode = ctx.resources.iter().any(|r| r.record.is_some());
        // Every generic (no-record) resource's `Data` field is a raw
        // JSON blob; the presence-check decoder needs it too. A
        // `table`'s own row struct needs it exactly when one of its
        // fields is `Json`-typed (`go_db_type` spells that field
        // `json.RawMessage`) -- found live: no v0.24 M1-M3 example
        // ever put a `Json` field on a `table`, so this arm of the
        // condition was structurally missing (an "undefined: json"
        // `go vet` failure) until v0.24 M4's own typed-handler examples
        // first did.
        let needs_json = ctx.resources.iter().any(|r| r.record.is_none())
            || resource_needs_decode
            || ctx
                .tables
                .iter()
                .any(|t| t.record.fields.iter().any(|f| f.is_json));
        // A CRUD resource's `<Name>In` payload struct carries the
        // record's own enum fields at their real, exported enum type
        // (`schemas.VideoStatus`), not the row struct's own DB-string
        // spelling (`go_db_type` correctly keeps that bare `string`)
        // -- the enum type itself is declared in `internal/schemas`,
        // a different package than `models`, so any resource with an
        // enum field needs the import. Found live: `filters::go_type`
        // returns the bare, unqualified enum name (correct *within*
        // `schemas.go`, where every sibling enum is already in scope
        // bare) -- `typed-video.ciac`/`routed-media.ciac` (both
        // `CIAC0011`-refused until this milestone's own `Cache`
        // un-gating) were the first examples to reach a `crud`
        // resource with an enum field at all, an "undefined:
        // VideoStatus" `go vet` failure this flag closes.
        let needs_schemas = ctx.resources.iter().any(|r| {
            r.record
                .as_ref()
                .is_some_and(|rec| rec.fields.iter().any(|f| f.is_enum))
        });
        project.add_file(
            at("internal/models/models.go"),
            render_go(
                "models.go.j2",
                context! {
                    needs_json => needs_json,
                    resource_needs_decode => resource_needs_decode,
                    needs_schemas => needs_schemas,
                },
            )?,
        );
    }
    for resource in &ctx.resources {
        project.add_file(
            at(&format!("internal/services/{}.go", resource.store_module)),
            render_go("resource_store.go.j2", context! { resource => resource })?,
        );
        project.add_file(
            at(&format!("internal/routes/{}_api.go", resource.plural)),
            render_go("resource_api.go.j2", context! { resource => resource })?,
        );
    }

    // `crud <Name>;` expands into `Api`/`Database` components only
    // (verified empirically against `examples/sqlite-notes.ciac` — no
    // `Service` node), so a resource-only program still needs a
    // router even when `ctx.apis` itself is empty. A jobs-only program
    // (e.g. `scheduled-cleanup.ciac`, found live: M3's first example
    // with zero apis/resources/channels) has none of these, so there
    // is no `internal/routes` package at all -- `main.go.j2` branches
    // on this same `has_routes` value to fall back to a bare
    // `http.NewServeMux()` with just `/health`.
    let has_routes = !ctx.apis.is_empty() || !ctx.resources.is_empty() || !ctx.channels.is_empty();
    if has_routes {
        project.add_file(
            at("internal/routes/router.go"),
            render_go("router.go.j2", empty())?,
        );
    }
    for api in &ctx.apis {
        project.add_file(
            at(&format!("internal/routes/{}_api.go", api.snake)),
            render_go("route_api.go.j2", context! { api => api })?,
        );
    }
    for channel in &ctx.channels {
        project.add_file(
            at(&format!("internal/routes/channel_{}.go", channel.snake)),
            render_go("channel.go.j2", context! { channel => channel })?,
        );
    }

    // Classic (`signature: None`) handler seeded stubs -- one struct
    // per `ctx.services` entry, business logic left to the user
    // (`Handle` starts as a TODO). Distinct from `ctx.resources`'
    // `resource_store.go.j2` files, which also live under
    // `internal/services/` but are compiler-owned CRUD persistence,
    // not user business logic.
    for service in &ctx.services {
        project.add_seeded_file(
            at(&format!("internal/services/{}.go", service.module)),
            render_go("service.go.j2", context! { service => service })?,
        );
    }

    // v0.24 M4: typed handlers (`Component::Service { signature:
    // Some(hir), .. }`). Mirrors `ciac-backend-ts`'s own dispatch:
    // inline bodies lower straight from the HIR and are compiler-owned
    // (`internal/logic/`); `extern` gets a typed stub in
    // `internal/services/` like classic handlers, since it's the same
    // "implement this yourself" contract -- both are plain functions
    // (not struct+constructor, unlike classic handlers), matching the
    // `_steps.go.j2` invocation convention `emit_step` uses for a
    // typed-handler `handler` pipeline step.
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
        let content = render_go(
            "logic.go.j2",
            context! { handler => minijinja::Value::from_serialize(&handler) },
        )?;
        if hir.body.is_some() {
            project.add_file(
                at(&format!("internal/logic/{}.go", handler.module)),
                content,
            );
        } else {
            project.add_seeded_file(
                at(&format!("internal/services/{}.go", handler.module)),
                content,
            );
        }
    }

    for worker in &ctx.workers {
        project.add_file(
            at(&format!("internal/workers/{}.go", worker.snake)),
            render_go("worker.go.j2", context! { worker => worker })?,
        );
    }
    for job in &ctx.jobs {
        project.add_file(
            at(&format!("internal/workers/{}.go", job.snake)),
            render_go("job.go.j2", context! { job => job })?,
        );
    }
    for consumer in &ctx.consumers {
        project.add_file(
            at(&format!("internal/workers/{}.go", consumer.snake)),
            render_go("consumer.go.j2", context! { consumer => consumer })?,
        );
    }
    if !ctx.workers.is_empty() || !ctx.jobs.is_empty() || !ctx.consumers.is_empty() {
        project.add_file(
            at("cmd/workers/main.go"),
            render_go("workers_main.go.j2", empty())?,
        );
    }

    project.add_file(
        at("cmd/api/main.go"),
        render_go("main.go.j2", context! { has_routes => has_routes })?,
    );

    // v0.24 M6: scope-enforcement behavioral test. `26UpdatePlan.md`
    // M5 widened this from JWT-only to both schemes: the file's own
    // no-token/malformed-token cases are scheme-agnostic (gated inside
    // the template on `HasAuthStep`/`HasAuth`), and its JWT-only
    // `bearer`/`bearerExp` helpers plus their wrong_scope/
    // correct_scope/expired_token blocks stay gated on
    // `c.auth_scheme == "jwt"` inside the template. OAuth2 gets the
    // scheme-specific equivalent via the real-RS256 rig below.
    if !ctx.scopes.is_empty() {
        project.add_file(
            at("internal/routes/scope_test.go"),
            render_go("scope_test.go.j2", empty())?,
        );
    }
    // The no-infra OAuth2 rig (`26UpdatePlan.md` M5): real RS256
    // signing against an in-process JWKS stub, gated the same way the
    // scope suite is gated on `c.scopes` above. `oauth_stub_test.go`
    // owns this package's `TestMain` (`internal/auth`'s JWKS fetch is
    // cached behind a package-level `sync.Once`, so only one stub per
    // test binary may ever set `OAUTH_ISSUER`).
    if ctx.auth_scheme == "oauth2" && !ctx.scopes.is_empty() {
        project.add_file(
            at("internal/routes/oauth_stub_test.go"),
            render_go("oauth_stub_test.go.j2", empty())?,
        );
        project.add_file(
            at("internal/routes/oauth_rig_test.go"),
            render_go("oauth_rig_test.go.j2", empty())?,
        );
    }

    Ok(())
}

/// Formats Go source through the real `gofmt` binary — see
/// `emit_service`'s `render_go` for why this is a deliberate,
/// disclosed dependency rather than a template-layer approximation.
fn gofmt(src: &str) -> Result<String, BackendError> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let mut child = Command::new("gofmt")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            BackendError::Other(format!(
                "`gofmt` not found on PATH ({e}) — the Go toolchain must be \
                 installed to generate `--target go` output, not only to validate it"
            ))
        })?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(src.as_bytes())
        .map_err(|e| BackendError::Other(format!("writing to gofmt: {e}")))?;
    let output = child
        .wait_with_output()
        .map_err(|e| BackendError::Other(format!("running gofmt: {e}")))?;
    if !output.status.success() {
        return Err(BackendError::Other(format!(
            "gofmt rejected generated source (this is a codegen bug, not a \
             user error): {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    String::from_utf8(output.stdout).map_err(|e| BackendError::Other(format!("gofmt output: {e}")))
}

/// The shared `sim-shared` module's own `go.mod` (28UpdatePlan.md
/// M7c): no third-party requirement of its own -- `world.go.j2`
/// depends on nothing beyond the Go standard library -- mirroring
/// Rust's `SIM_SHARED_CARGO_TOML`/TypeScript's `system_runner_
/// package_json`'s own minimal-dependency finding for the same reason.
const SIM_SHARED_GO_MOD: &str = "module sim-shared\n\ngo 1.24.0\n\ntoolchain go1.24.7\n";

/// The system-runner module's own `go.mod` (28UpdatePlan.md M7c):
/// `replace` directives pointing `sim-shared` and every service in
/// this system at their real, local directories, by their real module
/// names. Renders the real `go.mod.j2` template (with `c.package` set
/// to `"system-runner"`) rather than hand-writing a second copy of its
/// dependency list -- found live: `go build`'s own default `-mod=
/// readonly` mode refuses to resolve `system-runner/main.go`'s own
/// transitive imports (pulled in through every service's own
/// `simbridge` -- pgx, redis, franz-go, aws-sdk-s3, ...) unless every
/// one of them is already present in `go.mod`'s own `require` block
/// (confirmed with `go build -mod=mod`, which auto-populated exactly
/// the fixed indirect set `go.mod.j2`'s own unconditional require
/// blocks already declare for every service) -- `go.mod.j2` already
/// declares that full, fixed set unconditionally (no per-capability
/// gating), the same "every possible provider, always" shape every
/// service's own `go.mod` already carries, so reusing it here needs no
/// per-system dependency computation at all. The per-service `require`/
/// `replace` lines this template doesn't know about are appended as
/// plain text after it, mirroring `ciac-backend-rust`'s own
/// `system_runner_cargo_toml`/`ciac-backend-ts`'s own `system_runner_
/// package_json` for the one part that *is* genuinely per-system.
fn system_runner_go_mod(
    env: &minijinja::Environment<'_>,
    model: &context::SystemModel,
) -> Result<String, BackendError> {
    let base = env.get_template("go.mod.j2")?.render(context! {
        c => context! { package => "system-runner" },
        multi => true,
    })?;
    let mut out = base.trim_end().to_owned();
    out.push('\n');
    for ctx in &model.services {
        out.push_str(&format!("\nrequire {} v0.0.0\n", ctx.package));
        out.push_str(&format!("\nreplace {} => ../{}\n", ctx.package, ctx.dir));
    }
    Ok(out)
}

/// The full, fixed transitive dependency pin for every Go module this
/// backend's generated projects will ever import across the whole
/// `24UpdatePlan.md` arc, computed once against the real module proxy
/// and checked in verbatim (mirroring `ciac-backend-ts`'s own
/// `package-lock.json.j2`, and the same reasoning: `go build`/`go
/// test` run with `GOFLAGS=-mod=readonly`, so `go.sum` must already
/// contain every entry the build graph needs — a per-capability
/// *conditional* `go.sum` would need one precomputed hash set per
/// capability combination, whereas a fixed superset is always
/// sufficient and costs nothing unused entries don't get imported).
/// Extended milestone by milestone as new dependencies are actually
/// introduced (M1: `validator`, `goleak` only) rather than
/// front-loaded speculatively.
const GO_SUM: &str = include_str!("../templates/go.sum.pin");

#[cfg(test)]
mod tests {
    use super::*;

    fn ping_ir() -> NormalizedIr {
        let src = "service Ping;\n\nrecord Message {\n    id: Uuid;\n    text: String;\n}\n\napi Echo: Message {\n    method: POST;\n    path: \"/echo\";\n}\n\npipeline Echo: Return;\n";
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = ciac_diagnostics::Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()))
    }

    #[test]
    fn generates_ping_parity_file_set() {
        let ir = ping_ir();
        let backend = GoBackend;
        let project = backend
            .generate(&ir, &GenOptions::default())
            .expect("go generates");
        let paths: Vec<&str> = project.files().map(|(p, _)| p).collect();
        for expect in [
            "go.mod",
            "go.sum",
            "README.md",
            "Dockerfile",
            "docker-compose.yml",
            "openapi.json",
            "cmd/api/apidoc.json",
            "internal/config/config.go",
            "internal/state/state.go",
            "internal/state/state_test.go",
            "internal/observability/observability.go",
            "internal/httpx/httpx.go",
            "internal/schemas/schemas.go",
            "internal/routes/router.go",
            "internal/routes/echo_api.go",
            "cmd/api/main.go",
        ] {
            assert!(paths.contains(&expect), "missing {expect} in {paths:?}");
        }
    }

    #[test]
    fn supports_apis() {
        let backend = GoBackend;
        assert!(backend.supports(&Component::Api {
            name: "X".to_owned(),
            request: None,
            config: ciac_ir::ApiConfig {
                method: ciac_ir::HttpMethod::Get,
                path: Some("/x".to_owned()),
                scope: None,
            },
        }));
    }

    #[test]
    fn supports_broker_workers_jobs_channels_at_m3() {
        let backend = GoBackend;
        assert!(backend.supports(&Component::Queue {
            name: "Q".to_owned(),
            engine: ciac_ir::QueueEngine::Nats,
        }));
        assert!(backend.supports(&Component::Worker {
            name: "W".to_owned(),
            config: Default::default(),
        }));
        assert!(backend.supports(&Component::Job {
            name: "J".to_owned(),
            config: ciac_ir::JobConfig {
                schedule: "0 3 * * *".to_owned(),
                catch_up: false,
            },
        }));
        assert!(backend.supports(&Component::Channel {
            name: "C".to_owned(),
            config: ciac_ir::ChannelConfig {
                path: "/channels/c".to_owned(),
            },
        }));
        assert!(backend.supports(&Component::Scheduler {
            name: "S".to_owned(),
            provider: ciac_ir::SchedulerProvider::Cron,
        }));
        assert!(backend.supports(&Component::Realtime {
            name: "R".to_owned(),
            provider: ciac_ir::RealtimeProvider::WebSocket,
        }));
    }

    fn kafka_pipeline_ir() -> NormalizedIr {
        let src = "service Clickstream;\n\nuse {\n    queue Kafka;\n}\n\nrecord Click {\n    id: Uuid;\n    page: String;\n}\n\nstream Clicks: Click;\n\napi Ingest: Click {\n    method: POST;\n    path: \"/clicks\";\n}\npipeline Ingest: publish Clicks -> Return;\n\nworker Enrich on Clicks;\npipeline Enrich: EnrichClick;\n";
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = ciac_diagnostics::Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()))
    }

    #[test]
    fn generates_worker_pipeline_file_set() {
        let ir = kafka_pipeline_ir();
        let backend = GoBackend;
        let project = backend
            .generate(&ir, &GenOptions::default())
            .expect("go generates");
        let paths: Vec<&str> = project.files().map(|(p, _)| p).collect();
        for expect in [
            "internal/queue/queue.go",
            "internal/services/enrich_click.go",
            "internal/workers/enrich.go",
            "cmd/workers/main.go",
        ] {
            assert!(paths.contains(&expect), "missing {expect} in {paths:?}");
        }
    }

    #[test]
    fn target_info_is_populated() {
        let backend = GoBackend;
        let info = backend.target_info();
        assert_eq!(info.project_marker, "go.mod");
        assert_eq!(info.source_extension, "go");
        assert!(matches!(info.sim, SimSupport::Narrow { .. }));
        assert_eq!(info.validate.len(), 4);
    }
}
