//! End-to-end tests of semantic analysis: source text in, diagnostics and
//! graph shape out.

use ciac_diagnostics::{Diagnostics, ErrorCode, SourceMap};
use ciac_ir::{Component, EdgeKind, FieldType, HttpMethod, NodeKind, NormalizedIr, StepKind};

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
    assert!(matches!(&pipeline.steps[0].kind, StepKind::Auth { .. }));
    assert!(matches!(&pipeline.steps[1].kind, StepKind::Handler { .. }));
    let StepKind::Publish { stream } = &pipeline.steps[2].kind else {
        panic!("Queue step lowers to a publish on the default stream");
    };
    let Component::Stream { subject, .. } = &ir.node(*stream).component else {
        panic!("publish target is a stream node");
    };
    assert_eq!(subject, "video_platform.events");
    assert!(matches!(&pipeline.steps[3].kind, StepKind::Return));
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
fn job_requires_scheduler_and_owns_untyped_pipeline() {
    let (ir, diags) = analyze(
        "service T;\nuse { scheduler Cron; queue NATS; }\nstream Done: Video;\nrecord Video { id: Uuid; }\njob Cleanup { schedule: \"0 3 * * *\"; }\npipeline Cleanup: Prune;\n",
    );
    assert!(
        !diags.has_errors(),
        "unexpected errors: {:?}",
        error_codes(&diags)
    );
    let ir = ir.expect("valid program");
    let job = ir.find_named(NodeKind::Job, "Cleanup").expect("job");
    let scheduler = ir.singleton(NodeKind::Scheduler).expect("scheduler");
    assert!(ir
        .edges_from(job.id)
        .any(|e| e.to == scheduler.id && e.kind == EdgeKind::DependsOn));
    let pipeline = ir.pipeline_of(job.id).expect("job pipeline");
    assert_eq!(pipeline.payload, None);
}

