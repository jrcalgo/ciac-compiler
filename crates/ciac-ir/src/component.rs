//! The architectural ontology: the closed set of component types that CIaC
//! programs compose. Engines/providers are enumerated (not stringly typed)
//! so backends can match exhaustively and unsupported combinations are
//! caught at compile time of the compiler itself.

use crate::record::RecordId;
use serde::Serialize;

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
    },
    /// A business-logic handler invoked from pipelines.
    Service {
        name: String,
    },
    /// An asynchronous consumer of queue messages.
    Worker {
        name: String,
    },
    Database {
        engine: DbEngine,
    },
    Cache {
        engine: CacheEngine,
    },
    Queue {
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
        scheme: AuthScheme,
    },
    Logging {
        provider: LoggingProvider,
    },
    Metrics {
        provider: MetricsProvider,
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
        }
    }

    /// The user-visible name for named components (apis, services, workers).
    /// Infrastructure components are identified by their kind instead.
    pub fn name(&self) -> Option<&str> {
        match self {
            Component::Api { name, .. }
            | Component::Service { name }
            | Component::Worker { name }
            | Component::Stream { name, .. } => Some(name),
            _ => None,
        }
    }

    /// A stable human-readable label used in graph dumps and diagnostics.
    pub fn label(&self) -> String {
        match self {
            Component::Api { name, .. } => format!("api {name}"),
            Component::Service { name } => format!("service {name}"),
            Component::Worker { name } => format!("worker {name}"),
            Component::Database { engine } => format!("database {engine:?}"),
            Component::Cache { engine } => format!("cache {engine:?}"),
            Component::Queue { engine } => format!("queue {engine:?}"),
            Component::Stream { name, .. } => format!("stream {name}"),
            Component::Auth { scheme } => format!("auth {scheme:?}"),
            Component::Logging { provider } => format!("logging {provider:?}"),
            Component::Metrics { provider } => format!("metrics {provider:?}"),
        }
    }
}
