//! v0.8 M4: `tests/system/` — a compose-backed pytest project proving
//! whole-system edges actually work once the real system is running:
//! every cross-service `call` gets a reachability test against the
//! live target, every single-hop publish→consume edge gets a
//! broker-delivery test, every single-hop channel gets a
//! subscribe-and-receive test. Run by `ciac verify --system`
//! (`crates/ciac/src/commands.rs`), never by plain `ciac verify`.
//!
//! Always a Python project regardless of the build target: it's
//! exercising wire-level contracts (HTTP/NATS/WebSocket), not
//! target-language ones, and building this generator twice for no
//! behavioral difference isn't worth the maintenance cost. A
//! `rust`-target system therefore needs Python + `uv` installed to run
//! `ciac verify --system` specifically; plain `ciac verify` needs
//! nothing extra.
//!
//! Scoped, disclosed limits (see `08UpdatePlan.md`'s v0.8 M4 plan):
//! only publish sites owned directly by an `api` pipeline (not a
//! worker's) can be triggered, since triggering a worker-owned publish
//! would require first delivering to *its* upstream stream; and a
//! `call` target whose api requires auth is skipped, since this
//! generator has no credentials to present.

use crate::model::{build_system, SystemModel};
use crate::GenOptions;
use ciac_ir::{FieldType, NormalizedIr, Record, RecordField};

/// An HTTP endpoint, discovered from an api's `publish` step, that can
/// trigger delivery onto the subject it publishes.
struct Producer {
    host_port: u16,
    route: String,
    method_lower: String,
    /// A synthetic JSON body valid for this api's request record
    /// (`"{}"` when the api takes no typed payload).
    sample_json: String,
}

struct CallCheck {
    service: String,
    api: String,
    host_port: u16,
    route: String,
    method_lower: String,
    sample_json: String,
    fields: Vec<String>,
}

struct DeliveryCheck {
    stream: String,
    subject: String,
    producer: Producer,
}

struct ChannelCheck {
    channel: String,
    path: String,
    host_port: u16,
    producer: Producer,
}

/// Builds `tests/system/`'s file set from the validated IR. `None` when
/// the graph has no whole-system edge worth generating a test for.
pub fn build(ir: &NormalizedIr) -> Option<Vec<(String, String)>> {
    let system = build_system(ir, &GenOptions::default());
    let producers = find_producers(ir, &system);

    let call_checks = build_call_checks(ir, &system);
    let delivery_checks = build_delivery_checks(&system, &producers);
    let channel_checks = build_channel_checks(&system, &producers);

    if call_checks.is_empty() && delivery_checks.is_empty() && channel_checks.is_empty() {
        return None;
    }

    let mut files = vec![
        ("conftest.py".to_string(), CONFTEST.to_string()),
        (
            "pyproject.toml".to_string(),
            render_pyproject(!delivery_checks.is_empty(), !channel_checks.is_empty()),
        ),
    ];
    if !call_checks.is_empty() {
        files.push(("test_calls.py".to_string(), render_calls(&call_checks)));
    }
    if !delivery_checks.is_empty() {
        files.push((
            "test_delivery.py".to_string(),
            render_delivery(&delivery_checks),
        ));
    }
    if !channel_checks.is_empty() {
        files.push((
            "test_channels.py".to_string(),
            render_channels(&channel_checks),
        ));
    }
    Some(files)
}

/// Every subject a service's api publishes to directly (top-level
/// `publish` steps only — not nested inside a `match` arm, which keeps
/// this to the "one api, one publish" shape the milestone scopes to).
fn find_producers(ir: &NormalizedIr, system: &SystemModel) -> Vec<(String, Producer)> {
    let mut producers = Vec::new();
    for service in &system.services {
        for api in &service.apis {
            if !api.has_publish_step {
                continue;
            }
            for step in &api.steps {
                if step.kind == "publish" {
                    if let Some(subject) = &step.subject {
                        let sample_json = api
                            .payload
                            .as_ref()
                            .and_then(|p| ir.find_record(&p.class_name))
                            .map(|id| sample_json(ir.record(id)))
                            .unwrap_or_else(|| "{}".to_string());
                        producers.push((
                            subject.clone(),
                            Producer {
                                host_port: service.host_port,
                                route: api.route.clone(),
                                method_lower: api.method_lower.clone(),
                                sample_json,
                            },
                        ));
                    }
                }
            }
        }
    }
    producers
}

