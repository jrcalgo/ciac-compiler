//! Java (Spring Boot 3.x on Java 21, Spring MVC + virtual threads)
//! code-generation backend (`25UpdatePlan.md`).
//!
//! Maps the CIaC ontology onto production-standard Spring components:
//!
//! | CIaC          | Java                                        |
//! |---------------|-----------------------------------------------|
//! | API           | Spring MVC `@RestController`                   |
//! | Service       | plain class with a `handle` method             |
//! | Worker        | jnats `Dispatcher` / spring-kafka listener      |
//! | Database      | `JdbcClient` + Postgres/MySQL/sqlite-jdbc       |
//! | Cache         | spring-data-redis + Lettuce                     |
//! | Queue         | io.nats:jnats / spring-kafka                    |
//! | Auth (JWT)    | spring-boot-starter-oauth2-resource-server      |
//! | Logging       | SLF4J + Logback + logstash-logback-encoder      |
//! | Metrics       | Micrometer, `/actuator/prometheus`              |
//!
//! The generated project is buildable without any infrastructure
//! running: every provider client bean is `@Lazy` (or lazily
//! constructed by the library itself, as HikariCP/Lettuce are) — M1's
//! own `NoInfraBootTest` proves this from day one (Pillar 4's magic
//! detector), the same bar v0.17 M11 retrofitted onto Rust and every
//! later target has built in from milestone one.
//!
//! **M1 scope note, disclosed:** `jakarta.validation`/hibernate-
//! validator (Pillar 1's own table row) is deferred past this
//! milestone — `ping.ciac`'s own `Message.id: Uuid` format constraint
//! is enforced with a plain `Schemas.requireUuid` check (mirroring the
//! *shape* of Go's `validate:"uuid4"` tag without the dependency),
//! keeping `pom.xml` to exactly one Spring starter for the ping-parity
//! slice. Bean Validation lands whenever a later milestone's own
//! record needs a constraint this manual check can't express;
//! recorded here rather than pulled in speculatively.

pub mod filters;
pub mod lower;

use ciac_codegen::model as context;
use ciac_codegen::{
    Backend, BackendError, DevCommands, GenOptions, GeneratedProject, RestartStyle, SimSupport,
    TargetInfo, ValidateStep,
};
use ciac_ir::{Component, NormalizedIr};
use include_dir::{include_dir, Dir};
use minijinja::context;
use serde::Serialize;

static TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

/// This backend's compose-file divergences (mirroring every other
/// backend's own `COMPOSE_OPTS`): JDBC accepts a `jdbc:postgresql://`
/// URL directly (the JDBC containment note — user/password stay
/// separate `spring.datasource.*` properties, assembled from the same
/// discrete env vars compose already emits); the workers "binary" is
/// the same fat jar run with `--spring.profiles.active=workers`
/// (Pillar 3's one-artifact-two-profiles decision) rather than a
/// second compiled binary.
const COMPOSE_OPTS: ciac_codegen::compose::BackendComposeOpts =
    ciac_codegen::compose::BackendComposeOpts {
        db_url_scheme: "jdbc:postgresql",
        workers_command: r#"["java","-jar","/app/app.jar","--spring.profiles.active=workers"]"#,
        mysql_url_scheme: "jdbc:mysql",
        sqlite_url_prefix: "jdbc:sqlite:data/",
        sqlite_url_suffix: "",
        data_mount: "/data",
    };

/// The literal CI test-step YAML for this target (`25UpdatePlan.md`
/// Pillar 8): `setup-java@v4` (temurin 21, maven cache) then the one
/// `./mvnw -q -B verify` invocation `validate` runs locally too — the
/// startup tax paid once, per Pillar 8's own decision.
const CI_TEST_STEPS: &str = "      - uses: actions/setup-java@v4\n        with:\n          distribution: temurin\n          java-version: \"21\"\n          cache: maven\n      - run: ./mvnw -q -B verify\n";

/// `0001_slug.sql` -> `V0001__slug.sql` — the first non-identity
/// consumer of `TargetInfo::migration_filename` (every other current
/// target keeps the identity mapping); Flyway requires this exact
/// `V<version>__<description>.sql` shape.
fn flyway_migration_filename(seq: u32, slug: &str) -> String {
    format!("V{seq:04}__{slug}.sql")
}

/// This target's whole CLI/CI/compose/dev-loop/sim integration surface
/// (`22UpdatePlan.md` Pillar 1's factory contract), reached through
/// the `Backend` trait instead of a per-call-site
/// `match target { "python" => .., .. }`.
static TARGET_INFO: TargetInfo = TargetInfo {
    project_marker: "pom.xml",
    migrations_dir: "src/main/resources/db/migration",
    migration_filename: flyway_migration_filename,
    validate: &[ValidateStep {
        program: "./mvnw",
        args: &["-q", "-B", "verify"],
        env: &[],
        purpose: "compiles, formats (Spotless), and tests in one invocation (Pillar 8)",
    }],
    ci_test_steps: CI_TEST_STEPS,
    compose: COMPOSE_OPTS,
    dev: DevCommands {
        rebuild: &[ValidateStep {
            program: "./mvnw",
            args: &["-q", "-B", "-DskipTests", "package"],
            env: &[],
            purpose: "rebuild before restart",
        }],
        restart: RestartStyle::Restart,
    },
    source_extension: "java",
    sim: SimSupport::Narrow {
        unsupported: unsupported_sim_capabilities,
    },
    // 27UpdatePlan.md M1: see ciac-backend-rust's identical comment —
    // depth and replay-tape support are decoupled fields on purpose.
    sim_replay: false,
};

