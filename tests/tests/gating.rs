//! Build-time capability gating: constructs the language accepts but a
//! backend cannot generate must fail `ciac build` (CIAC0011 via
//! `check_support`) while `ciac check` still passes. Since v0.13 M2 the
//! two first-class backends gate nothing — these probes now prove
//! *support* end to end, and the mechanism stands ready for future
//! providers that arrive engine-by-engine.

use ciac_integration_tests::{compile, full_parity_backends};

const KAFKA_PROBE: &str = r#"
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
fn kafka_generates_on_both_backends() {
    // v0.11 M3 graduated Kafka on Python (aiokafka); v0.13 M2 on Rust
    // (rdkafka vendored). Topics and consumer groups reuse the same
    // subject/queue-group names on both, so a mixed-target system
    // shares one broker correctly.
    let (ir, diags) = compile(KAFKA_PROBE);
    assert!(
        !diags.has_errors(),
        "check must accept kafka declarations: {:?}",
        diags.codes()
    );
    let ir = ir.expect("program produces IR");

    for backend in full_parity_backends() {
        ciac_codegen::check_support(backend.as_ref(), &ir)
            .unwrap_or_else(|err| panic!("{} must support Kafka: {err}", backend.id()));
        let project = backend
            .generate(&ir, &ciac_codegen::GenOptions::default())
            .expect("kafka program generates");
        match backend.id() {
            "python" => {
                let queue_py = project.get("app/queue.py").expect("queue module");
                assert!(queue_py.contains("AIOKafkaProducer"), "{queue_py}");
            }
            "rust" => {
                let queue_rs = project.get("src/queue.rs").expect("queue module");
                assert!(queue_rs.contains("FutureProducer"), "{queue_rs}");
                let cargo = project.get("Cargo.toml").expect("manifest");
                assert!(cargo.contains("rdkafka"), "{cargo}");
                assert!(!cargo.contains("async-nats"), "{cargo}");
            }
            other => panic!("unexpected backend {other}"),
        }
    }
}

const MYSQL_PROBE: &str = r#"
service MysqlProbe;

use {
    db MySQL;
}

record Note {
    id: Uuid;
    title: String;
}

crud Note: Note;
"#;

/// v0.13 M1: MySQL graduated on the Rust backend — per-engine sqlx
/// pools and positional `?` placeholders. Both backends generate.
#[test]
fn mysql_generates_on_both_backends() {
    let (ir, diags) = compile(MYSQL_PROBE);
    assert!(!diags.has_errors(), "probe compiles: {:?}", diags.codes());
    let ir = ir.expect("probe produces IR");

    for backend in full_parity_backends() {
        ciac_codegen::check_support(backend.as_ref(), &ir)
            .unwrap_or_else(|err| panic!("{} must support MySQL: {err}", backend.id()));
        let project = backend
            .generate(&ir, &ciac_codegen::GenOptions::default())
            .expect("mysql program generates");
        if backend.id() == "rust" {
            let store = project
                .get("src/services/note_store.rs")
                .expect("resource store");
            assert!(store.contains("sqlx::MySqlPool"), "{store}");
            assert!(
                store.contains("WHERE id = ?") && !store.contains("$1"),
                "mysql SQL must use positional placeholders: {store}"
            );
            let update_pos = store.find("UPDATE notes SET").expect("update query");
            let binds_after_update = &store[update_pos..];
            let title_bind = binds_after_update.find(".bind(&entity.title)");
            let id_bind = binds_after_update.find(".bind(id)");
            assert!(
                title_bind.unwrap() < id_bind.unwrap(),
                "with `?` placeholders the UPDATE must bind fields before id"
            );
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
    for backend in full_parity_backends() {
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
    for backend in full_parity_backends() {
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

    for backend in full_parity_backends() {
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

    for backend in full_parity_backends() {
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
    for backend in full_parity_backends() {
        ciac_codegen::check_support(backend.as_ref(), &ir).unwrap_or_else(|err| {
            panic!(
                "{} must support ontology runtime kinds: {err}",
                backend.id()
            )
        });
    }
}
