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
    /// `nats` | `kafka` — selects the broker client (v0.11 M3).
    engine: String,
}

struct ChannelCheck {
    channel: String,
    path: String,
    host_port: u16,
    producer: Producer,
}

/// A typed, auth-less CRUD resource whose persistence can be verified
/// through a second, independent connection (v0.9 M2): create via the
/// real HTTP api, then read the row back directly from Postgres —
/// and, when the resource caches reads, check the cache entry directly
/// in Redis after a GET.
struct CapabilityCheck {
    service: String,
    resource: String,
    /// Route base and table name, e.g. `notes`.
    plural: String,
    /// The owning service's HTTP host port.
    host_port: u16,
    /// Create body with the server-generated `id` excluded.
    sample_json: String,
    /// Direct asyncpg DSN via the db instance's compose host port.
    db_dsn: String,
    /// `postgres` | `mysql` — selects the direct-connection client.
    db_engine: String,
    /// Compose-mapped host port + database name, for clients whose
    /// connect API takes parts rather than a DSN (aiomysql).
    db_host_port: u16,
    db_name: String,
    /// Direct Redis probe when the resource caches reads.
    cache: Option<CacheProbe>,
}

struct CacheProbe {
    host_port: u16,
    redis_db: u32,
}

/// A `tracing OpenTelemetry`-enabled api whose pipeline crosses a
/// `call` and/or a `publish`→worker hop (v0.15 M3/M4): the trace it
/// produces should be one continuous span tree from the entry request
/// through every hop, not a dangling root span per service.
struct TraceContinuityCheck {
    /// Test name component, e.g. `checkout_submit`.
    name: String,
    producer: Producer,
    /// OTel `service.name` the producing api's own service reports as
    /// — what to query Jaeger for.
    jaeger_service: String,
    /// Minimum span count the trace must contain to call the chain
    /// proven: 2 for a single hop (call *or* publish/consume), 3 for
    /// both in the same pipeline.
    min_spans: u32,
}

/// A scoped CRUD resource on a `users Keycloak` service (v0.15 M6):
/// unlike a plain `has_auth` resource (skipped by
/// [`build_capability_checks`], "no credentials to present"), a real
/// IdP is running, so `scripts/token.sh` can mint real
/// `dev-admin`/`dev-user` tokens and the 403-without/200-with claim
/// from v0.14 M6 gets a live assertion instead of an in-process one.
struct ScopeCheck {
    service: String,
    resource: String,
    plural: String,
    host_port: u16,
    sample_json: String,
    read_scope: Option<String>,
    write_scope: Option<String>,
}

