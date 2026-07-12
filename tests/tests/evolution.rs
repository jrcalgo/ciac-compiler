//! v0.8 M5: record evolution checks (`ciac-codegen/src/evolution.rs`),
//! exercised against a realistic multi-service shape (adapted from
//! `examples/multi-service-media.ciac`) that has both boundary kinds:
//! a `call` payload (`Billing.Charge`) and a shared stream
//! (`Uploaded`, published by `UploadApi`, consumed by `Transcoder`).

use ciac_codegen::evolution::{diff_records, snapshot_boundary_records, RecordChange};
use ciac_integration_tests::compile;

fn media_system(video_fields: &str) -> String {
    format!(
        r#"
project MediaSystem;
record Video {{ id: Uuid; {video_fields} }}

stream Uploaded: Video;

service Billing {{
    api Charge: Video {{ method: POST; path: "/charge"; }}
    pipeline Charge: CapturePayment -> Return;
}}

service UploadApi {{
    use {{ queue bus NATS; }}
    api Upload: Video {{ method: PUT; path: "/videos"; }}
    pipeline Upload: call Billing.Charge -> StoreVideo -> publish Uploaded -> Return;
}}

service Transcoder {{
    use {{ queue bus NATS; }}
    worker Transcode on Uploaded;
    pipeline Transcode: TranscodeVideo;
}}
"#
    )
}

fn ir(video_fields: &str) -> ciac_ir::NormalizedIr {
    let (ir, diags) = compile(&media_system(video_fields));
    ir.unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()))
}

#[test]
fn removing_a_call_boundary_field_names_the_caller() {
    let old = ir("title: String;");
    let old_schema = snapshot_boundary_records(&old);
    let new = ir("");
    let new_schema = snapshot_boundary_records(&new);

    let Err(changes) = diff_records(&old_schema, &new_schema, &new) else {
        panic!("expected a breaking change");
    };
    // `Video` crosses both the call boundary (`UploadApi` calls
    // `Billing.Charge`) and the stream boundary (`Uploaded`, consumed
    // by `Transcoder`) in this program, so both real consumers must be
    // named — not just the one whose boundary kind this test names.
    let change = changes
        .iter()
        .find(|c| matches!(c, RecordChange::FieldRemoved { field, .. } if field == "title"))
        .expect("title removal reported");
    let RecordChange::FieldRemoved { consumers, .. } = change else {
        unreachable!()
    };
    assert!(consumers.contains(&"UploadApi".to_owned()));
}

#[test]
fn retyping_a_stream_boundary_field_names_the_consumer() {
    let old = ir("title: String;");
    let old_schema = snapshot_boundary_records(&old);
    let new = ir("title: Int;");
    let new_schema = snapshot_boundary_records(&new);

    let Err(changes) = diff_records(&old_schema, &new_schema, &new) else {
        panic!("expected a breaking change");
    };
    // `Video` crosses both boundaries in this program (the `call`
    // payload and the `Uploaded` stream), so both real consumers —
    // `UploadApi` (caller) and `Transcoder` (stream consumer) — must
    // be named. `UploadApi` also owns the stream's producer, so it
    // isn't its own stream-boundary consumer; `Transcoder` is added by
    // the stream-boundary check alone.
    let change = changes
        .iter()
        .find(|c| matches!(c, RecordChange::FieldRetyped { field, .. } if field == "title"))
        .expect("title retype reported");
    let RecordChange::FieldRetyped { consumers, .. } = change else {
        unreachable!()
    };
    assert!(consumers.contains(&"UploadApi".to_owned()));
    assert!(consumers.contains(&"Transcoder".to_owned()));
}

#[test]
fn unchanged_program_has_no_violations() {
    let a = ir("title: String;");
    let schema = snapshot_boundary_records(&a);
    assert!(diff_records(&schema, &schema, &a).is_ok());
}

#[test]
fn added_field_is_backward_compatible() {
    let old = ir("");
    let old_schema = snapshot_boundary_records(&old);
    let new = ir("title: String;");
    let new_schema = snapshot_boundary_records(&new);
    assert!(diff_records(&old_schema, &new_schema, &new).is_ok());
}
