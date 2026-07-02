//! Rust (Axum ecosystem) code-generation backend.
//!
//! Maps the CIaC ontology onto production-standard Rust components:
//!
//! | CIaC          | Rust                                     |
//! |---------------|------------------------------------------|
//! | API           | Axum router                              |
//! | Service       | plain async service struct               |
//! | Worker        | async-nats subscription in a Tokio task  |
//! | Database      | SQLx (Postgres)                          |
//! | Cache         | redis crate (async)                      |
//! | Queue         | async-nats                               |
//! | Auth (JWT)    | middleware + jsonwebtoken                |
//! | Logging       | tracing + tracing-subscriber             |
//! | Metrics       | metrics-exporter-prometheus              |

use ciac_codegen::{Backend, BackendError, GenOptions, GeneratedProject};
use ciac_ir::{Component, NormalizedIr};

#[derive(Debug, Default)]
pub struct RustBackend;

impl Backend for RustBackend {
    fn id(&self) -> &'static str {
        "rust"
    }

    fn description(&self) -> &'static str {
        "Rust project using Axum, SQLx, redis, and async-nats"
    }

    fn supports(&self, component: &Component) -> bool {
        !matches!(
            component,
            Component::Queue {
                engine: ciac_ir::QueueEngine::Kafka
            }
        )
    }

    fn generate(
        &self,
        _ir: &NormalizedIr,
        _opts: &GenOptions,
    ) -> Result<GeneratedProject, BackendError> {
        unimplemented!("replaced in the codegen milestone")
    }
}
