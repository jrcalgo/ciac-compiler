//! Build-time capability gating: constructs the language accepts but no
//! backend can generate yet must fail `ciac build` (CIAC0011 via
//! `check_support`) while `ciac check` still passes. After v0.6, Kafka is
//! the only remaining gated construct.

use ciac_integration_tests::{backends, compile};

const KAFKA_GATED: &str = r#"
service GatedProbe;

use {
    queue Kafka;
}

api Ping {
    method: GET;
    path: "/ping";
}
"#;

const SCHEDULER_SUPPORTED: &str = r#"
service SchedulerProbe;

use {
    scheduler jobs Cron;
}

job Cleanup {
    schedule: "0 3 * * *";
}

pipeline Cleanup: Prune;
"#;

#[test]
fn kafka_generates_on_python_but_still_gates_on_rust() {
    // v0.11 M3: Kafka graduated on the Python backend (aiokafka); the
    // Rust backend still gates it (rdkafka's native build chain is a
    // cost with no new seam knowledge -- disclosed in 11UpdatePlan.md).
    let (ir, diags) = compile(KAFKA_GATED);
    assert!(
        !diags.has_errors(),
        "check must accept kafka declarations: {:?}",
        diags.codes()
    );
    let ir = ir.expect("program produces IR");

    for backend in backends() {
        let support = ciac_codegen::check_support(backend.as_ref(), &ir);
        match backend.id() {
            "python" => {
                support.expect("python supports Kafka since v0.11 M3");
                let project = backend
                    .generate(&ir, &ciac_codegen::GenOptions::default())
                    .expect("kafka program generates on python");
                let queue_py = project.get("app/queue.py").expect("queue module");
                assert!(queue_py.contains("AIOKafkaProducer"), "{queue_py}");
            }
            "rust" => {
                let err = support.expect_err("rust must still gate Kafka");
                let message = err.to_string();
                assert!(message.contains("queue default Kafka"), "{message}");
            }
            other => panic!("unexpected backend {other}"),
        }
    }
}

#[test]
fn scheduler_jobs_are_supported() {
    let (ir, diags) = compile(SCHEDULER_SUPPORTED);
    assert!(
        !diags.has_errors(),
        "scheduler probe compiles: {:?}",
        diags.codes()
    );
    let ir = ir.expect("probe produces IR");
    for backend in backends() {
        ciac_codegen::check_support(backend.as_ref(), &ir)
            .unwrap_or_else(|err| panic!("{} must support scheduler jobs: {err}", backend.id()));
    }
}

#[test]
fn realtime_channels_are_supported() {
    let source = r#"
service RealtimeProbe;

use {
    queue NATS;
    realtime live WebSocket;
}

record Video { id: Uuid; }
stream Progress: Video;
channel LiveProgress on Progress;
api Upload: Video;
pipeline Upload: publish Progress -> Return;
"#;
    let (ir, diags) = compile(source);
    assert!(
        !diags.has_errors(),
        "realtime probe compiles: {:?}",
        diags.codes()
    );
    let ir = ir.expect("probe produces IR");
    for backend in backends() {
        ciac_codegen::check_support(backend.as_ref(), &ir)
            .unwrap_or_else(|err| panic!("{} must support realtime channels: {err}", backend.id()));
    }
}

const TYPED_HANDLER_GATED: &str = r#"
service TypedHandlerProbe;

use {
    db Postgres;
    object_store S3;
}

record Video {
    id: Uuid;
    title: String;
}

table Videos: Video;

handler StoreVideo(v: Video) -> Video {
    let key = "videos/" + v.id;
    object_store.put(key, v);
    return db.insert(Videos, v);
}

api Upload: Video {
    method: POST;
    path: "/videos";
}

pipeline Upload:
    StoreVideo
    -> Return;
"#;

/// v0.7 M3 graduated Python's HIR→Python lowering for typed inline
/// handler bodies; M4 does the same for Rust. Neither backend gates a
/// signature-bearing handler anymore — only Kafka still does.
#[test]
fn typed_handler_signature_builds_on_both_backends() {
    let (ir, diags) = compile(TYPED_HANDLER_GATED);
    assert!(
        !diags.has_errors(),
        "a well-typed handler body must pass check: {:?}",
        diags.codes()
    );
    let ir = ir.expect("well-typed program produces IR");

    for backend in backends() {
        ciac_codegen::check_support(backend.as_ref(), &ir).unwrap_or_else(|err| {
            panic!(
                "{} must support a typed inline handler body: {err}",
                backend.id()
            )
        });
    }
}

const EXTERN_HANDLER_GATED: &str = r#"
service ExternHandlerProbe;

record Video {
    id: Uuid;
}

extern handler StoreVideo(v: Video) -> Video;

api Upload: Video {
    method: POST;
    path: "/videos";
}

pipeline Upload:
    StoreVideo
    -> Return;
"#;

/// `extern handler` (a typed signature with no body) passes `ciac check`
/// too, and both backends now emit a typed stub for it the same way
/// they seed classic handlers.
#[test]
fn extern_handler_signature_builds_on_both_backends() {
    let (ir, diags) = compile(EXTERN_HANDLER_GATED);
    assert!(
        !diags.has_errors(),
        "a well-typed extern handler must pass check: {:?}",
        diags.codes()
    );
    let ir = ir.expect("well-typed program produces IR");

    for backend in backends() {
        ciac_codegen::check_support(backend.as_ref(), &ir).unwrap_or_else(|err| {
            panic!(
                "{} must support an extern handler stub: {err}",
                backend.id()
            )
        });
    }
}

#[test]
fn ontology_runtime_kinds_are_supported() {
    // The v0.4 ontology kinds generate real runtime code; they must not
    // trip the gate.
    let source = r#"
service SupportedProbe;

use {
    object_store media S3 { bucket: "b"; }
    email transactional SES;
    search catalog OpenSearch;
    external_http billing { base_url: "https://billing.internal"; }
}

api Ping {
    method: GET;
    path: "/ping";
}
"#;
    let (ir, diags) = compile(source);
    assert!(!diags.has_errors(), "probe compiles: {:?}", diags.codes());
    let ir = ir.expect("probe produces IR");
    for backend in backends() {
        ciac_codegen::check_support(backend.as_ref(), &ir).unwrap_or_else(|err| {
            panic!(
                "{} must support ontology runtime kinds: {err}",
                backend.id()
            )
        });
    }
}
