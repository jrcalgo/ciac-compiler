//! Shared docker-compose assembly (v0.9 M1).
//!
//! Until v0.9 the Python and Rust backends each carried their own copy
//! of `docker-compose.yml.j2` and `system-compose.yml.j2` — 150+ lines
//! apiece that differed in exactly two places: the Postgres URL scheme
//! the generated app's driver wants, and the command that starts the
//! workers container. Everything infra-shaped (the per-instance
//! postgres/redis/minio/mailpit/opensearch service entries, volumes,
//! `depends_on` wiring) was byte-identical and drifted only by luck.
//!
//! The templates now live here, once, with those two divergences
//! injected per backend via [`BackendComposeOpts`]. Output is proven
//! byte-identical to the pre-extraction files by the golden snapshots
//! (all examples × both backends embed their compose files).

use crate::model::{Ctx, SystemModel};

/// The two (and only two) backend-specific values in a compose file.
#[derive(Debug)]
pub struct BackendComposeOpts {
    /// URL scheme for `DATABASE_URL`-style env vars, without the
    /// trailing `://` — e.g. `postgresql+asyncpg` (SQLAlchemy) or
    /// `postgres` (SQLx).
    pub db_url_scheme: &'static str,
    /// The workers container's `command:` value, as the literal YAML
    /// array text to emit — e.g. `["python", "-m", "app.workers"]` or
    /// `["workers"]`.
    pub workers_command: &'static str,
    /// URL scheme for MySQL instances (v0.11 M1), same convention as
    /// `db_url_scheme` — e.g. `mysql+aiomysql` (SQLAlchemy) or `mysql`.
    pub mysql_url_scheme: &'static str,
    /// SQLite URL for a db file named `<db_name>.db` under the app
    /// container's `data/` directory (v0.13 M3): the text before the
    /// db name — e.g. `sqlite+aiosqlite:///data/` or `sqlite://data/`.
    pub sqlite_url_prefix: &'static str,
    /// ...and after it — e.g. `` or `?mode=rwc`.
    pub sqlite_url_suffix: &'static str,
    /// Absolute in-container path of the app's `data/` directory,
    /// where the sqlite volume mounts — e.g. `/app/data` (Python
    /// image WORKDIR) or `/data` (Rust runtime image, cwd `/`).
    pub data_mount: &'static str,
}

const SERVICE_COMPOSE: &str = include_str!("../templates/docker-compose.yml.j2");
const SYSTEM_COMPOSE: &str = include_str!("../templates/system-compose.yml.j2");

fn environment() -> minijinja::Environment<'static> {
    crate::template::environment([
        ("docker-compose.yml.j2", SERVICE_COMPOSE),
        ("system-compose.yml.j2", SYSTEM_COMPOSE),
    ])
}

fn backend_value(opts: &BackendComposeOpts) -> minijinja::Value {
    minijinja::context! {
        db_scheme => opts.db_url_scheme,
        mysql_scheme => opts.mysql_url_scheme,
        workers_command => opts.workers_command,
        sqlite_url_prefix => opts.sqlite_url_prefix,
        sqlite_url_suffix => opts.sqlite_url_suffix,
        data_mount => opts.data_mount,
    }
}

/// Renders the single-service `docker-compose.yml` for one deployable.
pub fn render_service_compose(
    ctx: &Ctx,
    opts: &BackendComposeOpts,
) -> Result<String, minijinja::Error> {
    environment()
        .get_template("docker-compose.yml.j2")?
        .render(minijinja::context! {
            c => minijinja::Value::from_serialize(ctx),
            backend => backend_value(opts),
        })
}

/// Renders the system-root `docker-compose.yml` for a multi-service
/// project (one app + one workers entry per service, shared infra).
pub fn render_system_compose(
    model: &SystemModel,
    opts: &BackendComposeOpts,
) -> Result<String, minijinja::Error> {
    environment()
        .get_template("system-compose.yml.j2")?
        .render(minijinja::context! {
            m => minijinja::Value::from_serialize(model),
            backend => backend_value(opts),
        })
}

/// The otel-collector's config (v0.15 M3): receives OTLP from every
/// service, forwards traces to Jaeger's own OTLP receiver, and logs
/// them at `debug` verbosity too — the same "real dev container"
/// convention every other capability follows, minus a vendor-baked
/// image needing no config of its own. Same file regardless of
/// single-/multi-service layout (both compose templates mount it
/// unmodified), so it's a plain constant rather than a template.
pub const OTEL_COLLECTOR_CONFIG: &str = r#"receivers:
  otlp:
    protocols:
      grpc:
        endpoint: 0.0.0.0:4317
      http:
        endpoint: 0.0.0.0:4318

exporters:
  otlp:
    endpoint: jaeger:4317
    tls:
      insecure: true
  debug:
    verbosity: basic

service:
  pipelines:
    traces:
      receivers: [otlp]
      exporters: [otlp, debug]
"#;
