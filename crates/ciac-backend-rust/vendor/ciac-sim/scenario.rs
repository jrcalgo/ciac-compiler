//! Portable scenarios (17UpdatePlan.md Pillar 3): versioned JSON, not
//! target code and not a general-purpose scenario language. The closed
//! action set is `request`/`publish`/`advance`/`drain`/`expect`.
//!
//! M2 ships the schema, parsing, and structural (not yet preflight-
//! against-a-plan) validation. Resolving a scenario's names against a
//! real `SimPlan` -- the "every named service/API/table/stream/
//! capability resolves" preflight -- is M5's job, once a runner exists
//! to actually need it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SCENARIO_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scenario {
    pub simulation_version: u32,
    pub name: String,
    /// RFC 3339 virtual start time.
    pub start_at: String,
    #[serde(default)]
    pub given: Given,
    pub steps: Vec<ScenarioStep>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Given {
    #[serde(default)]
    pub db: Vec<GivenTableRows>,
    /// 27UpdatePlan.md M1 (Pillar 5): seed cache entries, optionally
    /// TTL-stamped against the virtual clock at scenario start.
    #[serde(default)]
    pub cache: Vec<GivenCacheEntry>,
    /// 27UpdatePlan.md M1 (Pillar 5): seed object-store entries.
    /// `value_base64` keeps the scenario file JSON-clean for binary
    /// payloads -- the one place this schema acknowledges a non-JSON
    /// value, matching the object store's byte-map semantics.
    #[serde(default)]
    pub store: Vec<GivenStoreObject>,
    /// 27UpdatePlan.md M1 (Pillar 5): seed search-index documents.
    #[serde(default)]
    pub search: Vec<GivenSearchDoc>,
    #[serde(default)]
    pub external_http: Vec<GivenHttpFixture>,
    /// v0.17 M10: failure rules a scenario declares up front (Pillar
    /// 7's own `{"at": {..}, "action": {..}}` shape, reused verbatim
    /// via `crate::failure::FailureRule`) so a checked-in scenario is
    /// fully self-describing -- a runner reads its failure injection
    /// from the scenario document itself, not from out-of-band
    /// per-fixture configuration a hand-written script used to supply.
    #[serde(default)]
    pub failures: Vec<crate::failure::FailureRule>,
}

/// 27UpdatePlan.md M1 (Pillar 5). `ttl` is a duration string like
/// `AdvanceStep::by` (e.g. `"30m"`); absent means no expiry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GivenCacheEntry {
    pub instance: String,
    pub key: String,
    pub value: serde_json::Value,
    #[serde(default)]
    pub ttl: Option<String>,
}

/// 27UpdatePlan.md M1 (Pillar 5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GivenStoreObject {
    pub instance: String,
    pub key: String,
    pub value_base64: String,
}