/// Human-readable, closed list of reasons `ciac sim --target java` cannot
/// yet simulate `ir`, empty when it can — the same gate Rust's own
/// `unsupported_sim_capabilities` (v0.17 M11, full parity 27UpdatePlan.md
/// M4), TypeScript's (v0.23 M9, full parity M6), and Go's (v0.24 M9, full
/// parity M7) compute. 27UpdatePlan.md M8 closes Java's own gate the
/// identical way: `World.java` (`World.java.j2`) now fakes every capability
/// a typed handler can call — `db.get`/`update`/`delete`/`query`/`count`/
/// `delete_where` (in addition to the v0.25 M9 narrow `db.insert`/publish),
/// `cache` (both via a `lower.rs` world-guard leaf), and `object_store`/
/// `email`/`search`/`external_http`/`auth` (each via its own wrapper
/// class's own `ObjectProvider<World>`, mirroring `Queue.java`'s own
/// pre-existing `publishChecked` pattern) — so `lower::scan`'s own
/// `unguarded_verbs` list is always empty and `auth` no longer needs a
/// standalone refusal either. Kept as a function (not a bare `Vec::new()`
/// constant) for the identical reason Rust's/TypeScript's/Go's own M4/M6/M7
/// kept theirs: `TargetInfo::sim` needs a `fn(&NormalizedIr) -> Vec<String>`
/// value, and a real function documents the empty list's own reasoning
/// inline rather than leaving a bare literal to look unintentional.
///
/// **A structural note, identical to Rust's/TypeScript's/Go's own M4/M6/M7
/// finding:** this does *not* flip `TargetInfo::sim` from `SimSupport::
/// Narrow` to `SimSupport::Full` — `commands.rs`'s `sim_inner` dispatch
/// hardcodes `SimSupport::Full => sim_drive_python(..)`, so doing that would
/// silently misroute Java-generated projects through Python's driver.
/// Java's `TargetInfo` stays `SimSupport::Narrow` with this function now
/// always returning empty — "full" in observed behavior, not in enum shape.
pub fn unsupported_sim_capabilities(_ir: &NormalizedIr) -> Vec<String> {
    Vec::new()
}

/// Template-facing counterpart of `World.java`'s own `WorldReference`.
#[derive(serde::Serialize)]
struct SimWorldReferenceCtx {
    field_name: String,
    target_table: Option<String>,
    on_delete: &'static str,
    unique: bool,
}

/// Template-facing counterpart of `World.java`'s own `WorldTable`.
#[derive(serde::Serialize)]
struct SimWorldTableCtx {
    name: String,
    references: Vec<SimWorldReferenceCtx>,
}

/// Builds the schema `SimRunner.java.j2` passes to `new World(.., tables)`
/// (27UpdatePlan.md M8) -- without it, `World` falls back to an empty
/// schema and every reference/unique/cascade check silently becomes a
/// no-op, the same gap Rust's/TypeScript's/Go's own M4/M6/M7 caught live
/// against `domain-orders.ciac`. Reuses `ciac_codegen::migrations::
/// snapshot_schema` -- the same reference/unique-column facts the
/// migration DDL itself is built from, so this can never drift from what
/// the real schema actually enforces. Mirrors `ciac-backend-go`'s own
/// `sim_world_tables` exactly, except `on_delete`'s own uppercase spelling
/// here (`World.java.j2`'s `RefAction.valueOf` reads this string
/// directly, unlike TS's/Go's own lowercase-string `on_delete` field).
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
                        "CASCADE"
                    } else {
                        "RESTRICT"
                    },
                })
                .collect();
            SimWorldTableCtx { name, references }
        })
        .collect()
}

