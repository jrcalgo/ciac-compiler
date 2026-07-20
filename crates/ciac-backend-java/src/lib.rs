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
    sim: SimSupport::None {
        reason: "Java simulation support lands at 25UpdatePlan.md M9",
    },
};

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
        // M1: only `Api` — `ping.ciac`'s `pipeline Echo: Return` binds
        // no handler, so no `Service` node is even in play (Go's own
        // M1 finding, true here for the identical reason: claiming a
        // kind before any template implements it would pass gating
        // and then fail on an undefined template variable instead of
        // a clean `CIAC0011`).
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

        let mut project = GeneratedProject::new();
        for ctx in &model.services {
            let prefix = if model.multi {
                format!("{}/", ctx.dir)
            } else {
                String::new()
            };
            emit_service(&env, ir, ctx, &prefix, &mut project)?;
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
        }
        Ok(project)
    }
}

fn empty() -> minijinja::Value {
    minijinja::Value::from_serialize(())
}

/// Emits one deployable Spring Boot module (today's single-service
/// layout) under `prefix`. Multi-service systems get their compose
/// file at the root instead of per service (mirrors every other
/// backend's own `emit_service`).
fn emit_service(
    env: &minijinja::Environment<'_>,
    _ir: &NormalizedIr,
    ctx: &context::Ctx,
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
            .render(context! { c => ctx, java_package => java_package, ..extra })
            .map_err(BackendError::Template)
    };
    // No hand-written Jinja template can reproduce
    // `google-java-format`'s own line-wrapping/column-width decisions
    // (mirrors Go's own `render_go`/`gofmt` reasoning exactly) — every
    // `.java` file routes through the real formatter at generation
    // time so Spotless's `check` goal (bound to `verify`, Pillar 8)
    // agrees with what CIaC just emitted instead of drifting from it.
    let render_java = |name: &str, extra: minijinja::Value| -> Result<String, BackendError> {
        google_java_format(&render(name, extra)?)
    };

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

    if !ctx.records.is_empty() {
        project.add_file(
            at(&format!("{java_root}/schemas/Schemas.java")),
            render_java("Schemas.java.j2", empty())?,
        );
        for record in &ctx.records {
            project.add_file(
                at(&format!("{java_root}/schemas/{}.java", record.name.clone())),
                render_java("record.java.j2", context! { record => record })?,
            );
        }
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

/// Formats Java source through the real `google-java-format` tool —
/// see `emit_service`'s `render_java` for why this is a deliberate,
/// disclosed dependency rather than a template-layer approximation
/// (mirrors Go's own `gofmt` reasoning exactly: no Jinja template can
/// reproduce a real formatter's column-width-sensitive line-wrapping
/// decisions).
///
/// The JVM's module system hides `com.sun.tools.javac.*` behind
/// `--add-exports`/`--add-opens` by default (JDK 16+ strong
/// encapsulation) — `google-java-format` reaches into javac's own
/// parser/AST internals to do its formatting, so every invocation
/// needs these flags; verified live against the vendored jar (piping
/// a test file through stdin/stdout) before wiring this in.
fn google_java_format(src: &str) -> Result<String, BackendError> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};

    let jar_path = vendored_jar_path()?;

    let mut child = Command::new("java")
        .arg("--add-exports=jdk.compiler/com.sun.tools.javac.api=ALL-UNNAMED")
        .arg("--add-exports=jdk.compiler/com.sun.tools.javac.file=ALL-UNNAMED")
        .arg("--add-exports=jdk.compiler/com.sun.tools.javac.parser=ALL-UNNAMED")
        .arg("--add-exports=jdk.compiler/com.sun.tools.javac.tree=ALL-UNNAMED")
        .arg("--add-exports=jdk.compiler/com.sun.tools.javac.util=ALL-UNNAMED")
        .arg("--add-opens=jdk.compiler/com.sun.tools.javac.code=ALL-UNNAMED")
        .arg("--add-opens=jdk.compiler/com.sun.tools.javac.comp=ALL-UNNAMED")
        .arg("-jar")
        .arg(&jar_path)
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            BackendError::Other(format!(
                "`java` not found on PATH ({e}) — a JDK 21 must be installed \
                 to generate `--target java` output, not only to validate it"
            ))
        })?;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(src.as_bytes())
        .map_err(|e| BackendError::Other(format!("writing to google-java-format: {e}")))?;
    let output = child
        .wait_with_output()
        .map_err(|e| BackendError::Other(format!("running google-java-format: {e}")))?;
    if !output.status.success() {
        return Err(BackendError::Other(format!(
            "google-java-format rejected generated source (this is a codegen \
             bug, not a user error): {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|e| BackendError::Other(format!("google-java-format output: {e}")))
}

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
        assert!(!backend.supports(&Component::Queue {
            name: "Q".to_owned(),
            engine: ciac_ir::QueueEngine::Nats,
        }));
    }

    #[test]
    fn target_info_is_populated() {
        let backend = JavaBackend;
        let info = backend.target_info();
        assert_eq!(info.project_marker, "pom.xml");
        assert_eq!(info.source_extension, "java");
        assert!(matches!(info.sim, SimSupport::None { .. }));
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
}
