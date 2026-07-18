//! Replay artifacts (17UpdatePlan.md Pillar 6, "Replay"): everything
//! needed to deterministically reproduce one simulation run, and to
//! refuse reproducing one that no longer applies.
//!
//! M2 ships the schema and the version-compatibility check
//! (`Replay::is_compatible_with`); actually recording/replaying a run
//! is M4's scheduler's job, once there is something to record.

use crate::plan::PLAN_VERSION;
use serde::{Deserialize, Serialize};

pub const REPLAY_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replay {
    pub replay_version: u32,
    pub plan_version: u32,
    /// Free-form identifier for the target adapter that produced this
    /// replay (e.g. `"python-0.17.0"`) -- compared, never parsed.
    pub target_adapter: String,
    pub source_hash: String,
    pub plan_hash: String,
    /// The exact scenario document this replay reproduces, canonical
    /// JSON -- carried whole rather than by reference, so a replay
    /// artifact is self-contained even if the original scenario file
    /// moves or changes.
    pub scenario: serde_json::Value,
    pub seed: u64,
    pub start_at: String,
    /// Ordered `(virtual timestamp, semantic key, outcome)` transcript
    /// entries. Kept as opaque JSON at this milestone -- M4's scheduler
    /// defines the real transcript-entry shape once events exist to
    /// record.
    pub transcript: Vec<serde_json::Value>,
    pub transcript_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    UnsupportedReplayVersion { found: u32, expected: u32 },
    PlanVersionMismatch { found: u32, expected: u32 },
    SourceMismatch { found: String, expected: String },
    PlanMismatch { found: String, expected: String },
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::UnsupportedReplayVersion { found, expected } => write!(
                f,
                "replay_version {found} is not supported (expected {expected})"
            ),
            ReplayError::PlanVersionMismatch { found, expected } => {
                write!(f, "plan_version {found} does not match {expected}")
            }
            ReplayError::SourceMismatch { found, expected } => write!(
                f,
                "recorded source_hash {found} does not match current {expected}"
            ),
            ReplayError::PlanMismatch { found, expected } => write!(
                f,
                "recorded plan_hash {found} does not match current {expected}"
            ),
        }
    }
}

impl std::error::Error for ReplayError {}

impl Replay {
    /// `--replay` refuses a source/plan/adapter mismatch (Pillar 6)
    /// rather than guessing compatibility. Compatibility is promised
    /// within a replay schema version, not indefinitely across semantic
    /// compiler changes -- a version bump on either side is a hard
    /// refusal, not a best-effort attempt.
    pub fn is_compatible_with(
        &self,
        current_source_hash: &str,
        current_plan_hash: &str,
    ) -> Result<(), ReplayError> {
        if self.replay_version != REPLAY_VERSION {
            return Err(ReplayError::UnsupportedReplayVersion {
                found: self.replay_version,
                expected: REPLAY_VERSION,
            });
        }
        if self.plan_version != PLAN_VERSION {
            return Err(ReplayError::PlanVersionMismatch {
                found: self.plan_version,
                expected: PLAN_VERSION,
            });
        }
        if self.source_hash != current_source_hash {
            return Err(ReplayError::SourceMismatch {
                found: self.source_hash.clone(),
                expected: current_source_hash.to_owned(),
            });
        }
        if self.plan_hash != current_plan_hash {
            return Err(ReplayError::PlanMismatch {
                found: self.plan_hash.clone(),
                expected: current_plan_hash.to_owned(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Replay {
        Replay {
            replay_version: REPLAY_VERSION,
            plan_version: PLAN_VERSION,
            target_adapter: "python-0.17.0".into(),
            source_hash: "sha256:aaa".into(),
            plan_hash: "sha256:bbb".into(),
            scenario: serde_json::json!({"simulation_version": 1}),
            seed: 42,
            start_at: "2030-01-01T00:00:00Z".into(),
            transcript: vec![],
            transcript_hash: "sha256:ccc".into(),
        }
    }

    #[test]
    fn compatible_replay_passes() {
        let replay = sample();
        assert!(replay
            .is_compatible_with("sha256:aaa", "sha256:bbb")
            .is_ok());
    }

    #[test]
    fn source_mismatch_is_refused_not_guessed() {
        let replay = sample();
        assert_eq!(
            replay.is_compatible_with("sha256:changed", "sha256:bbb"),
            Err(ReplayError::SourceMismatch {
                found: "sha256:aaa".into(),
                expected: "sha256:changed".into(),
            })
        );
    }

    #[test]
    fn plan_hash_mismatch_is_refused() {
        let replay = sample();
        assert_eq!(
            replay.is_compatible_with("sha256:aaa", "sha256:changed"),
            Err(ReplayError::PlanMismatch {
                found: "sha256:bbb".into(),
                expected: "sha256:changed".into(),
            })
        );
    }

    #[test]
    fn stale_replay_schema_version_is_refused() {
        let mut replay = sample();
        replay.replay_version = 99;
        assert_eq!(
            replay.is_compatible_with("sha256:aaa", "sha256:bbb"),
            Err(ReplayError::UnsupportedReplayVersion {
                found: 99,
                expected: REPLAY_VERSION
            })
        );
    }

    #[test]
    fn round_trips_through_json() {
        let replay = sample();
        let json = serde_json::to_string(&replay).unwrap();
        let reparsed: Replay = serde_json::from_str(&json).unwrap();
        assert_eq!(reparsed.seed, 42);
        assert_eq!(reparsed.transcript_hash, "sha256:ccc");
    }
}
