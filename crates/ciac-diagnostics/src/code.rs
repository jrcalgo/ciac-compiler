use crate::Severity;
use serde::Serialize;

macro_rules! error_codes {
    ($($variant:ident = ($code:literal, $severity:ident, $title:literal, $explanation:literal),)*) => {
        /// Stable, documented error codes for every diagnostic the compiler
        /// can emit. Codes are append-only: once published, a code's meaning
        /// never changes.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
        #[serde(into = "&'static str")]
        pub enum ErrorCode {
            $($variant,)*
        }

        impl ErrorCode {
            pub const ALL: &'static [ErrorCode] = &[$(ErrorCode::$variant,)*];

            /// The stable code string, e.g. `CIAC0001`.
            pub fn code(self) -> &'static str {
                match self {
                    $(ErrorCode::$variant => $code,)*
                }
            }

            pub fn default_severity(self) -> Severity {
                match self {
                    $(ErrorCode::$variant => Severity::$severity,)*
                }
            }

            /// Short human-readable title.
            pub fn title(self) -> &'static str {
                match self {
                    $(ErrorCode::$variant => $title,)*
                }
            }

            /// Long-form explanation shown by `ciac explain <code>`.
            pub fn explanation(self) -> &'static str {
                match self {
                    $(ErrorCode::$variant => $explanation,)*
                }
            }

            /// Looks up a code by its string form (case-insensitive).
            pub fn parse(s: &str) -> Option<ErrorCode> {
                let upper = s.to_ascii_uppercase();
                Self::ALL.iter().copied().find(|c| c.code() == upper)
            }
        }

        impl From<ErrorCode> for &'static str {
            fn from(code: ErrorCode) -> &'static str {
                code.code()
            }
        }
    };
}

error_codes! {
    InvalidToken = (
        "CIAC0001",
        Error,
        "invalid token",
        "The source contains a character sequence that is not part of the CIaC \
         language. Check for stray punctuation or unterminated comments."
    ),
    UnexpectedToken = (
        "CIAC0002",
        Error,
        "unexpected token",
        "The parser encountered a token that is not valid at this position. \
         The message lists what was expected instead. Declarations end with \
         `;` and pipelines are written `pipeline Name: Step -> Step;`."
    ),
    DuplicateDeclaration = (
        "CIAC0003",
        Error,
        "duplicate declaration",
        "A component with this name is already declared. Every api, worker, \
         crud, events, and pipeline name must be unique within a program."
    ),
    UnknownPipelineTarget = (
        "CIAC0004",
        Error,
        "pipeline has no matching component",
        "A pipeline's name must match a declared `api` or `worker`. A pipeline \
         attached to an api describes the synchronous request flow; a pipeline \
         attached to a worker describes its asynchronous processing chain."
    ),
    MissingCapability = (
        "CIAC0005",
        Error,
        "missing capability for construct",
        "A construct requires a capability that the `use { .. }` block does \
         not declare: the `Auth` step requires `auth`, the `Queue` step and \
         `events` require `queue`, and `crud` requires `db`."
    ),
    CyclicDependency = (
        "CIAC0006",
        Error,
        "cyclic dependency",
        "The architecture graph contains a cycle in its request, message, or \
         dependency flow, e.g. a worker that publishes back onto the queue it \
         consumes from. CIaC programs must describe acyclic flows."
    ),
    UnreachableComponent = (
        "CIAC0007",
        Warning,
        "unreachable component",
        "A declared component is never used: a worker no pipeline publishes \
         to, an api without a pipeline, or a declared capability nothing \
         consumes. Remove it or wire it into a pipeline."
    ),
    InvalidAuthPlacement = (
        "CIAC0008",
        Error,
        "invalid auth placement",
        "The `Auth` step must be the first step of an api pipeline so that no \
         work happens before the request is authenticated, and it may not \
         appear in worker pipelines, which have no incoming request to \
         authenticate."
    ),
    IncompatibleComposition = (
        "CIAC0009",
        Error,
        "incompatible composition",
        "The steps of a pipeline violate a composition rule: `Return` is only \
         valid as the final step of an api pipeline, and `Queue` may appear \
         at most once per pipeline."
    ),
    MissingServiceDeclaration = (
        "CIAC0010",
        Error,
        "missing service declaration",
        "Every CIaC program must begin by naming the system it describes with \
         exactly one `service <Name>;` declaration."
    ),
    UnsupportedConstruct = (
        "CIAC0011",
        Error,
        "construct not supported by backend",
        "The selected code-generation backend does not support a component in \
         this program (for example a queue or database engine it has no \
         implementation for). Choose a different provider or backend."
    ),
    DuplicateCapability = (
        "CIAC0012",
        Error,
        "duplicate capability",
        "The `use { .. }` block declares the same capability twice. Each of \
         auth, db, cache, queue, logging, and metrics may be declared once."
    ),
    UnknownProvider = (
        "CIAC0013",
        Error,
        "unknown provider",
        "A `use` entry names a capability or provider the language does not \
         know. Supported: auth JWT; db Postgres; cache Redis; queue NATS or \
         Kafka; logging Structured; metrics Prometheus."
    ),
    EmptyPipeline = (
        "CIAC0014",
        Error,
        "empty pipeline",
        "A pipeline must contain at least one step."
    ),
    UnknownType = (
        "CIAC0015",
        Error,
        "unknown type",
        "A declaration references a type the program does not define: a \
         stream, api, or crud names an undeclared record, or a record field \
         uses an unknown type. Field types are String, Int, Float, Bool, \
         Uuid, Timestamp, Json, or an inline `enum { A, B }`."
    ),
    TypeMismatch = (
        "CIAC0016",
        Error,
        "payload type mismatch",
        "A pipeline publishes to a stream whose record type differs from \
         the pipeline's payload type. The payload type comes from the api's \
         request record (or the consumed stream's record for workers); it \
         must match the published stream's record."
    ),
    UnknownStream = (
        "CIAC0017",
        Error,
        "unknown stream",
        "A `publish` step or a worker's `on` clause references a stream \
         that is not declared. Declare it with `stream <Name>: <Record>;`."
    ),
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for code in ErrorCode::ALL {
            let s = code.code();
            assert!(s.starts_with("CIAC") && s.len() == 8, "malformed code {s}");
            assert!(seen.insert(s), "duplicate code {s}");
        }
    }

    #[test]
    fn parse_roundtrip() {
        assert_eq!(
            ErrorCode::parse("ciac0006"),
            Some(ErrorCode::CyclicDependency)
        );
        assert_eq!(ErrorCode::parse("CIAC9999"), None);
    }
}