#[test]
fn job_without_scheduler_is_an_error() {
    let (ir, diags) = analyze("service T;\njob Cleanup { schedule: \"0 3 * * *\"; }\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::MissingCapability));
}

#[test]
fn invalid_job_cron_is_an_error() {
    let (ir, diags) =
        analyze("service T;\nuse { scheduler Cron; }\njob Cleanup { schedule: \"not cron\"; }\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::InvalidCron));
}

#[test]
fn job_pipeline_cannot_auth_or_return() {
    let (ir, diags) = analyze(
        "service T;\nuse { scheduler Cron; auth JWT; }\njob Cleanup { schedule: \"0 3 * * *\"; }\npipeline Cleanup: Auth -> Return;\n",
    );
    assert!(ir.is_none());
    let codes = error_codes(&diags);
    assert!(codes.contains(&ErrorCode::InvalidAuthPlacement));
    assert!(codes.contains(&ErrorCode::IncompatibleComposition));
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

#[test]
fn component_attributes_resolve_to_configs() {
    let (ir, diags) = analyze(
        r#"service Media;
use { auth JWT; db Postgres; cache Redis; queue NATS; }
record Video { id: Uuid; status: enum { Ready, Failed }; }
stream Uploaded: Video { subject: "media.custom"; }
api Upload: Video { method: PUT; path: "/videos"; scope: "videos:write"; }
worker Transcoder on Uploaded { concurrency: 4; max_retries: 2; }
crud Clip: Video { cache_ttl: 60; page_size: 50; }
pipeline Upload: Auth -> Return;
pipeline Transcoder: Process;
"#,
    );
    assert!(
        !diags.has_errors(),
        "unexpected errors: {:?}",
        diags.codes()
    );
    let ir = ir.expect("valid program");
    let api = ir.find_named(NodeKind::Api, "Upload").expect("api");
    let Component::Api { config, .. } = &api.component else {
        panic!("api node");
    };
    assert_eq!(config.method, HttpMethod::Put);
    assert_eq!(config.path.as_deref(), Some("/videos"));
    assert_eq!(config.scope.as_deref(), Some("videos:write"));

    let stream = ir.find_named(NodeKind::Stream, "Uploaded").expect("stream");
    let Component::Stream { subject, .. } = &stream.component else {
        panic!("stream node");
    };
    assert_eq!(subject, "media.custom");

    let worker = ir
        .find_named(NodeKind::Worker, "Transcoder")
        .expect("worker");
    let Component::Worker { config, .. } = &worker.component else {
        panic!("worker node");
    };
    assert_eq!(config.concurrency, 4);
    assert_eq!(config.max_retries, 2);

    assert_eq!(ir.resources[0].config.cache_ttl, 60);
    assert_eq!(ir.resources[0].config.page_size, 50);
}

#[test]
fn unknown_attribute_is_rejected() {
    let (ir, diags) = analyze("service X;\napi A { nope: 1; }\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::UnknownAttribute));
}

#[test]
fn invalid_attribute_values_are_rejected() {
    let cases = [
        (
            "service X;\nrecord A { id: Uuid; }\napi A: A { method: GET; }\n",
            "GET with typed body",
        ),
        (
            "service X;\napi A { scope: \"x\"; }\npipeline A: Return;\n",
            "scope without Auth",
        ),
        (
            "service X;\nuse { db Postgres; }\ncrud Note { cache_ttl: 60; }\n",
            "cache_ttl without cache",
        ),
    ];
    for (src, label) in cases {
        let (ir, diags) = analyze(src);
        assert!(ir.is_none(), "{label}");
        assert!(
            error_codes(&diags).contains(&ErrorCode::InvalidAttributeValue),
            "{label}: {:?}",
            diags.codes()
        );
    }
}

const MATCH_PROGRAM: &str = r#"service Media;
use { queue NATS; }
record Video { id: Uuid; status: enum { Ready, Failed }; }
stream Transcoded: Video;
stream DeadLetters: Video;
api In: Video;
worker TranscodedWorker on Transcoded;
worker DeadWorker on DeadLetters;
pipeline In:
    Transcode
    -> match status {
        Ready -> publish Transcoded;
        Failed -> publish DeadLetters;
    };
pipeline TranscodedWorker: Notify;
pipeline DeadWorker: Archive;
"#;

#[test]
fn exhaustive_match_analyzes_and_wires_arms() {
    let (ir, diags) = analyze(MATCH_PROGRAM);
    assert!(
        !diags.has_errors(),
        "unexpected errors: {:?}",
        diags.codes()
    );
    let ir = ir.expect("valid program");
    let api = ir.find_named(NodeKind::Api, "In").expect("api");
    let pipeline = ir.pipeline_of(api.id).expect("pipeline");
    let StepKind::Match { field, arms } = &pipeline.steps[1].kind else {
        panic!("expected match");
    };
    assert_eq!(field, "status");
    assert_eq!(arms.len(), 2);
    assert_eq!(arms[0].label.as_deref(), Some("Ready"));
    assert!(matches!(&arms[0].steps[0].kind, StepKind::Publish { .. }));
}

#[test]
fn wildcard_match_is_exhaustive() {
    let src = MATCH_PROGRAM.replace(
        "Ready -> publish Transcoded;\n        Failed -> publish DeadLetters;",
        "Ready -> publish Transcoded;\n        _ -> publish DeadLetters;",
    );
    let (ir, diags) = analyze(&src);
    assert!(
        ir.is_some(),
        "wildcard covers remaining variants: {:?}",
        diags.codes()
    );
    assert!(!diags.has_errors());
}

#[test]
fn match_validation_reports_new_codes() {
    let cases = [
        (
            MATCH_PROGRAM.replace("Failed -> publish DeadLetters;\n", ""),
            ErrorCode::NonExhaustiveMatch,
            "non-exhaustive",
        ),
        (
            MATCH_PROGRAM.replace("Failed -> publish DeadLetters;", "Other -> publish DeadLetters;"),
            ErrorCode::InvalidMatch,
            "unknown variant",
        ),
        (
            "service X;\napi A;\npipeline A: match status { Ready -> Return; };\n".to_owned(),
            ErrorCode::InvalidMatch,
            "untyped",
        ),
        (
            MATCH_PROGRAM.replace(
                "Ready -> publish Transcoded;",
                "Ready -> match status { Ready -> publish Transcoded; Failed -> publish DeadLetters; };",
            ),
            ErrorCode::InvalidMatch,
            "nested",
        ),
        (
            MATCH_PROGRAM.replace("    };\n", "    } -> Return;\n"),
            ErrorCode::InvalidMatch,
            "not terminal",
        ),
    ];
    for (src, code, label) in cases {
        let (ir, diags) = analyze(&src);
        assert!(ir.is_none(), "{label}");
        assert!(
            error_codes(&diags).contains(&code),
            "{label}: expected {code:?}, got {:?}",
            diags.codes()
        );
    }
}

#[test]
fn named_capability_instances_bind_to_handlers() {
    let (ir, diags) = analyze(
        "service X;\n\
         use { db main Postgres; db analytics Postgres; cache hot Redis; }\n\
         handler Store { db: main; cache: hot; }\n\
         api A;\n\
         pipeline A: Store -> Return;\n",
    );
    assert!(
        !diags.has_errors(),
        "unexpected errors: {:?}",
        diags.codes()
    );
    let ir = ir.expect("valid program");
    let store = ir.find_named(NodeKind::Service, "Store").expect("handler");
    let targets: Vec<_> = ir
        .edges_from(store.id)
        .map(|e| {
            ir.node(e.to)
                .component
                .name()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert!(targets.contains(&"main".to_owned()));
    assert!(targets.contains(&"hot".to_owned()));
    assert!(!targets.contains(&"analytics".to_owned()));
}

#[test]
fn multiple_instances_without_default_are_ambiguous_for_implicit_handlers() {
    let (ir, diags) = analyze(
        "service X;\n\
         use { db main Postgres; db analytics Postgres; }\n\
         api A;\n\
         pipeline A: Store -> Return;\n",
    );
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::AmbiguousCapabilityBinding));
}

#[test]
fn handler_binding_to_missing_instance_is_reported() {
    let (ir, diags) = analyze(
        "service X;\n\
         use { db main Postgres; }\n\
         handler Store { db: missing; }\n\
         api A;\n\
         pipeline A: Store -> Return;\n",
    );
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::UnknownCapabilityInstance));
}

#[test]
fn new_ontology_capabilities_can_be_declared_and_bound() {
    let (ir, diags) = analyze(
        r#"service X;
use {
    object_store media S3 { bucket: "videos"; }
    email transactional SES;
    search catalog OpenSearch;
    external_http billing { base_url: "https://billing.internal"; }
}
handler Enrich {
    object_store: media;
    email: transactional;
    search: catalog;
    external_http: billing;
}
api A;
pipeline A: Enrich -> Return;
"#,
    );
    assert!(
        !diags.has_errors(),
        "unexpected errors: {:?}",
        diags.codes()
    );
    let ir = ir.expect("valid program");
    assert!(ir.find_named(NodeKind::ObjectStore, "media").is_some());
    assert!(ir.find_named(NodeKind::Email, "transactional").is_some());
    assert!(ir.find_named(NodeKind::Search, "catalog").is_some());
    assert!(ir.find_named(NodeKind::ExternalHttp, "billing").is_some());
}

#[test]
fn multi_service_project_supports_shared_streams_and_typed_calls() {
    let (ir, diags) = analyze(
        r#"project MediaSystem;
record Video { id: Uuid; status: enum { Ready, Failed }; }
stream Uploaded: Video;

service Billing {
    api Charge: Video;
    pipeline Charge: CapturePayment -> Return;
}

service UploadApi {
    use { queue bus NATS; }
    api Upload: Video;
    pipeline Upload:
        call Billing.Charge
        -> publish Uploaded
        -> Return;
}

service Transcoder {
    use { queue bus NATS; }
    worker Transcode on Uploaded;
    pipeline Transcode: Process;
}
"#,
    );
    assert!(
        !diags.has_errors(),
        "unexpected errors: {:?}",
        diags.codes()
    );
    let ir = ir.expect("valid program");
    assert_eq!(ir.name, "MediaSystem");
    assert_eq!(ir.services().count(), 3);
    let upload_service = ir
        .services()
        .find(|service| service.name == "UploadApi")
        .expect("upload service")
        .id;
    let billing_service = ir
        .services()
        .find(|service| service.name == "Billing")
        .expect("billing service")
        .id;
    let upload = ir
        .find_named_in_service(upload_service, NodeKind::Api, "Upload")
        .expect("upload api");
    let charge = ir
        .find_named_in_service(billing_service, NodeKind::Api, "Charge")
        .expect("charge api");
    let pipeline = ir.pipeline_of(upload.id).expect("upload pipeline");
    assert_eq!(pipeline.service, Some(upload_service));
    assert!(matches!(&pipeline.steps[0].kind, StepKind::Call { target } if *target == charge.id));
    assert!(ir
        .edges_from(upload.id)
        .any(|edge| edge.to == charge.id && edge.kind == EdgeKind::ServiceCall));
}

#[test]
fn duplicate_service_is_rejected() {
    let (ir, diags) = analyze("project X;\nservice A {}\nservice A {}\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::DuplicateService));
}

#[test]
fn unknown_service_call_is_rejected() {
    let (ir, diags) = analyze(
        "project X;\nrecord R { id: Uuid; }\nservice A { api In: R; pipeline In: call Missing.Api -> Return; }\n",
    );
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::UnknownService));
}

#[test]
fn unknown_service_member_call_is_rejected() {
    let (ir, diags) = analyze(
        "project X;\nrecord R { id: Uuid; }\nservice A { api In: R; pipeline In: call B.Missing -> Return; }\nservice B { api Out: R; pipeline Out: Return; }\n",
    );
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::UnknownServiceMember));
}

#[test]
fn cross_service_payload_mismatch_is_rejected() {
    let (ir, diags) = analyze(
        "project X;\nrecord A { id: Uuid; }\nrecord B { id: Uuid; }\nservice Caller { api In: A; pipeline In: call Callee.Out -> Return; }\nservice Callee { api Out: B; pipeline Out: Return; }\n",
    );
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::CrossServiceTypeMismatch));
}

#[test]
fn flat_service_local_decls_cannot_mix_with_service_blocks() {
    let (ir, diags) = analyze("project X;\napi Flat;\nservice A {}\n");
    assert!(ir.is_none());
    assert!(error_codes(&diags).contains(&ErrorCode::InvalidServiceScope));
}
