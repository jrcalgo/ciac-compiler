//! End-to-end tests of semantic analysis: source text in, diagnostics and
//! graph shape out.

use ciac_diagnostics::{Diagnostics, ErrorCode, SourceMap};
use ciac_ir::{Component, EdgeKind, FieldType, NodeKind, NormalizedIr, Step};

fn analyze(src: &str) -> (Option<NormalizedIr>, Diagnostics) {
    let mut sources = SourceMap::new();
    let file = sources.add_file("test.ciac", src);
    let mut diags = Diagnostics::new();
    let program = ciac_syntax::parse(src, file, &mut diags);
    let ir = ciac_sema::analyze(&program, &mut diags);
    (ir, diags)
}

fn error_codes(diags: &Diagnostics) -> Vec<ErrorCode> {
    diags
        .iter()
        .filter(|d| d.severity == ciac_diagnostics::Severity::Error)
        .map(|d| d.code)
        .collect()
}

const VIDEO_PLATFORM: &str = "\
service VideoPlatform;
use {
    auth JWT;
    db Postgres;
    cache Redis;
    queue NATS;
}
api Upload;
worker Transcoder;
pipeline Upload:
    Auth
    -> StoreVideo
    -> Queue
    -> Return;
pipeline Transcoder: Transcode -> SaveResult;
";

#[test]
fn flagship_example_analyzes_cleanly() {
    let (ir, diags) = analyze(VIDEO_PLATFORM);
    assert!(
        !diags.has_errors(),
        "unexpected errors: {:?}",
        diags.codes()
    );
    let ir = ir.expect("valid program produces IR");
    assert_eq!(ir.name, "VideoPlatform");

    // The implicit handler exists and is wired to storage.
    let store = ir
        .find_named(NodeKind::Service, "StoreVideo")
        .expect("implicit handler created");
    let targets: Vec<NodeKind> = ir
        .edges_from(store.id)
        .map(|e| ir.node(e.to).component.kind())
        .collect();
    assert!(targets.contains(&NodeKind::Database));
    assert!(targets.contains(&NodeKind::Cache));

    // The worker consumes from the default stream, which sits on the broker.
    let queue = ir.singleton(NodeKind::Queue).expect("queue node");
    let stream = ir
        .find_named(NodeKind::Stream, "Events")
        .expect("default stream created for the legacy Queue step");
    let worker = ir
        .find_named(NodeKind::Worker, "Transcoder")
        .expect("worker");
    assert!(ir
        .edges_from(stream.id)
        .any(|e| e.to == worker.id && e.kind == EdgeKind::AsyncMessage));
    assert!(ir
        .edges_from(stream.id)
        .any(|e| e.to == queue.id && e.kind == EdgeKind::DependsOn));

    // Pipeline steps resolved in order.
    let api = ir.find_named(NodeKind::Api, "Upload").expect("api");
    let pipeline = ir.pipeline_of(api.id).expect("api pipeline");
    assert!(matches!(pipeline.steps[0], Step::Auth { .. }));
    assert!(matches!(pipeline.steps[1], Step::Handler { .. }));
    let Step::Publish { stream } = pipeline.steps[2] else {
        panic!("Queue step lowers to a publish on the default stream");
    };
    let Component::Stream { subject, .. } = &ir.node(stream).component else {
        panic!("publish target is a stream node");
    };
    assert_eq!(subject, "video_platform.events");
    assert!(matches!(pipeline.steps[3], Step::Return));
}

