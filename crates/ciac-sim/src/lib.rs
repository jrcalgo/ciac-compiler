//! `ciac-sim` (v0.17): deterministic whole-system simulation contracts.
//! Owns the plan/scenario/replay/transcript schemas and the `SIM`
//! runtime-outcome codes; depends on normalized IR, not on Python or
//! Rust -- a target adapter consumes what this crate defines, never the
//! reverse. See `17UpdatePlan.md` for the full pillar-by-pillar design,
//! and its "M1 findings" section for what's been reconciled against
//! actual v0.16 IR/generated-code behavior.
//!
//! M2 scope (this milestone): schemas, canonical hashing, structural
//! scenario/replay validation, and the `SIM` code registry. The
//! scheduler, fakes, and runner that give these schemas something to
//! describe land in M4 onward, gated by the M5 checkpoint per the
//! Rollout strategy restructuring.

mod codes;
mod plan;
mod replay;
mod scenario;

pub use codes::SimCode;
pub use plan::{
    SimCardinality, SimColumn, SimFieldType, SimJob, SimPlan, SimRefAction, SimService, SimStream,
    SimTable, SimWorker, PLAN_VERSION,
};
pub use replay::{Replay, ReplayError, REPLAY_VERSION};
pub use scenario::{
    AdvanceStep, DrainStep, ExpectStep, Given, GivenHttpFixture, GivenHttpResponse, GivenTableRows,
    Principal, PublishStep, RequestStep, Scenario, ScenarioError, ScenarioStep, SCENARIO_VERSION,
};