/// Multi-service counterpart of [`sim_world_tables`]: the SystemSimRunner's
/// one shared `World` (28UpdatePlan.md M8) needs every table name -- and
/// every foreign key's `target_table` -- spelled the same namespaced way
/// `lower.rs`'s own `world_table_key` composes them at typed-handler
/// lowering time (`"{service}::{table}"`, via `World.namespacedTableKey`),
/// or a reference/uniqueness check would silently look up a table the world
/// never registered. Builds a `physical table name -> owning service` map
/// from `ir.tables()` directly (mirrors `ciac-backend-rust`'s/
/// `ciac-backend-go`'s own `sim_world_tables_multi` exactly, modulo the
/// uppercase `on_delete` spelling this backend's own `sim_world_tables`
/// already established). A compiler-owned link table (`orders__line_items`)
/// carries no `Table` node of its own; its prefix (`orders`) is always its
/// source table's physical name, so the same map resolves it too.
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
                        "CASCADE"
                    } else {
                        "RESTRICT"
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
pub struct JavaBackend;

impl Backend for JavaBackend {
    fn id(&self) -> &'static str {
        "java"
    }

    fn description(&self) -> &'static str {
        "Java 21 project using Spring Boot 3.x (Spring MVC, virtual threads)"
    }

    fn supports(&self, component: &Component) -> bool {
        // M2: `Database` (all three engines uniformly — JDBC needs no
        // per-engine gating any more than `database/sql` did for Go)
        // plus `Service { signature: None }`, the crud-synthesized
        // store marker `ciac_sema::build::crud` also emits alongside
        // `Api`+`Database` (Go's own M2 finding, found the identical
        // way — a real example refused on `Database` alone).
        //
        // M3: `Queue`/`Stream`/`Worker`/`Job`/`Channel`/`Scheduler`/
        // `Realtime` — one wide gate, the same "engine-agnostic
        // component, per-engine branch stays inside the template"
        // shape M2 already established for `Database` (Go's own M3
        // precedent). `events <Name>;` needed no separate gate: it
        // lowers to the same `Component::Worker` node a plain
        // `worker` declaration does, split into a stub-vs-pipeline
        // shape only at the codegen model layer.
        //
        // M4: `Service { signature: Some(_) }` (typed handlers) widens
        // in alongside `signature: None` — every `HostSyntax` leaf is
        // implemented for trait completeness (`lower.rs`), but the
        // *component* kinds a typed handler body can request
        // (`ObjectStore`/`Email`/`Search`/`ExternalHttp`, plus
        // `Cache`/`Auth`) stay refused below until M6/M7 add their own
        // client wrappers — `typed-handlers.ciac`/`typed-video.ciac`/
        // `extras-verbs.ciac` stay `CIAC0011`-refused this milestone;
        // `domain-orders.ciac`/`query-verbs.ciac` (db-only) are this
        // milestone's actual proving examples, mirroring Go's/TS's own
        // M4 precedent exactly (read directly, not assumed).
        // M6: `Auth` (both JWT and OAuth2, one resource-server-starter
        // mechanism per Pillar 7's own table) widens in alongside
        // everything M1-M5 already support.
        //
        // M7: `Cache` (spring-data-redis `StringRedisTemplate`),
        // `ObjectStore` (AWS SDK v2 S3, path-style for MinIO), `Email`
        // (Spring's `JavaMailSenderImpl` against SMTP/Mailpit),
        // `Search` (dependency-free `HttpClient` against OpenSearch's
        // REST API, mirroring Go's own choice), `ExternalHttp`
        // (Spring's `RestClient`) — every typed-handler leaf these
        // widen in for was already implemented at M4 (`lower.rs`'s own
        // `cache_field`/`object_store_field`/`email_field`/
        // `search_field`/`http_field`), unexercised until this
        // milestone's own `AppState` bean wiring exists for them to
        // reference. `Metrics`/`Tracing` (Micrometer/OTel via
        // `pom.xml`/`application.yml`) and `Users` (the dev-Keycloak-
        // issuer-default computation Go's/TS's own M7 already found is
        // fully target-neutral, needing zero backend-specific code)
        // are distinct `Component`/`NodeKind` variants of their own —
        // found live via `traced-checkout.ciac`'s own `CIAC0011`
        // refusal before this widening — so each needs its own arm
        // here even though none but `Tracing` changes what this
        // backend actually emits.
        //
        // `26UpdatePlan.md` M3: `Logging` widens in too, closing a gap
        // this file's own module doc (line 15,
        // `| Logging | SLF4J + Logback + logstash-logback-encoder |`)
        // claimed true since M1 while `supports()` refused it —
        // `logback-spring.xml.j2`, gated on `has_logging`, is what
        // makes the claim true.
        matches!(
            component,
            Component::Api { .. }
                | Component::Database { .. }
                | Component::Queue { .. }
                | Component::Stream { .. }
                | Component::Worker { .. }
                | Component::Job { .. }
                | Component::Channel { .. }
                | Component::Scheduler { .. }
                | Component::Realtime { .. }
                | Component::Service { .. }
                | Component::Auth { .. }
                | Component::Cache { .. }
                | Component::ObjectStore { .. }
                | Component::Email { .. }
                | Component::Search { .. }
                | Component::ExternalHttp { .. }
                | Component::Metrics { .. }
                | Component::Tracing { .. }
                | Component::Users { .. }
                | Component::Logging { .. }
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
        env.add_filter("java_type", filters::java_type);
        env.add_filter("java_db_type", filters::java_db_type);
        env.add_filter("java_is_primitive", filters::java_is_primitive);
        env.add_filter("java_camel", filters::java_camel);
        env.add_filter("java_pascal", filters::java_pascal);
        env.add_filter("java_is_uuid", filters::java_is_uuid);
        env.add_filter("java_ddl_type", filters::java_ddl_type);
        env.add_filter("spring_cron", filters::spring_cron);
        env.add_filter("jdbcph", filters::jdbcph);

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
            // No system-level README template -- matching Go's/TS's
            // own settled precedent (found live at their own M1/M3):
            // neither emits one either, so this doesn't invent a new
            // template neither other target's own users asked for.
            project.add_file(
                "docker-compose.yml",
                ciac_codegen::compose::render_system_compose(&model, &COMPOSE_OPTS)?,
            );
            // The root-level combined index -- found live (C3, this
            // milestone's own wider `supports()` gate is what first
            // makes a multi-service system reachable for Java at all):
            // every other target writes one alongside each service's
            // own `openapi.json`, this one was simply missing.
            project.add_file(
                "openapi.json",
                serde_json::to_string_pretty(&ciac_codegen::openapi::build_index(&model))
                    .map_err(|e| BackendError::Other(e.to_string()))?,
            );
            // 28UpdatePlan.md M8c: `system-runner/SystemSimRunner.java`
            // -- one shared driver for every `--scenario` run across
            // the whole system, compiled directly by `commands.rs`'s
            // own `sim_drive_java_multi` against a driver-assembled
            // classpath (Option B, no aggregator pom -- see the
            // template's own doc comment). Each service's own `Ctx`
            // gains a `java_package` field here (the same
            // underscore-stripped name `emit_service` computes for
            // itself) since the template needs it to spell every
            // service's own fully-qualified class references.
            let system_services: Vec<minijinja::Value> = model
                .services
                .iter()
                .map(|ctx| {
                    let java_package = ctx.module.replace('_', "");
                    context! { java_package => java_package, ..minijinja::Value::from_serialize(ctx) }
                })
                .collect();
            project.add_file(
                "system-runner/SystemSimRunner.java",
                env.get_template("SystemSimRunner.java.j2")
                    .map_err(BackendError::Template)?
                    .render(context! {
                        services => system_services,
                        sim_world_tables => sim_world_tables_multi(ir),
                    })
                    .map_err(BackendError::Template)?,
            );
            project.notes.push(
                "multi-service system: each directory is a complete project; \
                 `docker compose up` runs them all together"
                    .to_owned(),
            );
        }
        format_all_java(&mut project)?;
        Ok(project)
    }
}

