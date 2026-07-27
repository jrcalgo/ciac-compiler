//! `ciac-sim` (v0.17): deterministic whole-system simulation contracts.
//! Owns the plan/scenario/replay/transcript schemas, the deterministic
//! scheduler/clock/failure engine, and the `SIM` runtime-outcome codes;
//! depends on normalized IR, not on Python or Rust -- a target adapter
//! consumes what this crate defines, never the reverse. See
//! `17UpdatePlan.md` for the full pillar-by-pillar design, and its "M1
//! findings" section for what's been reconciled against actual v0.16
//! IR/generated-code behavior.
//!
//! M2 shipped schemas, canonical hashing, structural scenario/replay
//! validation, and the `SIM` code registry. M4 (this milestone) adds
//! the target-neutral deterministic primitives -- virtual clock,
//! seeded entropy, the cron evaluator, the scheduling-key total order,
//! and failure injection -- as pure, independently testable logic, not
//! yet wired to a real running program. Actually driving generated
//! Python/Rust code through these primitives is M8/M9's job, gated by
//! the M5 checkpoint per the Rollout strategy restructuring.

mod clock;
mod codes;
mod cron;
mod failure;
mod plan;
mod replay;
mod scenario;
mod schedule;
pub mod world;

pub use clock::{Entropy, VirtualClock};
pub use codes::SimCode;
pub use cron::{CronError, CronSchedule};
pub use failure::{FailureAction, FailureEngine, FailurePhase, FailureRule, FailureSelector};
pub use plan::{
    ScenarioPlanError, SimApi, SimCallEdge, SimCardinality, SimColumn, SimFieldType, SimJob,
    SimPlan, SimRefAction, SimService, SimStream, SimTable, SimWorker, PLAN_VERSION,
};
pub use replay::{Replay, ReplayError, REPLAY_VERSION};
pub use scenario::{
    AdvanceStep, DrainStep, ExpectStep, Given, GivenHttpFixture, GivenHttpResponse, GivenTableRows,
    Principal, PublishStep, RequestStep, Scenario, ScenarioError, ScenarioStep, SCENARIO_VERSION,
};
pub use schedule::{retry_eligible, Phase, ScheduleRequest, Scheduler, SchedulingKey};
pub use world::{FakeDatabase, FakeQueue, SimWorld};