fn find_producer_for<'a>(
    producers: &'a [(String, Producer)],
    subject: &str,
) -> Option<&'a Producer> {
    producers.iter().find(|(s, _)| s == subject).map(|(_, p)| p)
}

fn build_call_checks(ir: &NormalizedIr, system: &SystemModel) -> Vec<CallCheck> {
    let mut checks = Vec::new();
    for service in &system.services {
        for target in &service.call_targets {
            let Some(target_ctx) = system
                .services
                .iter()
                .find(|s| s.service_name == target.service)
            else {
                continue;
            };
            for api in &target.apis {
                let Some(target_api) = target_ctx.apis.iter().find(|a| a.name == api.name) else {
                    continue;
                };
                if target_api.has_auth_step || target_api.scope.is_some() {
                    continue; // no credentials to present; disclosed non-goal.
                }
                let Some(payload) = &api.payload else {
                    continue; // untyped JSON payload: nothing to synthesize.
                };
                let Some(record) = ir.find_record(&payload.class_name).map(|id| ir.record(id))
                else {
                    continue;
                };
                checks.push(CallCheck {
                    service: target.service.clone(),
                    api: api.name.clone(),
                    host_port: target_ctx.host_port,
                    route: api.path.clone(),
                    method_lower: api.http_method_lower.clone(),
                    sample_json: sample_json(record),
                    fields: record.fields.iter().map(|f| f.name.clone()).collect(),
                });
            }
        }
    }
    checks
}

fn build_delivery_checks(
    system: &SystemModel,
    producers: &[(String, Producer)],
) -> Vec<DeliveryCheck> {
    let mut checks = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for service in &system.services {
        for worker in &service.workers {
            if !seen.insert(worker.subject.clone()) {
                continue;
            }
            let Some(producer) = find_producer_for(producers, &worker.subject) else {
                continue;
            };
            checks.push(DeliveryCheck {
                stream: worker.name.clone(),
                subject: worker.subject.clone(),
                producer: clone_producer(producer),
            });
        }
    }
    checks
}

fn build_channel_checks(
    system: &SystemModel,
    producers: &[(String, Producer)],
) -> Vec<ChannelCheck> {
    let mut checks = Vec::new();
    for service in &system.services {
        for channel in &service.channels {
            let Some(producer) = find_producer_for(producers, &channel.subject) else {
                continue;
            };
            checks.push(ChannelCheck {
                channel: channel.name.clone(),
                path: channel.path.clone(),
                host_port: service.host_port,
                producer: clone_producer(producer),
            });
        }
    }
    checks
}

fn clone_producer(p: &Producer) -> Producer {
    Producer {
        host_port: p.host_port,
        route: p.route.clone(),
        method_lower: p.method_lower.clone(),
        sample_json: p.sample_json.clone(),
    }
}

/// A total, deterministic synthetic value per [`FieldType`] — enough to
/// pass validation on the other end, not meant to be realistic data.
fn sample_value(ty: &FieldType) -> serde_json::Value {
    use serde_json::json;
    match ty {
        FieldType::Str => json!("test"),
        FieldType::Int => json!(0),
        FieldType::Float => json!(0.0),
        FieldType::Bool => json!(true),
        FieldType::Uuid => json!("00000000-0000-0000-0000-000000000000"),
        FieldType::Timestamp => json!("1970-01-01T00:00:00Z"),
        FieldType::Json => json!({}),
        FieldType::Enum { variants } => {
            json!(variants.first().cloned().unwrap_or_default())
        }
    }
}

fn sample_json(record: &Record) -> String {
    let fields: Vec<(String, serde_json::Value)> = record
        .fields
        .iter()
        .map(|f: &RecordField| (f.name.clone(), sample_value(&f.ty)))
        .collect();
    let object: serde_json::Map<String, serde_json::Value> = fields.into_iter().collect();
    serde_json::to_string(&serde_json::Value::Object(object)).expect("json map always serializes")
}

const CONFTEST: &str = "\"\"\"Pytest configuration. Generated by CIaC.\"\"\"\n\nimport pytest\n\n\n@pytest.fixture\ndef anyio_backend() -> str:\n    return \"asyncio\"\n";

