//! Build-time capability gating: constructs the language accepts but no
//! backend can generate yet must fail `ciac build` (CIAC0011 via
//! `check_support`) while `ciac check` still passes. This is the
//! discipline boundary — nothing compiles into a system that does not
//! actually work.

use ciac_integration_tests::{backends, compile};

const GATED: &str = r#"
service GatedProbe;

use {
    scheduler jobs Cron;
    realtime live WebSocket;
}

api Ping {
    method: GET;
    path: "/ping";
}
"#;

#[test]
fn scheduler_and_realtime_pass_check_but_gate_at_build() {
    let (ir, diags) = compile(GATED);
    assert!(
        !diags.has_errors(),
        "check must accept scheduler/realtime declarations: {:?}",
        diags.codes()
    );
    let ir = ir.expect("gated program still produces IR");

    for backend in backends() {
        let err = ciac_codegen::check_support(backend.as_ref(), &ir)
            .expect_err(&format!("{} must gate scheduler/realtime", backend.id()));
        let message = err.to_string();
        assert!(
            message.contains("scheduler"),
            "{} gating error should name the unsupported construct: {message}",
            backend.id()
        );
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