fn empty() -> minijinja::Value {
    minijinja::Value::from_serialize(())
}

/// One `RowMappers.java` entry — `table_name` is `Some` only for a
/// record backing a `table` declaration (never a CRUD resource),
/// carrying the table's own declared PascalCase name (e.g.
/// `Customers` for `table Customers: Customer;`) so `RowMappers.java.j2`
/// can surface it in a doc comment.
#[derive(Serialize)]
struct RowMapperEntry<'a> {
    table_name: Option<String>,
    record: &'a context::RecordCtx,
}

/// Emits one deployable Spring Boot module (today's single-service
/// layout) under `prefix`. Multi-service systems get their compose
/// file at the root instead of per service (mirrors every other
/// backend's own `emit_service`).
fn emit_service(
    env: &minijinja::Environment<'_>,
    ir: &NormalizedIr,
    ctx: &context::Ctx,
    multi: bool,
    prefix: &str,
    project: &mut GeneratedProject,
) -> Result<(), BackendError> {
    // `com.ciac.<module, underscores stripped>` (Pillar 3's default
    // package naming, derived via the factory's own `ctx.module`
    // rather than a new `GenOptions::java_package` override field --
    // M1's disclosed scope reduction: no reachable example needs a
    // custom package name yet, and the plan's own override surface is
    // deferred future work if/when one does, recorded here rather
    // than built speculatively).
    let java_package = ctx.module.replace('_', "");
    let at = |p: &str| format!("{prefix}{p}");
    let render = |name: &str, extra: minijinja::Value| -> Result<String, BackendError> {
        env.get_template(name)
            .map_err(BackendError::Template)?
            .render(context! { c => ctx, java_package => java_package, multi, ..extra })
            .map_err(BackendError::Template)
    };
    // No hand-written Jinja template can reproduce
    // `google-java-format`'s own line-wrapping/column-width decisions
    // (mirrors Go's own `render_go`/`gofmt` reasoning exactly), so
    // every `.java` file still needs the real formatter's pass before
    // Spotless's `check` goal (bound to `verify`, Pillar 8) will agree
    // with what CIaC emitted. `30UpdatePlan.md` M2: that pass no longer
    // happens here, per file — a JVM cold start plus javac-internals
    // classload costs ~0.51s regardless of file size, so one process
    // per file scaled the whole build to tens of seconds. Formatting is
    // now a single batched `format_all_java` post-pass over every
    // `.java` file `generate()` produced; this alias exists only so
    // every call site below keeps reading as "produces formatted Java
    // source" without a 39-site rename.
    let render_java = render;

    project.add_file(at("pom.xml"), render("pom.xml.j2", empty())?);
    project.add_file(
        at(".mvn/wrapper/maven-wrapper.properties"),
        render("maven-wrapper.properties.j2", empty())?,
    );
    project.add_file(at("mvnw"), MVNW_SH.to_owned());
    project.add_file(at("mvnw.cmd"), MVNW_CMD.to_owned());
    project.add_file(at("Dockerfile"), render("Dockerfile.j2", empty())?);
    project.add_file(at(".dockerignore"), "target\n.mvn/wrapper/*.jar\n");
    project.add_file(at("README.md"), render("README.md.j2", empty())?);
    if prefix.is_empty() {
        project.add_file(
            at("docker-compose.yml"),
            ciac_codegen::compose::render_service_compose(ctx, &COMPOSE_OPTS)?,
        );
    }

    let pkg_path = java_package.clone();
    let java_root = format!("src/main/java/com/ciac/{pkg_path}");
    let test_root = format!("src/test/java/com/ciac/{pkg_path}");

    project.add_file(
        at("src/main/resources/application.yml"),
        render("application.yml.j2", empty())?,
    );
    if ctx.has_logging {
        project.add_file(
            at("src/main/resources/logback-spring.xml"),
            render("logback-spring.xml.j2", empty())?,
        );
    }
    project.add_file(
        at(&format!("{java_root}/Application.java")),
        render_java("Application.java.j2", empty())?,
    );
    project.add_file(
        at(&format!("{java_root}/state/AppState.java")),
        render_java("AppState.java.j2", empty())?,
    );
    project.add_file(
        at(&format!("{java_root}/observability/HealthController.java")),
        render_java("HealthController.java.j2", empty())?,
    );
    project.add_file(
        at(&format!("{java_root}/routes/Envelope.java")),
        render_java("Envelope.java.j2", empty())?,
    );
    project.add_file(
        at(&format!("{java_root}/routes/ErrorAdvice.java")),
        render_java("ErrorAdvice.java.j2", empty())?,
    );
    project.add_file(
        at(&format!("{java_root}/routes/BadRequestException.java")),
        render_java("BadRequestException.java.j2", empty())?,
    );
    project.add_file(
        at(&format!("{java_root}/routes/NotFoundException.java")),
        render_java("NotFoundException.java.j2", empty())?,
    );
    if ctx.has_auth {
        project.add_file(
            at(&format!("{java_root}/routes/UnauthorizedException.java")),
            render_java("UnauthorizedException.java.j2", empty())?,
        );
        project.add_file(
            at(&format!("{java_root}/routes/ForbiddenException.java")),
            render_java("ForbiddenException.java.j2", empty())?,
        );
        project.add_file(
            at(&format!("{java_root}/routes/Auth.java")),
            render_java("Auth.java.j2", empty())?,
        );
        project.add_file(
            at(&format!("{java_root}/state/SecurityConfig.java")),
            render_java("SecurityConfig.java.j2", empty())?,
        );
    }
    // MockMvc `ScopeTests` (M6). `26UpdatePlan.md` M5 widened this
    // from JWT-only to both schemes: the file's own no-token/
    // malformed-token cases are scheme-agnostic (gated inside the
    // template on hasAuthStep/hasAuth), and its JWT-only `bearer`/
    // `bearerExp` helpers plus their wrong_scope/correct_scope/
    // expired_token blocks stay gated on `c.auth_scheme == "jwt"`
    // inside the template. OAuth2 gets the scheme-specific equivalent
    // via `OAuthRigTests` below.
    if !ctx.scopes.is_empty() {
        project.add_file(
            at(&format!("{test_root}/routes/ScopeTests.java")),
            render_java("ScopeTests.java.j2", empty())?,
        );
    }
    // The no-infra OAuth2 rig (`26UpdatePlan.md` M5): real RS256
    // signing against an in-process JWKS stub (`com.sun.net.httpserver`,
    // JDK stdlib), gated the same way `ScopeTests` is gated on
    // `c.scopes` above.
    if ctx.auth_scheme == "oauth2" && !ctx.scopes.is_empty() {
        project.add_file(
            at(&format!("{test_root}/routes/OAuthRigTests.java")),
            render_java("OAuthRigTests.java.j2", empty())?,
        );
    }
    // `LogShapeTest` (`26UpdatePlan.md` M3): proves the `LogstashEncoder`
    // wired into `logback-spring.xml` actually produces parseable
    // structured JSON with the expected field shape.
    if ctx.has_logging {
        project.add_file(
            at(&format!("{test_root}/LogShapeTest.java")),
            render_java("LogShapeTest.java.j2", empty())?,
        );
    }

    // A CRUD resource (typed or keyed) also needs `Schemas` — the
    // keyed variant especially, since it has no backing `RecordCtx` of
    // its own in `ctx.records` to have already pulled this file in.
    if !ctx.records.is_empty() || !ctx.resources.is_empty() {
        project.add_file(
            at(&format!("{java_root}/schemas/Schemas.java")),
            render_java("Schemas.java.j2", empty())?,
        );
        for record in &ctx.records {
            project.add_file(
                at(&format!("{java_root}/schemas/{}.java", record.name.clone())),
                render_java("record.java.j2", context! { record => record })?,
            );
            for r_enum in &record.enums {
                project.add_file(
                    at(&format!("{java_root}/schemas/{}.java", r_enum.name.clone())),
                    render_java("RecordEnum.java.j2", context! { record_enum => r_enum })?,
                );
            }
        }
    }

    if ctx.has_db {
        project.add_file(
            at(&format!("{java_root}/state/DataSources.java")),
            render_java("DataSources.java.j2", empty())?,
        );
    }

    // One `RowMapper` per distinct record backing either a CRUD
    // resource or a `table` declaration a typed handler's `db.get`/
    // `db.query` reads (M4) — deduplicated by record name here, in
    // Rust, rather than in the template (`RowMappers.java.j2` used to
    // own this loop itself before M4 widened it to cover `ctx.tables`
    // too; a `{% set %}`-accumulated dedupe list inside a Jinja `for`
    // loop doesn't reliably mutate across iterations the way this
    // plain `Vec`/`HashSet` does).
    let mut row_mapper_records: Vec<RowMapperEntry> = Vec::new();
    {
        let mut seen = std::collections::HashSet::new();
        for r in ctx.resources.iter().filter_map(|r| r.record.as_ref()) {
            if seen.insert(r.name.clone()) {
                row_mapper_records.push(RowMapperEntry {
                    table_name: None,
                    record: r,
                });
            }
        }
        // A table's own declared name (`table_name`, e.g. `Customers`
        // for `table Customers: Customer;`) never otherwise appears
        // anywhere in Java's own generated output the way it does in
        // every other target's own row-struct/ORM-model class name
        // (Go's `type Customers struct`, Python's `class
        // Customers(Base)`) — Java names things after the singular
        // *record*, not the table, everywhere else. Carrying it
        // through into a doc comment here (`c4b_declared_topology_
        // appears_in_every_target`'s own cross-target contract) is
        // cosmetic, not behavioral: found live via that shared
        // conformance test once `domain-orders.ciac` became
        // Java-reachable this milestone.
        for t in &ctx.tables {
            if seen.insert(t.record.name.clone()) {
                row_mapper_records.push(RowMapperEntry {
                    table_name: Some(t.class_name.clone()),
                    record: &t.record,
                });
            }
        }
    }
    if !row_mapper_records.is_empty() {
        project.add_file(
            at(&format!("{java_root}/schemas/RowMappers.java")),
            render_java(
                "RowMappers.java.j2",
                context! { row_mapper_records => row_mapper_records },
            )?,
        );
    }
    for resource in &ctx.resources {
        let non_id_fields: Vec<&ciac_codegen::model::FieldCtx> = resource
            .record
            .as_ref()
            .map(|r| r.fields.iter().filter(|f| f.name != "id").collect())
            .unwrap_or_default();
        project.add_file(
            at(&format!(
                "{java_root}/schemas/{}In.java",
                resource.name.clone()
            )),
            render_java(
                "ResourceIn.java.j2",
                context! { resource => resource, fields => non_id_fields },
            )?,
        );
        if resource.record.is_none() {
            project.add_file(
                at(&format!(
                    "{java_root}/schemas/{}Entity.java",
                    resource.name.clone()
                )),
                render_java("ResourceEntity.java.j2", context! { resource => resource })?,
            );
        }
        project.add_file(
            at(&format!(
                "{java_root}/services/{}.java",
                resource.store_class.clone()
            )),
            render_java(
                "ResourceStore.java.j2",
                context! { resource => resource, fields => non_id_fields },
            )?,
        );
        project.add_file(
            at(&format!(
                "{java_root}/routes/{}Controller.java",
                resource.name.clone()
            )),
            render_java(
                "ResourceController.java.j2",
                context! { resource => resource },
            )?,
        );
    }

    if ctx.queue_engine.is_some() {
        project.add_file(
            at(&format!("{java_root}/state/Queue.java")),
            render_java("Queue.java.j2", empty())?,
        );
    }

    // M9 simulation slice: `World.java` (main, gated the same as
    // Rust's/TS's/Go's own restatement) plus `SimRunner.java` (test-
    // scoped, `MockMvc`/`spring-test` only ever sit on the `test`
    // classpath) -- generated unconditionally for any program with
    // something this world can fake, whether or not that specific
    // program is ever actually eligible for `ciac sim` (an unguarded-
    // verb-calling program still gets a compiling SimRunner; `ciac sim`
    // itself is what refuses to run it, via `unsupported_sim_
    // capabilities` above). 27UpdatePlan.md M8 broadens this from
    // `has_db or has_queue` to the full 8-condition check, matching
    // TypeScript's/Go's own M6/M7 fix, now that `World.java` fakes
    // every capability, not just db.insert/publish.
    //
    // 28UpdatePlan.md M8c: `|| !ctx.call_targets.is_empty()` closes the
    // same gap Rust's/TypeScript's/Go's own M6a/M7a checkpoints already
    // found -- a service reachable only via a routed `call` from
    // another service (no local db/queue/etc. of its own) still needs a
    // world instance to register its handlers against. `World.java`
    // itself is emitted at `com.ciac.simshared.World` (physically
    // duplicated per service -- see the template's own doc comment) in
    // multi mode instead of `com.ciac.<pkg>.sim.World`; `SimRunner.java`
    // is still emitted per service even in multi mode (mirroring Go's
    // own M7c precedent: harmless, unused by `SystemSimRunner`, kept
    // for single-service-within-a-system build/test symmetry) and
    // imports the shared `World` type explicitly since it no longer
    // shares a package with it.
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
        let world_path = if multi {
            format!("{prefix}src/main/java/com/ciac/simshared/World.java")
        } else {
            at(&format!("{java_root}/sim/World.java"))
        };
        project.add_file(world_path, render_java("World.java.j2", empty())?);
        project.add_file(
            at(&format!("{test_root}/sim/SimRunner.java")),
            render_java(
                "SimRunner.java.j2",
                context! { sim_world_tables => sim_world_tables(ir) },
            )?,
        );
    }

    // M7 ontology wrapper classes: one shared class per capability
    // *kind*, reused across every named instance of that kind via a
    // distinct `AppState` bean per instance (mirrors Go's own
    // `objectstore.ObjectStore`/`email.Email`/`search.Search`/
    // `httpclients.ExternalHttp` one-struct-many-beans shape).
    if ctx.has_object_store {
        project.add_file(
            at(&format!("{java_root}/state/ObjectStore.java")),
            render_java("ObjectStore.java.j2", empty())?,
        );
    }
    if ctx.has_email {
        project.add_file(
            at(&format!("{java_root}/state/Email.java")),
            render_java("Email.java.j2", empty())?,
        );
    }
    if ctx.has_search {
        project.add_file(
            at(&format!("{java_root}/state/Search.java")),
            render_java("Search.java.j2", empty())?,
        );
    }
    if ctx.has_external_http {
        project.add_file(
            at(&format!("{java_root}/state/ExternalHttp.java")),
            render_java("ExternalHttp.java.j2", empty())?,
        );
    }

    // M7 typed HTTP call clients: one class per `call <Service>.<Api>`
    // target, deduplicated in `ctx.call_targets` by the shared model
    // layer already (mirrors Go's own `client.go.j2` emission loop).
    for target in &ctx.call_targets {
        project.add_file(
            at(&format!(
                "{java_root}/clients/{}.java",
                target.class_name.clone()
            )),
            render_java(
                "Client.java.j2",
                context! { t => target, caller => ctx.service_name },
            )?,
        );
    }

    // Classic (`signature: None`) handlers not tied to a `crud` --
    // seeded (business logic goes here, regeneration never overwrites
    // it), the same shape every other target's own handler stub uses.
    for service in &ctx.services {
        project.add_seeded_file(
            at(&format!(
                "{java_root}/services/{}.java",
                service.class_name.clone()
            )),
            render_java("Service.java.j2", context! { service => service })?,
        );
    }

    // Typed handlers (M4, `HostSyntax` leaves — `crates/ciac-backend-
    // java/src/lower.rs`): inline bodies are compiler-owned
    // (`logic/<Name>.java`, regenerated every build); `extern`
    // handlers are seeded (`services/<Name>.java`, same discipline as
    // a classic handler's own stub), mirroring Go's/Python's own M4
    // routing exactly.
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
        let content = render_java(
            "logic.java.j2",
            context! { handler => minijinja::Value::from_serialize(&handler) },
        )?;
        if hir.body.is_some() {
            project.add_file(at(&format!("{java_root}/logic/{name}.java")), content);
        } else {
            project.add_seeded_file(at(&format!("{java_root}/services/{name}.java")), content);
        }
    }

    for worker in &ctx.workers {
        project.add_file(
            at(&format!(
                "{java_root}/workers/{}Worker.java",
                worker.name.clone()
            )),
            render_java("Worker.java.j2", context! { worker => worker })?,
        );
    }
    for job in &ctx.jobs {
        project.add_file(
            at(&format!("{java_root}/workers/{}Job.java", job.name.clone())),
            render_java("Job.java.j2", context! { job => job })?,
        );
    }
    for consumer in &ctx.consumers {
        project.add_file(
            at(&format!(
                "{java_root}/workers/{}Consumer.java",
                consumer.name.clone()
            )),
            render_java("Consumer.java.j2", context! { consumer => consumer })?,
        );
    }
    for channel in &ctx.channels {
        project.add_file(
            at(&format!(
                "{java_root}/routes/{}Channel.java",
                channel.name.clone()
            )),
            render_java("Channel.java.j2", context! { channel => channel })?,
        );
    }

    let openapi_doc = serde_json::to_string_pretty(&ciac_codegen::openapi::build_document(ctx))
        .map_err(|e| BackendError::Other(e.to_string()))?;
    project.add_file(at("openapi.json"), openapi_doc.clone());
    // Spring can serve a classpath resource directly (no Go-style
    // `go:embed`-can't-reach-outside-its-own-directory constraint), so
    // this colocated copy exists purely so `HealthController`'s
    // `/openapi.json` route has a resource to load without reaching
    // outside `src/main/resources` at runtime. Named `apidoc.json`
    // (not `openapi.json`) so it doesn't trip the conformance
    // harness's C3 byte-identical-path-set check across targets --
    // found live (the same real gap Go's own M1 hit and fixed the
    // identical way, `cmd/api/apidoc.json`): C3 matches files by
    // `ends_with("openapi.json")`, so a same-named second file here
    // would make Java's own path set diverge from every other
    // target's single-copy set.
    project.add_file(at("src/main/resources/apidoc.json"), openapi_doc);

    for api in &ctx.apis {
        project.add_file(
            at(&format!(
                "{java_root}/routes/{}Controller.java",
                filters::java_pascal(api.snake.clone())
            )),
            render_java("ApiController.java.j2", context! { api => api })?,
        );
    }

    project.add_file(
        at(&format!("{test_root}/NoInfraBootTest.java")),
        render_java("NoInfraBootTest.java.j2", empty())?,
    );

    Ok(())
}

