//! The architectural ontology: the closed set of component types that CIaC
//! programs compose. Engines/providers are enumerated (not stringly typed)
//! so backends can match exhaustively and unsupported combinations are
//! caught at compile time of the compiler itself.

use crate::hir::HandlerBody;
use crate::record::RecordId;
use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize)]
pub enum HttpMethod {
    Get,
    #[default]
    Post,
    Put,
    Delete,
    Patch,
}

impl HttpMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            HttpMethod::Get => "GET",
            HttpMethod::Post => "POST",
            HttpMethod::Put => "PUT",
            HttpMethod::Delete => "DELETE",
            HttpMethod::Patch => "PATCH",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct ApiConfig {
    pub method: HttpMethod,
    pub path: Option<String>,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WorkerConfig {
    pub concurrency: u32,
    pub max_retries: u32,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            concurrency: 1,
            max_retries: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JobConfig {
    pub schedule: String,
    pub catch_up: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ChannelConfig {
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CrudConfig {
    pub cache_ttl: u32,
    pub page_size: u32,
    /// Scope required to read (`GET` list/get) — v0.14 M6. `None`
    /// means reads only need whatever `crud`'s automatic auth gating
    /// already requires (any valid token, no specific scope).
    pub read_scope: Option<String>,
    /// Scope required to write (`POST`/`PUT`/`PATCH`/`DELETE`) —
    /// v0.14 M6.
    pub write_scope: Option<String>,
}

impl Default for CrudConfig {
    fn default() -> Self {
        Self {
            cache_ttl: 300,
            page_size: 100,
            read_scope: None,
            write_scope: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum AuthScheme {
    Jwt,
    /// v0.11 M2: OAuth2 resource server — bearer RS256 JWTs validated
    /// against the issuer's JWKS. Both backends implement it.
    OAuth2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum DbEngine {
    Postgres,
    /// v0.11 M1 (Python), v0.13 M1 (Rust): per-engine pools and SQL
    /// placeholder styles in both backends.
    MySql,
    /// v0.13 M3: the zero-container database — a file under the
    /// generated project's `data/` directory instead of a compose
    /// service; no RDS module; excluded from the direct-connection
    /// system-test round-trip (no host port to reach it on).
    Sqlite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum CacheEngine {
    Redis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum QueueEngine {
    Nats,
    Kafka,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum LoggingProvider {
    Structured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum MetricsProvider {
    Prometheus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum ObjectStoreProvider {
    S3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum EmailProvider {
    Ses,
    Smtp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SearchProvider {
    OpenSearch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum SchedulerProvider {
    Cron,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum RealtimeProvider {
    WebSocket,
    Sse,
}

/// The category of a node, without its configuration payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub enum NodeKind {
    Api,
    Service,
    Worker,
    Job,
    Channel,
    Database,
    Cache,
    Queue,
    Stream,
    Auth,
    Logging,
    Metrics,
    ObjectStore,
    Email,
    Search,
    ExternalHttp,
    Scheduler,
    Realtime,
}

/// A node payload: an architectural component with its resolved
/// configuration.
///
/// `Eq` is deliberately not derived: `Service.signature` can hold a
/// `HandlerBody` containing `f64` literals (`FloatLit`), and `f64` has no
/// total equality (`NaN != NaN`). `PartialEq` (used by tests and
/// `assert_eq!`) is unaffected.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind")]
pub enum Component {
    /// An HTTP API surface (one router / group of endpoints).
    Api {
        name: String,
        /// Request body record, when the api is typed.
        request: Option<RecordId>,
        config: ApiConfig,
    },
    /// A business-logic handler invoked from pipelines.
    Service {
        name: String,
        /// `None` is the classic binding-only handler (v0.1-v0.6):
        /// capability dependencies flow through `EdgeKind::DataFlow`
        /// edges, unaffected by this field. `Some` is a v0.7 typed
        /// handler — either `extern` (`HandlerBody::body: None`) or an
        /// inline body that type-checked successfully. No backend
        /// implements either yet (see `Backend::supports`), so this is
        /// always build-gated with `CIAC0011` until a later milestone.
        signature: Option<HandlerBody>,
    },
    /// An asynchronous consumer of queue messages.
    Worker {
        name: String,
        config: WorkerConfig,
    },
    /// A scheduled in-process pipeline trigger.
    Job {
        name: String,
        config: JobConfig,
    },
    /// A realtime exposure of a stream.
    Channel {
        name: String,
        config: ChannelConfig,
    },
    Database {
        name: String,
        engine: DbEngine,
    },
    Cache {
        name: String,
        engine: CacheEngine,
    },
    Queue {
        name: String,
        engine: QueueEngine,
    },
    /// A named message channel on the queue broker. `record` is the
    /// payload type it carries (`None` = untyped JSON).
    Stream {
        name: String,
        subject: String,
        record: Option<RecordId>,
    },
    Auth {
        name: String,
        scheme: AuthScheme,
        /// OAuth2 only: the token issuer (JWKS at
        /// `{issuer}/.well-known/jwks.json`).
        issuer: Option<String>,
        /// OAuth2 only: expected `aud` claim, unchecked when absent.
        audience: Option<String>,
    },
    Logging {
        name: String,
        provider: LoggingProvider,
    },
    Metrics {
        name: String,
        provider: MetricsProvider,
    },
    ObjectStore {
        name: String,
        provider: ObjectStoreProvider,
        bucket: Option<String>,
    },
    Email {
        name: String,
        provider: EmailProvider,
    },
    Search {
        name: String,
        provider: SearchProvider,
    },
    ExternalHttp {
        name: String,
        base_url: String,
    },
    Scheduler {
        name: String,
        provider: SchedulerProvider,
    },
    Realtime {
        name: String,
        provider: RealtimeProvider,
    },
}

impl Component {
    pub fn kind(&self) -> NodeKind {
        match self {
            Component::Api { .. } => NodeKind::Api,
            Component::Service { .. } => NodeKind::Service,
            Component::Worker { .. } => NodeKind::Worker,
            Component::Job { .. } => NodeKind::Job,
            Component::Channel { .. } => NodeKind::Channel,
            Component::Database { .. } => NodeKind::Database,
            Component::Cache { .. } => NodeKind::Cache,
            Component::Queue { .. } => NodeKind::Queue,
            Component::Stream { .. } => NodeKind::Stream,
            Component::Auth { .. } => NodeKind::Auth,
            Component::Logging { .. } => NodeKind::Logging,
            Component::Metrics { .. } => NodeKind::Metrics,
            Component::ObjectStore { .. } => NodeKind::ObjectStore,
            Component::Email { .. } => NodeKind::Email,
            Component::Search { .. } => NodeKind::Search,
            Component::ExternalHttp { .. } => NodeKind::ExternalHttp,
            Component::Scheduler { .. } => NodeKind::Scheduler,
            Component::Realtime { .. } => NodeKind::Realtime,
        }
    }

    /// The user-visible name for named components and capability instances.
    pub fn name(&self) -> Option<&str> {
        match self {
            Component::Api { name, .. }
            | Component::Service { name, .. }
            | Component::Worker { name, .. }
            | Component::Job { name, .. }
            | Component::Channel { name, .. }
            | Component::Stream { name, .. }
            | Component::Database { name, .. }
            | Component::Cache { name, .. }
            | Component::Queue { name, .. }
            | Component::Auth { name, .. }
            | Component::Logging { name, .. }
            | Component::Metrics { name, .. }
            | Component::ObjectStore { name, .. }
            | Component::Email { name, .. }
            | Component::Search { name, .. }
            | Component::ExternalHttp { name, .. }
            | Component::Scheduler { name, .. }
            | Component::Realtime { name, .. } => Some(name),
        }
    }

    /// A stable human-readable label used in graph dumps and diagnostics.
    pub fn label(&self) -> String {
        match self {
            Component::Api { name, .. } => format!("api {name}"),
            Component::Service { name, .. } => format!("service {name}"),
            Component::Worker { name, .. } => format!("worker {name}"),
            Component::Job { name, .. } => format!("job {name}"),
            Component::Channel { name, .. } => format!("channel {name}"),
            Component::Database { name, engine } => format!("database {name} {engine:?}"),
            Component::Cache { name, engine } => format!("cache {name} {engine:?}"),
            Component::Queue { name, engine } => format!("queue {name} {engine:?}"),
            Component::Stream { name, .. } => format!("stream {name}"),
            Component::Auth { name, scheme, .. } => format!("auth {name} {scheme:?}"),
            Component::Logging { name, provider } => format!("logging {name} {provider:?}"),
            Component::Metrics { name, provider } => format!("metrics {name} {provider:?}"),
            Component::ObjectStore { name, provider, .. } => {
                format!("object_store {name} {provider:?}")
            }
            Component::Email { name, provider } => format!("email {name} {provider:?}"),
            Component::Search { name, provider } => format!("search {name} {provider:?}"),
            Component::ExternalHttp { name, .. } => format!("external_http {name}"),
            Component::Scheduler { name, provider } => format!("scheduler {name} {provider:?}"),
            Component::Realtime { name, provider } => format!("realtime {name} {provider:?}"),
        }
    }
}
