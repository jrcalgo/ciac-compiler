//! End-to-end tests of semantic analysis: source text in, diagnostics and
//! graph shape out.

use ciac_diagnostics::{Diagnostics, ErrorCode, SourceMap};
use ciac_ir::{EdgeKind, NodeKind, NormalizedIr, Step};

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

    // The worker consumes from the queue.
    let queue = ir.singleton(NodeKind::Queue).expect("queue node");
    let worker = ir
        .find_named(NodeKind::Worker, "Transcoder")
        .expect("worker");
    assert!(ir
        .edges_from(queue.id)
        .any(|e| e.to == worker.id && e.kind == EdgeKind::AsyncMessage));

    // Pipeline steps resolved in order.
    let api = ir.find_named(NodeKind::Api, "Upload").expect("api");
    let pipeline = ir.pipeline_of(api.id).expect("api pipeline");
    assert!(matches!(pipeline.steps[0], Step::Auth { .. }));
    assert!(matches!(pipeline.steps[1], Step::Handler { .. }));
    assert!(matches!(pipeline.steps[2], Step::Queue { .. }));
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
    assert_eq!(ir.event_streams[0].subject, "click");
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