#[test]
fn crud_expands_into_primitives() {
    let (ir, diags) =
        analyze("service Notes;\nuse { auth JWT; db Postgres; cache Redis; }\ncrud Note;\n");
    assert!(
        !diags.has_errors(),
        "unexpected errors: {:?}",
        diags.codes()
    );
    let ir = ir.expect("valid program");
    assert_eq!(ir.resources.len(), 1);
    let resource = &ir.resources[0];
    assert_eq!(resource.name, "Note");

    // API routes through auth to the store service.
    let auth = ir.singleton(NodeKind::Auth).expect("auth node");
    assert!(ir
        .edges_from(resource.api)
        .any(|e| e.to == auth.id && e.kind == EdgeKind::RequestFlow));
    assert!(ir
        .edges_from(resource.service)
        .any(|e| ir.node(e.to).component.kind() == NodeKind::Database));
}

#[test]
fn events_expands_into_queue_and_worker() {
    let (ir, diags) = analyze("service T;\nuse { queue NATS; db Postgres; }\nevents Click;\n");
    assert!(!diags.has_errors());
    let ir = ir.expect("valid program");
    assert_eq!(ir.event_streams.len(), 1);
    let Component::Stream { subject, .. } = &ir.node(ir.event_streams[0].stream).component else {
        panic!("events expansion creates a stream node");
    };
    assert_eq!(subject, "t.click");
    let worker = ir.node(ir.event_streams[0].worker);
    assert_eq!(worker.component.name(), Some("ClickConsumer"));
}