/// The standard Maven wrapper shell/batch scripts (`mvn -N
/// wrapper:wrapper -Dmaven=3.9.11`'s own "only-script" distribution
/// type, the modern wrapper generation, live-generated once against
/// the real Maven install and vendored verbatim) — boilerplate
/// identical across every Maven project (nothing in it is program-
/// specific). No `MavenWrapperDownloader.java`/bootstrap jar needed:
/// the "only-script" wrapper type downloads the pinned Maven
/// distribution zip directly via `curl`/`wget`, resolved through
/// `.mvn/wrapper/maven-wrapper.properties`' own `distributionUrl`.
const MVNW_SH: &str = include_str!("../vendor/mvnw");
const MVNW_CMD: &str = include_str!("../vendor/mvnw.cmd");

/// The vendored `google-java-format` "all-deps" jar. Unlike `gofmt`
/// (ships free with the Go toolchain), `google-java-format` does not
/// ship with the JDK — vendoring it here keeps `ciac build --target
/// java` self-contained the same way `mvnw`/`mvnw.cmd` already are,
/// rather than requiring a second external tool on `PATH`. The plain
/// jar Maven Central also publishes is *not* self-contained (throws
/// `NoClassDefFoundError` for Guava at runtime); this "all-deps"
/// variant bundles its transitive dependencies, verified live by
/// running it standalone before vendoring.
const GOOGLE_JAVA_FORMAT_JAR: &[u8] =
    include_bytes!("../vendor/google-java-format-1.19.2-all-deps.jar");

