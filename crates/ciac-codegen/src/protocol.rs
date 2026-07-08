//! External backend protocol M1: a stable, versioned, serializable
//! wire contract wrapping [`crate::model::SystemModel`] — the same
//! language-neutral presentation layer every in-process `Backend`
//! already consumes (`ciac-backend-python`/`-rust`), now made
//! round-trippable through JSON so a backend can, in principle, exist
//! as an external process rather than a linked-in Rust crate.
//!
//! M1 defined the *request* half (`ciac codegen-request` dumps it for
//! inspection). M2 adds the *response* half (`CodegenResponse`) — the
//! shape an external `ciac-backend-<target>` process writes to its own
//! stdout after reading a [`CodegenRequest`] from stdin. See
//! [`crate::external::ExternalBackend`] for the process that actually
//! speaks this protocol; this module only defines the wire shapes.

use crate::model::SystemModel;
use crate::FileRole;
use serde::{Deserialize, Serialize};

/// Bumped whenever [`SystemModel`]'s shape changes in a way that isn't
/// purely additive. Not enforced by anything in M1 — the field exists
/// and is populated so "versioned contract" is a real property from
/// the start, not an afterthought bolted on once something external
/// actually depends on it.
pub const PROTOCOL_VERSION: u32 = 1;

/// Everything an external backend needs to generate a project for one
/// `--target`, serialized as the request payload.
#[derive(Debug, Serialize, Deserialize)]
pub struct CodegenRequest {
    pub protocol_version: u32,
    /// The `--target` name the request was built for (informational at
    /// this stage — nothing branches on it yet, since `SystemModel` is
    /// already target-neutral).
    pub target: String,
    /// Mirrors [`crate::GenOptions::project_name`].
    pub project_name: Option<String>,
    pub system: SystemModel,
}

impl CodegenRequest {
    pub fn new(
        target: impl Into<String>,
        project_name: Option<String>,
        system: SystemModel,
    ) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            target: target.into(),
            project_name,
            system,
        }
    }
}

/// What an external backend writes to its own stdout after reading a
/// [`CodegenRequest`] from stdin: the generated file tree, in exactly
/// the shape [`crate::GeneratedProject`] already uses internally, plus
/// `protocol_version` so [`crate::external::ExternalBackend`] can
/// refuse a response from a backend built against an incompatible
/// contract instead of silently misinterpreting it.
#[derive(Debug, Serialize, Deserialize)]
pub struct CodegenResponse {
    pub protocol_version: u32,
    pub files: Vec<ResponseFile>,
    /// Mirrors [`crate::GeneratedProject::notes`].
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseFile {
    pub path: String,
    pub content: String,
    pub role: FileRole,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::build_system;
    use crate::GenOptions;

    fn compile(src: &str) -> ciac_ir::NormalizedIr {
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = ciac_diagnostics::Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()))
    }

    #[test]
    fn request_round_trips_through_json_byte_for_byte() {
        let ir = compile(
            "service Notes;\nuse { db Postgres; }\nrecord Note { id: Uuid; title: String; }\ncrud Note;\n",
        );
        let system = build_system(&ir, &GenOptions::default());
        let request = CodegenRequest::new("python", None, system);

        let json = serde_json::to_string(&request).expect("serializes");
        let restored: CodegenRequest = serde_json::from_str(&json).expect("deserializes");
        let round_tripped = serde_json::to_string(&restored).expect("serializes again");

        assert_eq!(json, round_tripped, "request must round-trip byte-for-byte");
        assert_eq!(restored.protocol_version, PROTOCOL_VERSION);
        assert_eq!(restored.target, "python");
    }

    #[test]
    fn typed_handler_ids_survive_the_round_trip_even_though_theyre_opaque() {
        // examples/typed-handlers.ciac's shape: a program with an
        // inline typed handler body. `Ctx::typed_handlers` carries raw
        // `NodeId`s pointing back into the IR (see the note on
        // `ciac_ir::graph::NodeId`) — this test only proves the ID
        // itself deserializes correctly, not that it's independently
        // useful to a consumer without the IR it points into.
        let ir = compile(
            r#"
service Notes;
use { db Postgres; }
record Note { id: Uuid; title: String; }
table Notes: Note;
handler Store(n: Note) -> Note {
    let inserted = db.insert(Notes, n);
    return inserted;
}
api Create: Note { method: POST; path: "/notes"; }
pipeline Create: Store -> Return;
"#,
        );
        let system = build_system(&ir, &GenOptions::default());
        assert!(
            !system.services[0].typed_handlers.is_empty(),
            "fixture must actually exercise a typed handler"
        );
        let request = CodegenRequest::new("python", None, system);
        let json = serde_json::to_string(&request).expect("serializes");
        let restored: CodegenRequest = serde_json::from_str(&json).expect("deserializes");
        assert_eq!(
            restored.system.services[0].typed_handlers,
            request.system.services[0].typed_handlers
        );
    }
}