#[test]
fn missing_service_declaration_is_an_error() {
    let (ir, diags) = analyze("api Upload;\npipeline Upload: Work -> Return;\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::MissingServiceDeclaration));
}

#[test]
fn queue_step_without_queue_capability_is_an_error() {
    let (ir, diags) = analyze("service X;\napi A;\npipeline A: Work -> Queue -> Return;\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::MissingCapability));
}

#[test]
fn auth_step_must_come_first() {
    let (ir, diags) =
        analyze("service X;\nuse { auth JWT; }\napi A;\npipeline A: Work -> Auth -> Return;\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::InvalidAuthPlacement));
}

#[test]
fn worker_publishing_to_own_queue_is_a_cycle() {
    let (ir, diags) = analyze(
        "service X;\nuse { queue NATS; }\nworker Loop;\npipeline Loop: Process -> Queue;\n",
    );
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::CyclicDependency));
}

#[test]
fn pipeline_must_match_api_or_worker() {
    let (ir, diags) = analyze("service X;\npipeline Nope: Work -> Return;\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::UnknownPipelineTarget));
}

#[test]
fn duplicate_declarations_are_rejected() {
    let (ir, diags) = analyze("service X;\napi A;\napi A;\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::DuplicateDeclaration));
}

#[test]
fn unknown_provider_is_rejected() {
    let (ir, diags) = analyze("service X;\nuse { db Mongo; }\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::UnknownProvider));
}

#[test]
fn return_is_invalid_in_worker_pipelines() {
    let (ir, diags) =
        analyze("service X;\nuse { queue NATS; }\nworker W;\npipeline W: Process -> Return;\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::IncompatibleComposition));
}

#[test]
fn unused_api_is_a_warning_not_an_error() {
    let (ir, diags) = analyze("service X;\napi Idle;\n");
    assert!(ir.is_some(), "warnings alone must not block compilation");
    assert!(diags.codes().contains(&ErrorCode::UnreachableComponent));
}

#[test]
fn crud_requires_database() {
    let (ir, diags) = analyze("service X;\ncrud Note;\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::MissingCapability));
}

const TYPED_PLATFORM: &str = "\
service Media;
use { auth JWT; db Postgres; queue NATS; }
record Video {
    id: Uuid;
    title: String;
    status: enum { Pending, Ready };
}
stream Uploaded: Video;
stream Notified: Video;
api Upload: Video;
worker Transcoder on Uploaded;
worker Notifier on Notified;
pipeline Upload: Auth -> StoreVideo -> publish Uploaded -> Return;
pipeline Transcoder: Transcode -> publish Notified;
pipeline Notifier: Notify;
";

#[test]
fn typed_program_with_stream_fanout_analyzes_cleanly() {
    let (ir, diags) = analyze(TYPED_PLATFORM);
    assert!(
        !diags.has_errors(),
        "unexpected errors: {:?}",
        diags.codes()
    );
    let ir = ir.expect("valid program");

    // Records resolved with typed fields.
    let (video_id, video) = ir.records().next().expect("record exists");
    assert_eq!(video.name, "Video");
    assert_eq!(video.fields[0].ty, FieldType::Uuid);
    assert!(matches!(video.fields[2].ty, FieldType::Enum { .. }));

    // Streams are typed nodes; the api pipeline carries the record.
    let uploaded = ir.find_named(NodeKind::Stream, "Uploaded").expect("stream");
    let Component::Stream {
        record, subject, ..
    } = &uploaded.component
    else {
        panic!("stream node");
    };
    assert_eq!(*record, Some(video_id));
    assert_eq!(subject, "media.uploaded");

    let api = ir.find_named(NodeKind::Api, "Upload").expect("api");
    let pipeline = ir.pipeline_of(api.id).expect("pipeline");
    assert_eq!(pipeline.payload, Some(video_id));

    // Worker chain: Uploaded -> Transcoder, whose pipeline republishes to
    // a *different* stream — legal, no cycle.
    let transcoder = ir
        .find_named(NodeKind::Worker, "Transcoder")
        .expect("worker");
    assert!(ir
        .edges_from(uploaded.id)
        .any(|e| e.to == transcoder.id && e.kind == EdgeKind::AsyncMessage));
    let worker_pipeline = ir.pipeline_of(transcoder.id).expect("worker pipeline");
    assert_eq!(worker_pipeline.payload, Some(video_id));
}

#[test]
fn republishing_to_consumed_stream_is_a_cycle() {
    let (ir, diags) = analyze(
        "service X;\nuse { queue NATS; }\nrecord E { id: Uuid; }\nstream S: E;\napi In: E;\nworker W on S;\npipeline In: publish S -> Return;\npipeline W: Work -> publish S;\n",
    );
    assert!(
        ir.is_none(),
        "worker republishing to its own stream must fail"
    );
    assert!(error_codes(&diags).contains(&ErrorCode::CyclicDependency));
}

#[test]
fn publish_type_mismatch_is_reported() {
    let (ir, diags) = analyze(
        "service X;\nuse { queue NATS; }\nrecord A { id: Uuid; }\nrecord B { id: Uuid; }\nstream S: A;\napi In: B;\nworker W on S;\npipeline W: Work;\npipeline In: publish S -> Return;\n",
    );
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::TypeMismatch));
}

#[test]
fn untyped_pipeline_cannot_publish_to_typed_stream() {
    let (ir, diags) = analyze(
        "service X;\nuse { queue NATS; }\nrecord A { id: Uuid; }\nstream S: A;\napi In;\nworker W on S;\npipeline W: Work;\npipeline In: publish S -> Return;\n",
    );
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::TypeMismatch));
}

#[test]
fn unknown_stream_is_reported() {
    let (ir, diags) = analyze(
        "service X;\nuse { queue NATS; }\napi In;\npipeline In: publish Nowhere -> Return;\n",
    );
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::UnknownStream));
}

#[test]
fn unknown_record_and_field_type_are_reported() {
    let (ir, diags) = analyze(
        "service X;\nuse { queue NATS; }\nrecord R { bad: Nope; }\nstream S: Missing;\nworker W on S;\npipeline W: Work;\n",
    );
    assert!(ir.is_none());
    let codes = error_codes(&diags);
    assert_eq!(
        codes
            .iter()
            .filter(|c| **c == ErrorCode::UnknownType)
            .count(),
        2,
        "both the bad field type and the missing record are reported: {codes:?}"
    );
}

#[test]
fn stream_requires_queue_capability() {
    let (ir, diags) = analyze("service X;\nrecord A { id: Uuid; }\nstream S: A;\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::MissingCapability));
}
