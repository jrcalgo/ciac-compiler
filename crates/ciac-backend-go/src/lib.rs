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
    sim: SimSupport::None {
        reason: "Go simulation support lands at 24UpdatePlan.md M9",
    },
};

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
        // `resource_store.go.j2`/`resource_api.go.j2` already render
        // unconditionally from `ctx.resources`, with no handler *body*
        // to lower. A typed handler (`signature: Some(_)`) is a
        // different, much bigger feature (HostSyntax leaf lowering)
        // that stays gated until M4. Bare classic handlers *not* tied
        // to a `crud` (seeded-stub `ctx.services` entries) are also not
        // yet implemented and stay gated by this same narrow condition
        // -- confirmed by generating every example with this gate: only
        // `crud`-synthesized Store nodes are ever `signature: None`
        // among currently Api+Database-reachable programs.
        matches!(
            component,
            Component::Api { .. }
                | Component::Database { .. }
                | Component::Service {
                    signature: None,
                    ..
                }
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
            emit_service(&env, ctx, model.multi, &prefix, &mut project)?;
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
    let openapi_doc = serde_json::to_string_pretty(&ciac_codegen::openapi::build_document(ctx))
        .map_err(|e| BackendError::Other(e.to_string()))?;
    if !multi {
        project.add_file(at("openapi.json"), openapi_doc.clone());
    }
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

    if !ctx.records.is_empty() {
        project.add_file(
            at("internal/schemas/schemas.go"),
            render_go("schemas.go.j2", empty())?,
        );
    }

    if ctx.has_db {
        project.add_file(at("internal/db/db.go"), render_go("db.go.j2", empty())?);
    }
    if !ctx.resources.is_empty() || !ctx.tables.is_empty() {
        let resource_needs_decode = ctx.resources.iter().any(|r| r.record.is_some());
        // Every generic (no-record) resource's `Data` field is a raw
        // JSON blob; every `table` row struct needs no extra import.
        // json.RawMessage itself needs `encoding/json` whenever it's
        // used *or* whenever the presence-check decoder needs it.
        let needs_json = ctx.resources.iter().any(|r| r.record.is_none()) || resource_needs_decode;
        project.add_file(
            at("internal/models/models.go"),
            render_go(
                "models.go.j2",
                context! { needs_json => needs_json, resource_needs_decode => resource_needs_decode },
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
    // router even when `ctx.apis` itself is empty.
    if !ctx.apis.is_empty() || !ctx.resources.is_empty() {
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

    project.add_file(at("cmd/api/main.go"), render_go("main.go.j2", empty())?);
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
    fn supports_apis_only_at_m1() {
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
    fn target_info_is_populated() {
        let backend = GoBackend;
        let info = backend.target_info();
        assert_eq!(info.project_marker, "go.mod");
        assert_eq!(info.source_extension, "go");
        assert!(matches!(info.sim, SimSupport::None { .. }));
        assert_eq!(info.validate.len(), 4);
    }
}
