//! The architectural ontology: the closed set of component types that CIaC
//! programs compose. Engines/providers are enumerated (not stringly typed)
//! so backends can match exhaustively and unsupported combinations are
//! caught at compile time of the compiler itself.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CrudConfig {
    pub cache_ttl: u32,
    pub page_size: u32,
}

impl Default for CrudConfig {
    fn default() -> Self {
        Self {
            cache_ttl: 300,
            page_size: 100,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum AuthScheme {
    Jwt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
pub enum DbEngine {
    Postgres,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    },
    /// An asynchronous consumer of queue messages.
    Worker {
        name: String,
        config: WorkerConfig,
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
            | Component::Service { name }
            | Component::Worker { name, .. }
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
            Component::Service { name } => format!("service {name}"),
            Component::Worker { name, .. } => format!("worker {name}"),
            Component::Database { name, engine } => format!("database {name} {engine:?}"),
            Component::Cache { name, engine } => format!("cache {name} {engine:?}"),
            Component::Queue { name, engine } => format!("queue {name} {engine:?}"),
            Component::Stream { name, .. } => format!("stream {name}"),
            Component::Auth { name, scheme } => format!("auth {name} {scheme:?}"),
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
