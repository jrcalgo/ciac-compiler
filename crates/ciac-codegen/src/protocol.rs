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
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Bumped whenever [`SystemModel`]'s shape changes in a way that isn't
/// purely additive. Not enforced by anything in M1 — the field exists
/// and is populated so "versioned contract" is a real property from
/// the start, not an afterthought bolted on once something external
/// actually depends on it.
///
/// v0.22 M2: bumped to 2 — `FieldCtx` dropped its `py_type`/
/// `py_out_type`/`rust_type`/`db_rust_type` fields (per-language
/// spellings now live as backend-owned minijinja filters over
/// `type_kind`, which was already on the wire since v0.10 M1).
/// External backends render types from `type_kind` the same way the
/// bundled backends' filters do — see `docs/external-backends.md`.
pub const PROTOCOL_VERSION: u32 = 2;

/// The full wire contract as one JSON Schema document (v0.10 M2):
/// `protocol_version` plus schemas for both halves, derived from the
/// same types that serialize the real payloads so it cannot drift.
/// `ciac codegen-schema` prints this; `docs/protocol-schema.json` is
/// it checked in, held identical by an integration test.
pub fn schema_document() -> serde_json::Value {
    serde_json::json!({
        "protocol_version": PROTOCOL_VERSION,
        "request": schemars::schema_for!(CodegenRequest),
        "response": schemars::schema_for!(CodegenResponse),
    })
}

/// Everything an external backend needs to generate a project for one
/// `--target`, serialized as the request payload.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
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
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CodegenResponse {
    pub protocol_version: u32,
    pub files: Vec<ResponseFile>,
    /// Mirrors [`crate::GeneratedProject::notes`].
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
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
    fn enum_field_type_kind_survives_the_wire() {
        // v0.10 M1: an external backend must be able to read a field's
        // language-neutral kind — including a real enum's generated
        // type name and variants — straight off the wire, instead of
        // string-matching `rust_type`.
        let ir = compile(
            "service Media;\nuse { db Postgres; }\nrecord Video { id: Uuid; status: enum { Pending, Ready }; }\ncrud Video: Video;\n",
        );
        let system = build_system(&ir, &GenOptions::default());
        let request = CodegenRequest::new("go", None, system);
        let json = serde_json::to_string(&request).expect("serializes");
        let restored: CodegenRequest = serde_json::from_str(&json).expect("deserializes");

        let record = restored.system.services[0]
            .records
            .iter()
            .find(|r| r.name == "Video")
            .expect("Video record on the wire");
        let status = record
            .fields
            .iter()
            .find(|f| f.name == "status")
            .expect("status field");
        match &status.type_kind {
            crate::model::FieldTypeKind::Enum { name, variants } => {
                assert_eq!(name, "VideoStatus");
                assert_eq!(variants, &["Pending".to_owned(), "Ready".to_owned()]);
            }
            other => panic!("expected an enum kind, got {other:?}"),
        }
        // And the wire text itself is the documented serde shape.
        assert!(
            json.contains(r#""type_kind":{"kind":"enum","name":"VideoStatus""#),
            "wire shape changed: {json}"
        );
    }

    #[test]
    fn typed_handler_ids_survive_the_round_trip_even_though_theyre_opaque() {
        // examples/single-service/typed-handlers.ciac's shape: a program with an
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
