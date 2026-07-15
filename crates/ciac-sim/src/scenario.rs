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
    /// M5's preflight, not this.
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

    #[test]
    fn m5_checkpoint_scenarios_are_valid_instances_of_this_schema() {
        // The two scenario files 17UpdatePlan.md's M5 milestone checks
        // in (`sim/vertical-slice.ciac-sim.json`,
        // `sim/virtual-week.ciac-sim.json`) are real JSON documents, not
        // just prose examples -- this test is the schema-side half of
        // the M5 checkpoint's proof: they parse and structurally
        // validate against the schema this module owns. The Python-side
        // half (a real generated project executing the equivalent
        // effect sequence) lives in `sim/pyrunner/`, outside this crate.
        for name in ["vertical-slice", "virtual-week"] {
            let path = format!(
                "{}/../../sim/{name}.ciac-sim.json",
                env!("CARGO_MANIFEST_DIR")
            );
            let json =
                std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
            let scenario = Scenario::parse(&json)
                .unwrap_or_else(|e| panic!("{name}.ciac-sim.json failed to parse: {e}"));
            scenario
                .validate()
                .unwrap_or_else(|e| panic!("{name}.ciac-sim.json failed to validate: {e}"));
            assert!(!scenario.steps.is_empty());
        }
    }

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