/// 27UpdatePlan.md M1 (Pillar 5).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GivenSearchDoc {
    pub instance: String,
    pub id: String,
    pub doc: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GivenTableRows {
    pub service: String,
    pub table: String,
    pub rows: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GivenHttpFixture {
    pub instance: String,
    pub responses: Vec<GivenHttpResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum GivenHttpResponse {
    Error {
        error: String,
    },
    Ok {
        status: u16,
        json: serde_json::Value,
    },
}

/// One scenario step: exactly one of the closed action set. `serde`'s
/// externally-tagged enum-of-structs matches the plan's own worked
/// example (`{"request": {...}}`, `{"drain": {}}`, ...) directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ScenarioStep {
    #[serde(rename = "request")]
    Request(RequestStep),
    #[serde(rename = "publish")]
    Publish(PublishStep),
    #[serde(rename = "advance")]
    Advance(AdvanceStep),
    #[serde(rename = "drain")]
    Drain(DrainStep),
    #[serde(rename = "expect")]
    Expect(ExpectStep),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestStep {
    pub service: String,
    pub api: String,
    #[serde(default)]
    pub json: serde_json::Value,
    /// Verified principal: `sub` and `scopes`. Absent means an
    /// unauthenticated request.
    #[serde(rename = "as", default)]
    pub principal: Option<Principal>,
    /// Names this step's response for later `expect` steps to
    /// reference.
    #[serde(default)]
    pub save_as: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub sub: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishStep {
    pub stream: String,
    #[serde(default)]
    pub json: serde_json::Value,
}

/// A virtual-time advance, e.g. `{"by": "7d"}`. Parsed leniently here
/// (a duration string); resolving it to a concrete tick count is the
/// scheduler's job (M4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvanceStep {
    pub by: String,
}

/// Drains every eligible scheduled/pending effect at the current
/// virtual instant before the next step runs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DrainStep {}

/// One assertion. Deliberately a flat, closed set of named checks
/// rather than a boolean expression language -- see Pillar 3's own
/// "no arbitrary scripts, loops, conditional scenario language."
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectStep {
    Response {
        #[serde(rename = "of")]
        of: Option<String>,
        status: Option<u16>,
        json: Option<serde_json::Value>,
    },
    Row {
        service: String,
        table: String,
        #[serde(rename = "where")]
        matches: BTreeMap<String, serde_json::Value>,
        present: bool,
    },
    WorkerAttempts {
        worker: String,
        count: u32,
    },
    JobRuns {
        job: String,
        count: u32,
    },
    Quiescence {},
    /// 27UpdatePlan.md M1 (Pillar 5): the email fake's observation
    /// surface. `to`/`subject_contains` are optional filters over the
    /// send log; `count` is the number of matching sends (exact, per
    /// the point-assertion discipline -- no ranges).
    Email {
        #[serde(default)]
        to: Option<String>,
        #[serde(default)]
        subject_contains: Option<String>,
        count: u32,
    },
    Cache {
        instance: String,
        key: String,
        present: bool,
        #[serde(default)]
        value: Option<serde_json::Value>,
    },
    /// Named `store`/`key` rather than `instance`/`key` to read
    /// unambiguously next to `expect.row`'s `service`/`table` shape --
    /// `store` is this expectation's own name for the object-store
    /// instance being asked about.
    Object {
        store: String,
        key: String,
        present: bool,
    },
    /// Corrected from the plan's own drafted `"index"` field name
    /// (27UpdatePlan.md Pillar 5) to `instance`, matching
    /// `given.search`'s field and the design rule that every new
    /// given/expect names its capability *instance* consistently.
    SearchHits {
        instance: String,
        query: String,
        count: u32,
    },
    /// Corrected from the plan's own drafted `"fixture"` field name
    /// (27UpdatePlan.md Pillar 5) to `instance` -- the current
    /// `given.external_http` schema keys a fixture by `instance`
    /// (there is no separate "fixture key" concept it also carries),
    /// so `instance` is what a runner can actually resolve this
    /// expectation against: the count of responses consumed so far
    /// for that named HTTP client instance.
    HttpCalls {
        instance: String,
        count: u32,
    },
}

/// Errors a scenario document can fail with before any simulated
/// effect runs -- structural/parse-level only for M2; plan-aware
/// preflight ("every named service/API/table/stream resolves") lands
/// with the M5 runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioError {
    UnsupportedVersion { found: u32, expected: u32 },
    EmptySteps,
}

impl std::fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ScenarioError::UnsupportedVersion { found, expected } => write!(
                f,
                "scenario simulation_version {found} is not supported (expected {expected})"
            ),
            ScenarioError::EmptySteps => write!(f, "scenario has no steps"),
        }
    }
}

impl std::error::Error for ScenarioError {}

