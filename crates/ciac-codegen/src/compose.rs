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
}

const SERVICE_COMPOSE: &str = include_str!("../templates/docker-compose.yml.j2");
const SYSTEM_COMPOSE: &str = include_str!("../templates/system-compose.yml.j2");

fn environment() -> Result<minijinja::Environment<'static>, minijinja::Error> {
    crate::template::environment([
        ("docker-compose.yml.j2", SERVICE_COMPOSE),
        ("system-compose.yml.j2", SYSTEM_COMPOSE),
    ])
}

fn backend_value(opts: &BackendComposeOpts) -> minijinja::Value {
    minijinja::context! {
        db_scheme => opts.db_url_scheme,
        workers_command => opts.workers_command,
    }
}

/// Renders the single-service `docker-compose.yml` for one deployable.
pub fn render_service_compose(
    ctx: &Ctx,
    opts: &BackendComposeOpts,
) -> Result<String, minijinja::Error> {
    environment()?
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
    environment()?
        .get_template("system-compose.yml.j2")?
        .render(minijinja::context! {
            m => minijinja::Value::from_serialize(model),
            backend => backend_value(opts),
        })
}
