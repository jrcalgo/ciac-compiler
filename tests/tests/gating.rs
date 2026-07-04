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
fn kafka_passes_check_but_gates_at_build() {
    let (ir, diags) = compile(KAFKA_GATED);
    assert!(
        !diags.has_errors(),
        "check must accept kafka declarations: {:?}",
        diags.codes()
    );
    let ir = ir.expect("gated program still produces IR");

    for backend in backends() {
        let err = ciac_codegen::check_support(backend.as_ref(), &ir)
            .expect_err(&format!("{} must gate Kafka", backend.id()));
        let message = err.to_string();
        assert!(
            message.contains("queue default Kafka"),
            "{} gating error should name the unsupported construct: {message}",
            backend.id()
        );
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
