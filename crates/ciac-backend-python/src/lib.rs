//! Python (FastAPI ecosystem) code-generation backend.
//!
//! Maps the CIaC ontology onto production-standard Python components:
//!
//! | CIaC          | Python                                  |
//! |---------------|-----------------------------------------|
//! | API           | FastAPI `APIRouter`                     |
//! | Service       | plain async service class               |
//! | Worker        | NATS subscription in an asyncio task    |
//! | Database      | SQLAlchemy (async) + asyncpg            |
//! | Cache         | redis-py (asyncio)                      |
//! | Queue         | nats-py                                 |
//! | Auth (JWT)    | FastAPI dependency + PyJWT              |
//! | Logging       | structlog                               |
//! | Metrics       | prometheus-client `/metrics` endpoint   |

use ciac_codegen::{Backend, BackendError, GenOptions, GeneratedProject};
use ciac_ir::{Component, NormalizedIr};

#[derive(Debug, Default)]
pub struct PythonBackend;

impl Backend for PythonBackend {
    fn id(&self) -> &'static str {
        "python"
    }

    fn description(&self) -> &'static str {
        "Python 3.11+ project using FastAPI, SQLAlchemy, redis-py, and nats-py"
    }

    fn supports(&self, component: &Component) -> bool {
        // Kafka has no generator yet; everything else is implemented.
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