/// Materializes the embedded jar to a stable path on disk once (`java
/// -jar` needs a real file, not stdin-supplied bytes) — reused across
/// calls within a process (and across processes, since the path is
/// content-addressed by a length check) rather than a fresh temp file
/// per invocation.
fn vendored_jar_path() -> Result<std::path::PathBuf, BackendError> {
    let path = std::env::temp_dir().join("ciac-google-java-format-1.19.2-all-deps.jar");
    let up_to_date = path
        .metadata()
        .is_ok_and(|m| m.len() == GOOGLE_JAVA_FORMAT_JAR.len() as u64);
    if !up_to_date {
        std::fs::write(&path, GOOGLE_JAVA_FORMAT_JAR)
            .map_err(|e| BackendError::Other(format!("writing vendored jar to {path:?}: {e}")))?;
    }
    Ok(path)
}

/// Batch-formats every `.java` file this backend just generated through
/// one `google-java-format -i @argfile` invocation instead of one JVM
/// spawn per file (`30UpdatePlan.md` M2, the dominant fix in the arc:
/// per-file stdin formatting cost ~0.51s/file regardless of size —
/// mostly JVM startup and javac-internals classload — so a 40-file
/// `order-system.ciac` Java build paid ~20s in formatter spawns alone).
/// The batching machinery itself lives in `ciac_codegen::format_batch`
/// (M3 generalized it so Go's `gofmt` could share it); this function is
/// only the Java-specific configuration: which files to match, and how
/// to invoke `google-java-format` given the scratch directory and file
/// list `format_batch` already wrote out. `@argfile` is used
/// unconditionally (not just past `ARG_MAX`) since it costs nothing on
/// small batches and removes a size-dependent code path entirely.
fn format_all_java(project: &mut GeneratedProject) -> Result<(), BackendError> {
    ciac_codegen::format_batch::format_batch(
        project,
        |path| path.ends_with(".java"),
        |scratch, paths| {
            let argfile_path = scratch.join("argfile");
            let mut argfile_lines = String::new();
            for path in paths {
                argfile_lines.push_str(&path.to_string_lossy());
                argfile_lines.push('\n');
            }
            std::fs::write(&argfile_path, &argfile_lines)
                .map_err(|e| BackendError::Other(format!("writing java-format argfile: {e}")))?;

            let jar_path = vendored_jar_path()?;
            let mut cmd = std::process::Command::new("java");
            cmd.arg("--add-exports=jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED")
                .arg("--add-exports=jdk.compiler/com.sun.tools.javac.file=ALL-UNNAMED")
                .arg("--add-exports=jdk.compiler/com.sun.tools.javac.parser=ALL-UNNAMED")
                .arg("--add-exports=jdk.compiler/com.sun.tools.javac.tree=ALL-UNNAMED")
                .arg("--add-exports=jdk.compiler/com.sun.tools.javac.util=ALL-UNNAMED")
                .arg("--add-opens=jdk.compiler/com.sun.tools.javac.code=ALL-UNNAMED")
                .arg("--add-opens=jdk.compiler/com.sun.tools.javac.comp=ALL-UNNAMED")
                .arg("-jar")
                .arg(&jar_path)
                .arg("-i")
                .arg(format!("@{}", argfile_path.display()));
            Ok(cmd)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ping_ir() -> NormalizedIr {
        let src = "service Ping;\n\nrecord Message {\n    id: Uuid;\n    text: String;\n}\n\napi Echo: Message {\n    method: POST;\n    path: \"/echo\";\n}\npipeline Echo: Return;\n";
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = ciac_diagnostics::Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()))
    }

    #[test]
    fn supports_apis() {
        let backend = JavaBackend;
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
        let backend = JavaBackend;
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

    #[test]
    fn target_info_is_populated() {
        let backend = JavaBackend;
        let info = backend.target_info();
        assert_eq!(info.project_marker, "pom.xml");
        assert_eq!(info.source_extension, "java");
        assert!(matches!(info.sim, SimSupport::Narrow { .. }));
        assert_eq!(
            (info.migration_filename)(1, "add_orders"),
            "V0001__add_orders.sql"
        );
    }

    #[test]
    fn generates_ping_parity_file_set() {
        let ir = ping_ir();
        let backend = JavaBackend;
        let project = backend
            .generate(&ir, &GenOptions::default())
            .expect("java generates");
        let paths: Vec<&str> = project.files().map(|(p, _)| p).collect();
        for expect in [
            "pom.xml",
            "mvnw",
            "mvnw.cmd",
            "Dockerfile",
            "README.md",
            "docker-compose.yml",
            "src/main/resources/application.yml",
            "src/main/java/com/ciac/ping/Application.java",
            "src/main/java/com/ciac/ping/state/AppState.java",
            "src/main/java/com/ciac/ping/routes/Envelope.java",
            "src/main/java/com/ciac/ping/schemas/Schemas.java",
            "src/main/java/com/ciac/ping/schemas/Message.java",
            "src/main/java/com/ciac/ping/routes/EchoController.java",
            "src/test/java/com/ciac/ping/NoInfraBootTest.java",
            "openapi.json",
        ] {
            assert!(paths.contains(&expect), "missing {expect} in {paths:?}");
        }
    }

    /// `30UpdatePlan.md` M2's own exit condition: a rejected file's
    /// error must name the real generated path the user can go look
    /// at, not the batch pass's own scratch-directory path (which is
    /// removed by the time the error reaches anyone).
    #[test]
    fn format_all_java_remaps_scratch_paths_in_errors() {
        let mut project = GeneratedProject::new();
        project.add_file(
            "src/main/java/com/ciac/broken/Broken.java",
            "this is not valid java {{{",
        );
        let err = format_all_java(&mut project).expect_err("invalid source is rejected");
        let message = err.to_string();
        assert!(
            message.contains("src/main/java/com/ciac/broken/Broken.java"),
            "error should name the real path: {message}"
        );
        assert!(
            !message.contains("ciac-java-fmt"),
            "error should not leak the scratch directory: {message}"
        );
    }
}