fn render_pyproject(needs_nats: bool, needs_ws: bool) -> String {
    let mut deps = vec!["\"pytest>=8.0\"", "\"anyio>=4.4\"", "\"httpx>=0.27\""];
    if needs_nats {
        deps.push("\"nats-py>=2.7\"");
    }
    if needs_ws {
        deps.push("\"websockets>=12.0\"");
    }
    format!(
        "[project]\nname = \"system-tests\"\nversion = \"0.1.0\"\nrequires-python = \">=3.11\"\ndependencies = [\n{}\n]\n\n[build-system]\nrequires = [\"hatchling\"]\nbuild-backend = \"hatchling.build\"\n\n[tool.hatch.build.targets.wheel]\nbypass-selection = true\n",
        deps.iter()
            .map(|d| format!("    {d},"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn render_calls(checks: &[CallCheck]) -> String {
    let mut out = String::from(
        "\"\"\"Generated by CIaC (v0.8 M4). Call-reachability tests: hits each\ncross-service call target's real endpoint directly (bypassing the\ncaller) against the compose-booted stack, proving the callee's\ncontract is live at the URL/path the caller is configured to reach.\nRequires `ciac verify --system` (docker compose up).\n\"\"\"\n\nimport json\n\nimport httpx\nimport pytest\n\n",
    );
    for check in checks {
        let test_name = format!(
            "test_{}_{}_is_reachable",
            to_snake(&check.service),
            to_snake(&check.api)
        );
        out.push_str(&format!(
            "\n@pytest.mark.anyio\nasync def {test_name}() -> None:\n    payload = json.loads('{sample}')\n    async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as client:\n        response = await client.{method}(\"{route}\", json=payload)\n    assert response.status_code == 200\n    data = response.json()[\"data\"]\n    for field in {fields:?}:\n        assert field in data, f\"{{field}} missing from {{data}}\"\n",
            test_name = test_name,
            sample = check.sample_json,
            port = check.host_port,
            method = check.method_lower,
            route = check.route,
            fields = check.fields,
        ));
    }
    out
}

fn render_delivery(checks: &[DeliveryCheck]) -> String {
    let mut out = String::from(
        "\"\"\"Generated by CIaC (v0.8 M4). Broker-delivery tests: subscribes\nindependently to a stream's subject (outside any queue group, so it\nnever competes with the real consumer's load-balanced subscription),\ntriggers the producing api over real HTTP, and asserts the message\nactually crosses the real broker. Requires `ciac verify --system`.\n\"\"\"\n\nimport asyncio\nimport json\n\nimport httpx\nimport nats\nimport pytest\n\nNATS_URL = \"nats://localhost:4222\"\n\n",
    );
    for check in checks {
        let test_name = format!("test_{}_delivery", to_snake(&check.stream));
        out.push_str(&format!(
            "\n@pytest.mark.anyio\nasync def {test_name}() -> None:\n    client = await nats.connect(NATS_URL)\n    received: asyncio.Queue[bytes] = asyncio.Queue()\n\n    async def _handler(msg) -> None:\n        await received.put(msg.data)\n\n    sub = await client.subscribe(\"{subject}\", cb=_handler)\n    try:\n        payload = json.loads('{sample}')\n        async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as http:\n            response = await http.{method}(\"{route}\", json=payload)\n        assert response.status_code == 200\n        raw = await asyncio.wait_for(received.get(), timeout=10)\n        assert json.loads(raw)\n    finally:\n        await sub.unsubscribe()\n        await client.close()\n",
            test_name = test_name,
            subject = check.subject,
            port = check.producer.host_port,
            method = check.producer.method_lower,
            route = check.producer.route,
            sample = check.producer.sample_json,
        ));
    }
    out
}

fn render_channels(checks: &[ChannelCheck]) -> String {
    let mut out = String::from(
        "\"\"\"Generated by CIaC (v0.8 M4). Channel subscribe-and-receive tests:\nconnects a real WebSocket client to the channel's endpoint, triggers\nthe producing api over real HTTP, and asserts the socket receives the\ndelivered message. Requires `ciac verify --system`.\n\"\"\"\n\nimport asyncio\nimport json\n\nimport httpx\nimport pytest\nimport websockets\n\n",
    );
    for check in checks {
        let test_name = format!("test_{}_channel_delivers", to_snake(&check.channel));
        out.push_str(&format!(
            "\n@pytest.mark.anyio\nasync def {test_name}() -> None:\n    uri = \"ws://localhost:{ws_port}{path}\"\n    async with websockets.connect(uri) as socket:\n        payload = json.loads('{sample}')\n        async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as http:\n            response = await http.{method}(\"{route}\", json=payload)\n        assert response.status_code == 200\n        raw = await asyncio.wait_for(socket.recv(), timeout=10)\n        assert json.loads(raw)\n",
            test_name = test_name,
            ws_port = check.host_port,
            path = check.path,
            port = check.producer.host_port,
            method = check.producer.method_lower,
            route = check.producer.route,
            sample = check.producer.sample_json,
        ));
    }
    out
}

fn to_snake(name: &str) -> String {
    use heck::ToSnakeCase;
    name.to_snake_case()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ciac_ir::RecordKind;

    fn compile(src: &str) -> NormalizedIr {
        let mut sources = ciac_diagnostics::SourceMap::new();
        let file = sources.add_file("test.ciac", src);
        let mut diags = ciac_diagnostics::Diagnostics::new();
        let program = ciac_syntax::parse(src, file, &mut diags);
        ciac_sema::analyze(&program, &mut diags)
            .unwrap_or_else(|| panic!("compiles: {:?}", diags.codes()))
    }

    #[test]
    fn no_qualifying_edges_builds_nothing() {
        let ir = compile("service Notes;\nuse { auth JWT; db Postgres; }\ncrud Note;\n");
        assert!(build(&ir).is_none());
    }

    #[test]
    fn call_and_single_hop_delivery_edges_build_tests() {
        let src = r#"
project MediaSystem;
record Video { id: Uuid; title: String; status: enum { Ready, Failed }; }
stream Uploaded: Video;

service Billing {
    api Charge: Video { method: POST; path: "/charge"; }
    pipeline Charge: CapturePayment -> Return;
}

service UploadApi {
    use { queue bus NATS; }
    api Upload: Video { method: PUT; path: "/videos"; }
    pipeline Upload: call Billing.Charge -> StoreVideo -> publish Uploaded -> Return;
}

service Transcoder {
    use { queue bus NATS; }
    worker Transcode on Uploaded;
    pipeline Transcode: TranscodeVideo;
}
"#;
        let ir = compile(src);
        let files = build(&ir).expect("call + single-hop delivery edges qualify");
        let names: Vec<&str> = files.iter().map(|(p, _)| p.as_str()).collect();
        assert!(names.contains(&"test_calls.py"));
        assert!(names.contains(&"test_delivery.py"));
        assert!(!names.contains(&"test_channels.py"));
    }

    #[test]
    fn sample_json_covers_every_field_type() {
        let record = Record {
            name: "Sample".into(),
            kind: RecordKind::Data,
            fields: vec![
                RecordField {
                    name: "s".into(),
                    ty: FieldType::Str,
                },
                RecordField {
                    name: "i".into(),
                    ty: FieldType::Int,
                },
                RecordField {
                    name: "f".into(),
                    ty: FieldType::Float,
                },
                RecordField {
                    name: "b".into(),
                    ty: FieldType::Bool,
                },
                RecordField {
                    name: "u".into(),
                    ty: FieldType::Uuid,
                },
                RecordField {
                    name: "t".into(),
                    ty: FieldType::Timestamp,
                },
                RecordField {
                    name: "j".into(),
                    ty: FieldType::Json,
                },
                RecordField {
                    name: "e".into(),
                    ty: FieldType::Enum {
                        variants: vec!["A".into(), "B".into()],
                    },
                },
            ],
        };
        let json = sample_json(&record);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["s"], "test");
        assert_eq!(parsed["i"], 0);
        assert_eq!(parsed["f"], 0.0);
        assert_eq!(parsed["b"], true);
        assert_eq!(parsed["u"], "00000000-0000-0000-0000-000000000000");
        assert_eq!(parsed["t"], "1970-01-01T00:00:00Z");
        assert_eq!(parsed["j"], serde_json::json!({}));
        assert_eq!(parsed["e"], "A");
    }

    /// JSON's `true`/`false`/lowercase-`null` aren't valid Python
    /// identifiers (`True`/`False`/`None` are) — a `Bool` field's sample
    /// value must never be spliced into generated source as a bare
    /// Python literal, only as a quoted string handed to `json.loads`.
    #[test]
    fn bool_sample_value_is_never_a_bare_python_literal() {
        let check = CallCheck {
            service: "Billing".into(),
            api: "Charge".into(),
            host_port: 8000,
            route: "/charge".into(),
            method_lower: "post".into(),
            sample_json: "{\"active\":true}".into(),
            fields: vec!["active".into()],
        };
        let rendered = render_calls(&[check]);
        assert!(
            rendered.contains("json.loads('{\"active\":true}')"),
            "bool sample must stay inside a quoted json.loads(...) call:\n{rendered}"
        );
        assert!(
            !rendered.contains("payload = {\"active\":true}"),
            "bool sample must never be spliced as a bare Python dict literal:\n{rendered}"
        );
    }
}