/// Builds `tests/system/`'s file set from the validated IR. `None` when
/// the graph has no whole-system edge worth generating a test for.
pub fn build(ir: &NormalizedIr) -> Option<Vec<(String, String)>> {
    let system = build_system(ir, &GenOptions::default());
    let producers = find_producers(ir, &system);

    let call_checks = build_call_checks(ir, &system);
    let delivery_checks = build_delivery_checks(&system, &producers);
    let channel_checks = build_channel_checks(&system, &producers);
    let capability_checks = build_capability_checks(ir, &system);
    let trace_checks = build_trace_continuity_checks(ir, &system);
    let scope_checks = build_scope_checks(ir, &system);

    if call_checks.is_empty()
        && delivery_checks.is_empty()
        && channel_checks.is_empty()
        && capability_checks.is_empty()
        && trace_checks.is_empty()
        && scope_checks.is_empty()
    {
        return None;
    }

    let mut files = vec![
        ("conftest.py".to_string(), CONFTEST.to_string()),
        (
            "pyproject.toml".to_string(),
            render_pyproject(&PyprojectDeps {
                nats: delivery_checks.iter().any(|c| c.engine != "kafka"),
                kafka: delivery_checks.iter().any(|c| c.engine == "kafka"),
                websockets: !channel_checks.is_empty(),
                asyncpg: capability_checks.iter().any(|c| c.db_engine != "mysql"),
                aiomysql: capability_checks.iter().any(|c| c.db_engine == "mysql"),
                redis: capability_checks.iter().any(|c| c.cache.is_some()),
            }),
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
    if !capability_checks.is_empty() {
        files.push((
            "test_capabilities.py".to_string(),
            render_capabilities(&capability_checks),
        ));
    }
    if !trace_checks.is_empty() {
        files.push((
            "test_trace_continuity.py".to_string(),
            render_trace_continuity(&trace_checks),
        ));
    }
    if !scope_checks.is_empty() {
        files.push((
            "test_scopes.py".to_string(),
            render_scope_tests(&scope_checks),
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
                engine: system
                    .queue_engine
                    .clone()
                    .unwrap_or_else(|| "nats".to_owned()),
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

/// The instance a resource's `key_arg` binds to: empty means the
/// implicit `default` instance; otherwise the quoted instance name.
fn bound_instance<'a>(
    instances: &'a [crate::model::InstanceCtx],
    key_arg: &str,
) -> Option<&'a crate::model::InstanceCtx> {
    if key_arg.is_empty() {
        instances.iter().find(|inst| inst.is_default)
    } else {
        let name = key_arg.trim_matches('"');
        instances.iter().find(|inst| inst.snake == name)
    }
}

fn build_capability_checks(ir: &NormalizedIr, system: &SystemModel) -> Vec<CapabilityCheck> {
    let mut checks = Vec::new();
    for service in &system.services {
        for resource in &service.resources {
            if resource.has_auth {
                continue; // no credentials to present; same as call checks.
            }
            let Some(record_ctx) = &resource.record else {
                continue; // untyped keyed-document resource: no columns to verify.
            };
            let Some(db) = bound_instance(&service.db_instances, &resource.db_session.key_arg)
            else {
                continue;
            };
            let Some(record) = ir.find_record(&record_ctx.name).map(|id| ir.record(id)) else {
                continue;
            };
            let cache = if resource.has_cache {
                // `cache_expr` is `get_cache()` or `get_cache("name")`.
                let key_arg = resource
                    .cache_expr
                    .trim_start_matches("get_cache(")
                    .trim_end_matches(')');
                bound_instance(&service.cache_instances, key_arg).map(|inst| CacheProbe {
                    host_port: inst.host_port,
                    redis_db: inst.redis_db,
                })
            } else {
                None
            };
            if db.db_engine == "sqlite" {
                // The database is a file inside the app container --
                // there is no host port for a second, independent
                // connection to prove persistence through (v0.13 M3).
                // The HTTP CRUD behavior is still covered by the
                // generated per-project tests.
                continue;
            }
            checks.push(CapabilityCheck {
                service: service.service_name.clone(),
                resource: resource.snake.clone(),
                plural: resource.plural.clone(),
                host_port: service.host_port,
                sample_json: sample_json_without_id(record),
                db_dsn: if db.db_engine == "mysql" {
                    format!(
                        "mysql://root:root@localhost:{}/{}",
                        db.host_port, db.db_name
                    )
                } else {
                    format!(
                        "postgresql://postgres:postgres@localhost:{}/{}",
                        db.host_port, db.db_name
                    )
                },
                db_engine: db.db_engine.clone(),
                db_host_port: db.host_port,
                db_name: db.db_name.clone(),
                cache,
            });
        }
    }
    checks
}

/// One check per tracing-enabled api whose pipeline crosses a `call`
/// and/or a `publish` step — the shape a trace-continuity claim is
/// actually about. An api with neither (plain request/response) has
/// nothing cross-service to prove here even if its service declares
/// `tracing`.
fn build_trace_continuity_checks(
    ir: &NormalizedIr,
    system: &SystemModel,
) -> Vec<TraceContinuityCheck> {
    let mut checks = Vec::new();
    for service in &system.services {
        if !service.has_tracing {
            continue;
        }
        for api in &service.apis {
            let has_call = api.steps.iter().any(|s| s.kind == "call");
            if !has_call && !api.has_publish_step {
                continue;
            }
            let sample_json = api
                .payload
                .as_ref()
                .and_then(|p| ir.find_record(&p.class_name))
                .map(|id| sample_json(ir.record(id)))
                .unwrap_or_else(|| "{}".to_string());
            checks.push(TraceContinuityCheck {
                name: format!(
                    "{}_{}",
                    to_snake(&service.service_name),
                    to_snake(&api.name)
                ),
                producer: Producer {
                    host_port: service.host_port,
                    route: api.route.clone(),
                    method_lower: api.method_lower.clone(),
                    sample_json,
                },
                jaeger_service: service.service_name.clone(),
                min_spans: if has_call && api.has_publish_step {
                    3
                } else {
                    2
                },
            });
        }
    }
    checks
}

/// One check per scoped CRUD resource on a service whose `auth
/// OAuth2` issuer resolves to a live `users Keycloak` container --
/// the only case `scripts/token.sh` can mint a real token against.
/// Resources with neither `read_scope` nor `write_scope` have nothing
/// scope-specific to prove here even if the service has `users`.
fn build_scope_checks(ir: &NormalizedIr, system: &SystemModel) -> Vec<ScopeCheck> {
    let mut checks = Vec::new();
    for service in &system.services {
        if !(service.auth_scheme == "oauth2" && service.has_users) {
            continue; // no live IdP to mint real tokens from.
        }
        for resource in &service.resources {
            if resource.read_scope.is_none() && resource.write_scope.is_none() {
                continue;
            }
            let sample_json = resource
                .record
                .as_ref()
                .and_then(|record_ctx| ir.find_record(&record_ctx.name))
                .map(|id| sample_json_without_id(ir.record(id)))
                .unwrap_or_else(|| "{}".to_string());
            checks.push(ScopeCheck {
                service: service.service_name.clone(),
                resource: resource.snake.clone(),
                plural: resource.plural.clone(),
                host_port: service.host_port,
                sample_json,
                read_scope: resource.read_scope.clone(),
                write_scope: resource.write_scope.clone(),
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
        // v0.16 M4: a to-one reference is a plain id on the wire (see
        // `ciac_codegen::model::FieldTypeKind::Reference`); a fixed
        // placeholder is exactly as synthetic as every other sample
        // value here. Real per-build topological ids (create the
        // target first, thread its actual id through) are v0.16 M7's
        // flagship work, not this generator's job.
        FieldType::Reference {
            cardinality: ciac_ir::Cardinality::One,
            ..
        } => json!("00000000-0000-0000-0000-000000000000"),
        FieldType::Reference {
            cardinality: ciac_ir::Cardinality::Many,
            ..
        } => unreachable!("many-relation codegen is gated until v0.16 M5/M6 land"),
    }
}

/// The wire property name for a field (v0.16 M4): a to-one reference's
/// declared name (`customer`) maps to `customer_id`, matching
/// `ciac_codegen::model::build_record`'s field-name computation exactly
/// — kept as one small helper rather than risking the two drifting.
fn wire_field_name(field: &RecordField) -> String {
    match &field.ty {
        FieldType::Reference {
            cardinality: ciac_ir::Cardinality::One,
            ..
        } => format!("{}_id", field.name),
        _ => field.name.clone(),
    }
}

/// A to-many reference has no wire exposure yet (v0.16 M4; it isn't
/// part of a record's own row — see `build_record`), so it's excluded
/// from the sample payload rather than sampled at all.
fn is_wire_field(field: &RecordField) -> bool {
    !matches!(
        field.ty,
        FieldType::Reference {
            cardinality: ciac_ir::Cardinality::Many,
            ..
        }
    )
}

fn sample_json(record: &Record) -> String {
    let fields: Vec<(String, serde_json::Value)> = record
        .fields
        .iter()
        .filter(|f| is_wire_field(f))
        .map(|f: &RecordField| (wire_field_name(f), sample_value(&f.ty)))
        .collect();
    let object: serde_json::Map<String, serde_json::Value> = fields.into_iter().collect();
    serde_json::to_string(&serde_json::Value::Object(object)).expect("json map always serializes")
}

/// Create body for a typed CRUD resource: the `id` primary key is
/// always server-generated, so the `<Name>In` schema excludes it.
fn sample_json_without_id(record: &Record) -> String {
    let object: serde_json::Map<String, serde_json::Value> = record
        .fields
        .iter()
        .filter(|f| f.name != "id" && is_wire_field(f))
        .map(|f| (wire_field_name(f), sample_value(&f.ty)))
        .collect();
    serde_json::to_string(&serde_json::Value::Object(object)).expect("json map always serializes")
}

const CONFTEST: &str = "\"\"\"Pytest configuration. Generated by CIaC.\"\"\"\n\nimport pytest\n\n\n@pytest.fixture\ndef anyio_backend() -> str:\n    return \"asyncio\"\n";

struct PyprojectDeps {
    nats: bool,
    kafka: bool,
    websockets: bool,
    asyncpg: bool,
    aiomysql: bool,
    redis: bool,
}

fn render_pyproject(needs: &PyprojectDeps) -> String {
    let mut deps = vec!["\"pytest>=8.0\"", "\"anyio>=4.4\"", "\"httpx>=0.27\""];
    if needs.nats {
        deps.push("\"nats-py>=2.7\"");
    }
    if needs.kafka {
        deps.push("\"aiokafka>=0.11\"");
    }
    if needs.websockets {
        deps.push("\"websockets>=12.0\"");
    }
    if needs.asyncpg {
        deps.push("\"asyncpg>=0.29\"");
    }
    if needs.aiomysql {
        deps.push("\"aiomysql>=0.2\"");
    }
    if needs.redis {
        deps.push("\"redis>=5.0\"");
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
    let any_nats = checks.iter().any(|c| c.engine != "kafka");
    let any_kafka = checks.iter().any(|c| c.engine == "kafka");
    let mut out = String::from(
        "\"\"\"Generated by CIaC (v0.8 M4, engines v0.11 M3). Broker-delivery tests:\nsubscribes independently to a stream's subject/topic (outside any\nqueue/consumer group, so it never competes with the real consumer),\ntriggers the producing api over real HTTP, and asserts the message\nactually crosses the real broker. Requires `ciac verify --system`.\n\"\"\"\n\nimport asyncio\nimport json\n\nimport httpx\nimport pytest\n",
    );
    if any_nats {
        out.push_str("import nats\n\nNATS_URL = \"nats://localhost:4222\"\n");
    }
    if any_kafka {
        // 29092 is the broker's EXTERNAL listener (v0.13 M2): host
        // clients on 9092 would be redirected to the in-network
        // advertised name `queue`, which doesn't resolve out here.
        out.push_str("from aiokafka import AIOKafkaConsumer\n\nKAFKA_URL = \"localhost:29092\"\n");
    }
    for check in checks {
        let test_name = format!("test_{}_delivery", to_snake(&check.stream));
        if check.engine == "kafka" {
            out.push_str(&format!(
                "\n\n@pytest.mark.anyio\nasync def {test_name}() -> None:\n    consumer = AIOKafkaConsumer(\"{subject}\", bootstrap_servers=KAFKA_URL)\n    await consumer.start()\n    try:\n        payload = json.loads('{sample}')\n        async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as http:\n            response = await http.{method}(\"{route}\", json=payload)\n        assert response.status_code == 200\n\n        async def _next() -> bytes:\n            async for message in consumer:\n                return message.value\n        raw = await asyncio.wait_for(_next(), timeout=15)\n        assert raw is not None and json.loads(raw)\n    finally:\n        await consumer.stop()\n",
                test_name = test_name,
                subject = check.subject,
                port = check.producer.host_port,
                method = check.producer.method_lower,
                route = check.producer.route,
                sample = check.producer.sample_json,
            ));
            continue;
        }
        out.push_str(&format!(
            "\n\n@pytest.mark.anyio\nasync def {test_name}() -> None:\n    client = await nats.connect(NATS_URL)\n    received: asyncio.Queue[bytes] = asyncio.Queue()\n\n    async def _handler(msg) -> None:\n        await received.put(msg.data)\n\n    sub = await client.subscribe(\"{subject}\", cb=_handler)\n    try:\n        payload = json.loads('{sample}')\n        async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as http:\n            response = await http.{method}(\"{route}\", json=payload)\n        assert response.status_code == 200\n        raw = await asyncio.wait_for(received.get(), timeout=10)\n        assert json.loads(raw)\n    finally:\n        await sub.unsubscribe()\n        await client.close()\n",
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

fn render_capabilities(checks: &[CapabilityCheck]) -> String {
    let any_cache = checks.iter().any(|c| c.cache.is_some());
    let any_pg = checks.iter().any(|c| c.db_engine != "mysql");
    let any_mysql = checks.iter().any(|c| c.db_engine == "mysql");
    let mut out = String::from(
        "\"\"\"Generated by CIaC (v0.9 M2, engines v0.11 M1). Capability round-trip\ntests: creates a record through the real HTTP api, then reads it back\nthrough a second, independent client connection (asyncpg/aiomysql\nstraight into the database; a direct Redis client for cached reads) —\nproving the write actually persisted to the infrastructure, not just\nto app-process state. Requires `ciac verify --system`.\n\"\"\"\n\nimport json\n\nimport httpx\nimport pytest\n",
    );
    if any_pg {
        out.push_str("import asyncpg\n");
    }
    if any_mysql {
        out.push_str("import aiomysql\n");
    }
    if any_cache {
        out.push_str("import redis.asyncio as aioredis\n");
    }
    for check in checks {
        let service_snake = to_snake(&check.service);
        if check.db_engine == "mysql" {
            out.push_str(&format!(
                "\n\n@pytest.mark.anyio\nasync def test_{service_snake}_{resource}_persists_to_mysql() -> None:\n    payload = json.loads('{sample}')\n    async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as client:\n        response = await client.post(\"/{plural}\", json=payload)\n    assert response.status_code == 201, response.text\n    row_id = response.json()[\"id\"]\n\n    conn = await aiomysql.connect(\n        host=\"localhost\", port={db_port}, user=\"root\", password=\"root\", db=\"{db_name}\"\n    )\n    try:\n        async with conn.cursor() as cur:\n            await cur.execute(\"SELECT id FROM {plural} WHERE id = %s\", (row_id,))\n            row = await cur.fetchone()\n    finally:\n        conn.close()\n    assert row is not None, f\"created {{row_id}} not found in mysql\"\n",
                service_snake = service_snake,
                resource = check.resource,
                sample = check.sample_json,
                port = check.host_port,
                plural = check.plural,
                db_port = check.db_host_port,
                db_name = check.db_name,
            ));
        } else {
            out.push_str(&format!(
                "\n\n@pytest.mark.anyio\nasync def test_{service_snake}_{resource}_persists_to_postgres() -> None:\n    payload = json.loads('{sample}')\n    async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as client:\n        response = await client.post(\"/{plural}\", json=payload)\n    assert response.status_code == 201, response.text\n    row_id = response.json()[\"id\"]\n\n    conn = await asyncpg.connect(\"{dsn}\")\n    try:\n        row = await conn.fetchrow(\"SELECT id FROM {plural} WHERE id = $1\", row_id)\n    finally:\n        await conn.close()\n    assert row is not None, f\"created {{row_id}} not found in postgres\"\n",
                service_snake = service_snake,
                resource = check.resource,
                sample = check.sample_json,
                port = check.host_port,
                plural = check.plural,
                dsn = check.db_dsn,
            ));
        }
        if let Some(cache) = &check.cache {
            out.push_str(&format!(
                "\n\n@pytest.mark.anyio\nasync def test_{service_snake}_{resource}_read_populates_redis() -> None:\n    payload = json.loads('{sample}')\n    async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as client:\n        response = await client.post(\"/{plural}\", json=payload)\n        assert response.status_code == 201, response.text\n        row_id = response.json()[\"id\"]\n        got = await client.get(f\"/{plural}/{{row_id}}\")\n    assert got.status_code == 200, got.text\n\n    client = aioredis.Redis(host=\"localhost\", port={cache_port}, db={redis_db})\n    try:\n        cached = await client.get(f\"{plural}:{{row_id}}\")\n    finally:\n        await client.aclose()\n    assert cached is not None, f\"read of {{row_id}} did not populate the cache\"\n",
                service_snake = service_snake,
                resource = check.resource,
                sample = check.sample_json,
                port = check.host_port,
                plural = check.plural,
                cache_port = cache.host_port,
                redis_db = cache.redis_db,
            ));
        }
    }
    out
}

/// Generated by CIaC (v0.15 M3/M4). Trace-continuity tests: hits an
/// edge-bearing route over real HTTP, then polls Jaeger's query API
/// (the queryable backend behind the otel-collector every traced
/// service exports to) for a trace reported under that service's
/// name, and asserts the returned trace contains enough spans for the
/// whole call/publish/consume chain to plausibly be one trace, not a
/// dangling root span per hop. Requires `ciac verify --system` with
/// `tracing OpenTelemetry` declared (otel-collector + Jaeger up).
fn render_trace_continuity(checks: &[TraceContinuityCheck]) -> String {
    let mut out = String::from(
        "\"\"\"Generated by CIaC (v0.15 M3/M4). Trace-continuity tests: hits an\nedge-bearing route over real HTTP, then polls Jaeger's query API (the\nqueryable backend behind the otel-collector every traced service\nexports to) for a trace reported under that service's name, and\nasserts the returned trace has enough spans for the whole\ncall/publish/consume chain to be one trace, not a dangling root span\nper hop. Requires `ciac verify --system` with `tracing OpenTelemetry`\ndeclared (otel-collector + Jaeger up).\n\"\"\"\n\nimport asyncio\nimport json\n\nimport httpx\nimport pytest\n\nJAEGER_URL = \"http://localhost:16686\"\n",
    );
    for check in checks {
        out.push_str(&format!(
            "\n\n@pytest.mark.anyio\nasync def test_trace_{name}_spans_the_full_hop() -> None:\n    payload = json.loads('{sample}')\n    async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as client:\n        response = await client.{method}(\"{route}\", json=payload)\n    assert response.status_code == 200\n\n    deadline = asyncio.get_event_loop().time() + 20\n    trace = None\n    async with httpx.AsyncClient(base_url=JAEGER_URL) as jaeger:\n        while asyncio.get_event_loop().time() < deadline:\n            resp = await jaeger.get(\"/api/traces\", params={{\"service\": \"{jaeger_service}\", \"limit\": 5}})\n            if resp.status_code == 200:\n                traces = resp.json().get(\"data\", [])\n                if traces:\n                    trace = traces[0]\n                    break\n            await asyncio.sleep(1)\n    assert trace is not None, \"no trace reported to Jaeger for {jaeger_service}\"\n    span_count = len(trace[\"spans\"])\n    assert span_count >= {min_spans}, (\n        f\"expected the request/call/publish/consume chain to produce at least \"\n        f\"{min_spans} spans under one trace id, got {{span_count}}\"\n    )\n",
            name = check.name,
            sample = check.producer.sample_json,
            port = check.producer.host_port,
            method = check.producer.method_lower,
            route = check.producer.route,
            jaeger_service = check.jaeger_service,
            min_spans = check.min_spans,
        ));
    }
    out
}

/// Generated by CIaC (v0.15 M6). Scope-enforcement tests against a
/// live IdP: mints real access tokens from the dev Keycloak realm via
/// `scripts/token.sh` (the resource-owner-password-credentials
/// grant), then asserts a request with no token is rejected, one with
/// a token that lacks the required scope is rejected, and one with a
/// token that carries it succeeds -- the 403-without/200-with claim
/// from v0.14 M6, now against a real IdP instead of a locally-signed
/// JWT (only possible for the `jwt` scheme; see `scope_tests.rs.j2`/
/// `test_smoke.py.j2`'s `jwt`-only gate). Requires `ciac verify
/// --system` with `users Keycloak` declared (Keycloak up, realm
/// imported).
fn render_scope_tests(checks: &[ScopeCheck]) -> String {
    let mut out = String::from(
        "\"\"\"Generated by CIaC (v0.15 M6). Scope-enforcement tests against a\nlive IdP: mints real access tokens from the dev Keycloak realm via\n`scripts/token.sh` (password grant), then asserts a request with no\ntoken is rejected, one with a token lacking the required scope is\nrejected, and one with a token that carries it succeeds. Requires\n`ciac verify --system` with `users Keycloak` declared (Keycloak up,\nrealm imported).\n\"\"\"\n\nimport json\nimport subprocess\nfrom pathlib import Path\n\nimport httpx\nimport pytest\n\nTOKEN_SCRIPT = Path(__file__).resolve().parents[2] / \"scripts\" / \"token.sh\"\n\n\ndef _token(user: str, scope: str) -> str:\n    result = subprocess.run(\n        [\"bash\", str(TOKEN_SCRIPT), user, scope],\n        capture_output=True,\n        text=True,\n        check=True,\n    )\n    return result.stdout.strip()\n",
    );
    for check in checks {
        let service_snake = to_snake(&check.service);
        if let Some(read_scope) = &check.read_scope {
            out.push_str(&format!(
                "\n\n@pytest.mark.anyio\nasync def test_{service_snake}_{resource}_read_requires_scope() -> None:\n    async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as client:\n        anon = await client.get(\"/{plural}\")\n        assert anon.status_code in (401, 403), anon.text\n\n        no_scope = _token(\"dev-user\", \"\")\n        forbidden = await client.get(\n            \"/{plural}\", headers={{\"Authorization\": f\"Bearer {{no_scope}}\"}}\n        )\n        assert forbidden.status_code == 403, forbidden.text\n\n        scoped = _token(\"dev-admin\", \"{scope}\")\n        allowed = await client.get(\n            \"/{plural}\", headers={{\"Authorization\": f\"Bearer {{scoped}}\"}}\n        )\n        assert allowed.status_code not in (401, 403), allowed.text\n",
                service_snake = service_snake,
                resource = check.resource,
                port = check.host_port,
                plural = check.plural,
                scope = read_scope,
            ));
        }
        if let Some(write_scope) = &check.write_scope {
            out.push_str(&format!(
                "\n\n@pytest.mark.anyio\nasync def test_{service_snake}_{resource}_write_requires_scope() -> None:\n    payload = json.loads('{sample}')\n    async with httpx.AsyncClient(base_url=\"http://localhost:{port}\") as client:\n        anon = await client.post(\"/{plural}\", json=payload)\n        assert anon.status_code in (401, 403), anon.text\n\n        no_scope = _token(\"dev-user\", \"\")\n        forbidden = await client.post(\n            \"/{plural}\", json=payload, headers={{\"Authorization\": f\"Bearer {{no_scope}}\"}}\n        )\n        assert forbidden.status_code == 403, forbidden.text\n\n        scoped = _token(\"dev-admin\", \"{scope}\")\n        allowed = await client.post(\n            \"/{plural}\", json=payload, headers={{\"Authorization\": f\"Bearer {{scoped}}\"}}\n        )\n        assert allowed.status_code not in (401, 403), allowed.text\n",
                service_snake = service_snake,
                resource = check.resource,
                sample = check.sample_json,
                port = check.host_port,
                plural = check.plural,
                scope = write_scope,
            ));
        }
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
    fn authless_typed_crud_generates_capability_round_trips() {
        let ir = compile(
            "service Catalog;\nuse { db Postgres; cache Redis; }\nrecord Item { id: Uuid; name: String; }\ncrud Item: Item;\n",
        );
        let files = build(&ir).expect("auth-less typed crud qualifies");
        let caps = files
            .iter()
            .find(|(p, _)| p == "test_capabilities.py")
            .map(|(_, c)| c)
            .expect("test_capabilities.py generated");
        assert!(
            caps.contains("test_catalog_item_persists_to_postgres"),
            "{caps}"
        );
        assert!(
            caps.contains("test_catalog_item_read_populates_redis"),
            "{caps}"
        );
        // Second, independent connections: direct DSN + direct Redis.
        assert!(
            caps.contains("postgresql://postgres:postgres@localhost:5432/catalog"),
            "{caps}"
        );
        assert!(caps.contains("port=6379, db=0"), "{caps}");
        // Create body excludes the server-generated id.
        assert!(caps.contains(r#"json.loads('{"name":"test"}')"#), "{caps}");
        let (_, pyproject) = files
            .iter()
            .find(|(p, _)| p == "pyproject.toml")
            .expect("pyproject");
        assert!(pyproject.contains("asyncpg"), "{pyproject}");
        assert!(pyproject.contains("redis"), "{pyproject}");
    }

    #[test]
    fn auth_gated_typed_crud_generates_no_capability_tests() {
        let ir = compile(
            "service Catalog;\nuse { auth JWT; db Postgres; }\nrecord Item { id: Uuid; name: String; }\ncrud Item: Item;\n",
        );
        assert!(
            build(&ir).is_none(),
            "auth-gated crud has no credentials to present"
        );
    }

    #[test]
    fn scoped_oauth2_resource_without_users_builds_no_scope_tests() {
        let ir = compile(
            "service Accounts;\nuse { db Postgres; auth OAuth2 { issuer: \"https://real-idp.example\"; } }\nrecord Account { id: Uuid; email: String; }\ncrud Account: Account { read_scope: \"accounts:read\"; write_scope: \"accounts:write\"; }\n",
        );
        // No `users Keycloak` -- no live IdP to mint a real token from,
        // and nothing else here qualifies (auth-gated crud is also
        // skipped by capability checks), so there's nothing to build.
        assert!(build(&ir).is_none());
    }

    #[test]
    fn scoped_oauth2_resource_with_users_builds_live_scope_tests() {
        let ir = compile(
            "service Accounts;\nuse { db Postgres; auth OAuth2; users Keycloak; }\nrecord Account { id: Uuid; email: String; }\ncrud Account: Account { read_scope: \"accounts:read\"; write_scope: \"accounts:write\"; }\n",
        );
        let files = build(&ir).expect("scoped resource on a users-backed oauth2 service qualifies");
        let (_, scopes) = files
            .iter()
            .find(|(p, _)| p == "test_scopes.py")
            .expect("test_scopes.py generated");
        assert!(
            scopes.contains("test_accounts_account_read_requires_scope"),
            "{scopes}"
        );
        assert!(
            scopes.contains("test_accounts_account_write_requires_scope"),
            "{scopes}"
        );
        assert!(
            scopes.contains(r#"_token("dev-admin", "accounts:read")"#),
            "{scopes}"
        );
        assert!(
            scopes.contains(r#"_token("dev-admin", "accounts:write")"#),
            "{scopes}"
        );
        assert!(scopes.contains(r#"_token("dev-user", "")"#), "{scopes}");
        assert!(scopes.contains("parents[2]"), "{scopes}");
        assert!(
            scopes.contains(r#"json.loads('{"email":"test"}')"#),
            "{scopes}"
        );
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