impl Scenario {
    pub fn parse(json: &str) -> Result<Scenario, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Structural validation only (see module docs): version and
    /// non-emptiness. Name/reference resolution against a `SimPlan` is
    /// `SimPlan::validate_scenario`, not this -- deliberately not a
    /// method here (see that method's own doc comment for why: this
    /// file is vendored verbatim into every generated Rust project,
    /// which has no `ciac_ir`/`SimPlan` dependency to reference).
    pub fn validate(&self) -> Result<(), ScenarioError> {
        if self.simulation_version != SCENARIO_VERSION {
            return Err(ScenarioError::UnsupportedVersion {
                found: self.simulation_version,
                expected: SCENARIO_VERSION,
            });
        }
        if self.steps.is_empty() {
            return Err(ScenarioError::EmptySteps);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RETRY_AND_CLEANUP: &str = r#"
    {
      "simulation_version": 1,
      "name": "third-retry-and-nightly-cleanup",
      "start_at": "2030-01-01T00:00:00Z",
      "given": {
        "db": [
          { "service": "Orders", "table": "Orders", "rows": [] }
        ],
        "external_http": [
          {
            "instance": "payments",
            "responses": [
              {"error": "timeout"},
              {"error": "timeout"},
              {"status": 200, "json": {"accepted": true}}
            ]
          }
        ]
      },
      "steps": [
        {
          "request": {
            "service": "Gateway",
            "api": "CreateOrder",
            "json": {"total": 10},
            "as": {"sub": "user-1", "scopes": ["orders:write"]},
            "save_as": "create"
          }
        },
        {"drain": {}},
        {"advance": {"by": "7d"}},
        {"drain": {}},
        {"expect": {"worker_attempts": {"worker": "Charge", "count": 3}}},
        {"expect": {"job_runs": {"job": "Cleanup", "count": 7}}}
      ]
    }
    "#;

    #[test]
    fn parses_the_plan_s_own_worked_example_verbatim() {
        let scenario = Scenario::parse(RETRY_AND_CLEANUP).expect("parses");
        assert_eq!(scenario.name, "third-retry-and-nightly-cleanup");
        assert_eq!(scenario.steps.len(), 6);
        assert_eq!(scenario.given.db.len(), 1);
        assert_eq!(scenario.given.external_http[0].responses.len(), 3);
        scenario.validate().expect("structurally valid");

        match &scenario.steps[0] {
            ScenarioStep::Request(r) => {
                assert_eq!(r.service, "Gateway");
                assert_eq!(r.api, "CreateOrder");
                assert_eq!(r.principal.as_ref().unwrap().sub, "user-1");
                assert_eq!(r.save_as.as_deref(), Some("create"));
            }
            other => panic!("expected a request step, got {other:?}"),
        }
        match &scenario.steps[4] {
            ScenarioStep::Expect(ExpectStep::WorkerAttempts { worker, count }) => {
                assert_eq!(worker, "Charge");
                assert_eq!(*count, 3);
            }
            other => panic!("expected a worker_attempts expectation, got {other:?}"),
        }
    }

    #[test]
    fn round_trips_through_json() {
        let scenario = Scenario::parse(RETRY_AND_CLEANUP).unwrap();
        let rendered = serde_json::to_string(&scenario).unwrap();
        let reparsed = Scenario::parse(&rendered).unwrap();
        assert_eq!(scenario.steps.len(), reparsed.steps.len());
    }

    #[test]
    fn rejects_unsupported_version() {
        let scenario = Scenario {
            simulation_version: 99,
            name: "x".into(),
            start_at: "2030-01-01T00:00:00Z".into(),
            given: Given::default(),
            steps: vec![ScenarioStep::Drain(DrainStep {})],
        };
        assert_eq!(
            scenario.validate(),
            Err(ScenarioError::UnsupportedVersion {
                found: 99,
                expected: SCENARIO_VERSION
            })
        );
    }

    // `SimPlan::validate_scenario` (relocated from this file's own
    // `validate_against_plan` -- see that method's doc comment in
    // `plan.rs`) has its test coverage in `plan.rs`'s own test module,
    // since it now lives there.

    // The M5-checkpoint fixture-file test moved to
    // `tests/scenario_fixtures.rs` (v0.17 M11): it reads
    // `sim/*.ciac-sim.json` via `CARGO_MANIFEST_DIR`, a path that only
    // resolves inside this crate's own checkout. `scenario.rs` itself is
    // vendored verbatim (`include_str!`) into every generated Rust
    // project that needs `SimWorld` (see `ciac-backend-rust/src/lib.rs`'s
    // `VENDORED_SIM_*` constants), and a generated project has no
    // `sim/` directory at that relative path — keeping the test here
    // would fail in every vendored copy. An integration test isn't
    // `include_str!`-ed, only `src/scenario.rs` is, so this is the one
    // reliable way to keep the fixture check without breaking vendoring.

    #[test]
    fn given_failures_parses_the_pillar_7_worked_example_verbatim() {
        let json = r#"
        {
          "simulation_version": 1,
          "name": "with-failures",
          "start_at": "2030-01-01T00:00:00Z",
          "given": {
            "failures": [
              {
                "at": {
                  "effect": "broker.ack",
                  "subject": "orders.created",
                  "occurrence": 1,
                  "phase": "after"
                },
                "action": {"kind": "lose"}
              }
            ]
          },
          "steps": [{"drain": {}}]
        }
        "#;
        let scenario = Scenario::parse(json).expect("parses");
        assert_eq!(scenario.given.failures.len(), 1);
        assert_eq!(scenario.given.failures[0].at.effect, "broker.ack");
        assert_eq!(
            scenario.given.failures[0].action,
            crate::failure::FailureAction::Lose
        );
    }

    #[test]
    fn given_failures_defaults_to_empty() {
        let scenario = Scenario::parse(RETRY_AND_CLEANUP).unwrap();
        assert!(scenario.given.failures.is_empty());
    }

    #[test]
    fn given_peripherals_parse_pillar_5_worked_examples() {
        let json = r#"
        {
          "simulation_version": 1,
          "name": "peripherals",
          "start_at": "2030-01-01T00:00:00Z",
          "given": {
            "cache": [
              { "instance": "sessions", "key": "u1", "value": {"a": 1}, "ttl": "30m" }
            ],
            "store": [
              { "instance": "uploads", "key": "a.png", "value_base64": "AAA=" }
            ],
            "search": [
              { "instance": "catalog", "id": "p1", "doc": {"name": "widget"} }
            ]
          },
          "steps": [{"drain": {}}]
        }
        "#;
        let scenario = Scenario::parse(json).expect("parses");
        assert_eq!(scenario.given.cache[0].instance, "sessions");
        assert_eq!(scenario.given.cache[0].ttl.as_deref(), Some("30m"));
        assert_eq!(scenario.given.store[0].value_base64, "AAA=");
        assert_eq!(scenario.given.search[0].doc["name"], "widget");
        scenario.validate().expect("structurally valid");
    }

    #[test]
    fn given_peripherals_default_to_empty() {
        let scenario = Scenario::parse(RETRY_AND_CLEANUP).unwrap();
        assert!(scenario.given.cache.is_empty());
        assert!(scenario.given.store.is_empty());
        assert!(scenario.given.search.is_empty());
    }

    type ExpectCheck = fn(&ExpectStep);

    #[test]
    fn expect_peripherals_parse_pillar_5_worked_examples() {
        let cases: &[(&str, ExpectCheck)] = &[
            (
                r#"{"email": {"to": "ops@example.com", "subject_contains": "reconciled", "count": 1}}"#,
                |e| match e {
                    ExpectStep::Email {
                        to,
                        subject_contains,
                        count,
                    } => {
                        assert_eq!(to.as_deref(), Some("ops@example.com"));
                        assert_eq!(subject_contains.as_deref(), Some("reconciled"));
                        assert_eq!(*count, 1);
                    }
                    other => panic!("expected email, got {other:?}"),
                },
            ),
            (
                r#"{"cache": {"instance": "sessions", "key": "u1", "present": false}}"#,
                |e| match e {
                    ExpectStep::Cache {
                        instance,
                        key,
                        present,
                        value,
                    } => {
                        assert_eq!(instance, "sessions");
                        assert_eq!(key, "u1");
                        assert!(!present);
                        assert!(value.is_none());
                    }
                    other => panic!("expected cache, got {other:?}"),
                },
            ),
            (
                r#"{"object": {"store": "uploads", "key": "a.png", "present": true}}"#,
                |e| match e {
                    ExpectStep::Object {
                        store,
                        key,
                        present,
                    } => {
                        assert_eq!(store, "uploads");
                        assert_eq!(key, "a.png");
                        assert!(*present);
                    }
                    other => panic!("expected object, got {other:?}"),
                },
            ),
            (
                r#"{"search_hits": {"instance": "catalog", "query": "widget", "count": 2}}"#,
                |e| match e {
                    ExpectStep::SearchHits {
                        instance,
                        query,
                        count,
                    } => {
                        assert_eq!(instance, "catalog");
                        assert_eq!(query, "widget");
                        assert_eq!(*count, 2);
                    }
                    other => panic!("expected search_hits, got {other:?}"),
                },
            ),
            (
                r#"{"http_calls": {"instance": "payments", "count": 3}}"#,
                |e| match e {
                    ExpectStep::HttpCalls { instance, count } => {
                        assert_eq!(instance, "payments");
                        assert_eq!(*count, 3);
                    }
                    other => panic!("expected http_calls, got {other:?}"),
                },
            ),
        ];
        for (json, check) in cases {
            let step: ExpectStep = serde_json::from_str(json).expect("parses");
            check(&step);
            let rendered = serde_json::to_string(&step).unwrap();
            serde_json::from_str::<ExpectStep>(&rendered).expect("round-trips");
        }
    }

    #[test]
    fn rejects_empty_steps() {
        let scenario = Scenario {
            simulation_version: SCENARIO_VERSION,
            name: "x".into(),
            start_at: "2030-01-01T00:00:00Z".into(),
            given: Given::default(),
            steps: vec![],
        };
        assert_eq!(scenario.validate(), Err(ScenarioError::EmptySteps));
    }
}
