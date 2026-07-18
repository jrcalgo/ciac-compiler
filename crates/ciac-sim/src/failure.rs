//! Deterministic failure injection (17UpdatePlan.md Pillar 7). Failures
//! select semantic effects, not threads, line numbers, or target stack
//! frames -- `{"at": {"effect": "broker.ack", "subject":
//! "orders.created", "occurrence": 1, "phase": "after"}, "action":
//! {"kind": "lose"}}` is the plan's own worked example, matched
//! verbatim by this module's (de)serialization.
//!
//! `before` means the effect was not applied; `after` means it was
//! applied but the caller observed failure or ambiguity -- the
//! distinction the plan calls "essential for commit-then-error and
//! lost-ack tests." A required rule that never matches is a scenario
//! error (`SIM0007`), never a silently-skipped no-op.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePhase {
    Before,
    After,
}

/// Which real effect occurrence a rule targets. `occurrence` is
/// 1-based and counts matches of `(effect, subject)` -- `None` matches
/// every occurrence of that effect/subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureSelector {
    pub effect: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub occurrence: Option<u64>,
    pub phase: FailurePhase,
}

/// The closed action vocabulary. `Delay` alone carries a parameter
/// (`by_ms`) -- the plan does not invent a default backoff duration, so
/// a rule that delays must say how long.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FailureAction {
    Error,
    Delay { by_ms: i64 },
    Timeout,
    Lose,
    Duplicate,
    Disconnect,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureRule {
    pub at: FailureSelector,
    pub action: FailureAction,
}

/// Tracks a set of failure rules against a live run's real effect
/// occurrences, deciding which action (if any) applies to each one and
/// which required rules never fired.
#[derive(Debug)]
pub struct FailureEngine {
    rules: Vec<FailureRule>,
    matched: Vec<bool>,
    /// Running count of `(effect, subject)` occurrences seen so far,
    /// independent of which (if any) rule matches them -- an
    /// occurrence-1 rule and an occurrence-2 rule on the same
    /// `(effect, subject)` both need this shared counter to agree on
    /// which real occurrence is "first" versus "second."
    occurrence_counts: HashMap<(String, Option<String>), u64>,
}

impl FailureEngine {
    pub fn new(rules: Vec<FailureRule>) -> FailureEngine {
        let matched = vec![false; rules.len()];
        FailureEngine {
            rules,
            matched,
            occurrence_counts: HashMap::new(),
        }
    }

    /// Records one real occurrence of `effect`/`subject` at `phase`,
    /// returning the action of the first still-applicable rule that
    /// matches it (rules are checked in declaration order; the first
    /// match wins, deterministically).
    pub fn record_occurrence(
        &mut self,
        effect: &str,
        subject: Option<&str>,
        phase: FailurePhase,
    ) -> Option<FailureAction> {
        let key = (effect.to_owned(), subject.map(str::to_owned));
        let count = self.occurrence_counts.entry(key).or_insert(0);
        *count += 1;
        let this_occurrence = *count;

        for (idx, rule) in self.rules.iter().enumerate() {
            if self.matched[idx] {
                continue;
            }
            let selector = &rule.at;
            if selector.effect != effect {
                continue;
            }
            if let Some(want_subject) = &selector.subject {
                if subject != Some(want_subject.as_str()) {
                    continue;
                }
            }
            if selector.phase != phase {
                continue;
            }
            if let Some(want_occurrence) = selector.occurrence {
                if want_occurrence != this_occurrence {
                    continue;
                }
            }
            self.matched[idx] = true;
            return Some(rule.action.clone());
        }
        None
    }

    /// Rules that never matched any real occurrence -- `SIM0007`
    /// territory. Every rule is "required" in this engine: there is no
    /// separate optional-rule concept in 17UpdatePlan.md's Pillar 7.
    pub fn unmatched_rules(&self) -> Vec<&FailureRule> {
        self.rules
            .iter()
            .zip(&self.matched)
            .filter(|(_, matched)| !**matched)
            .map(|(rule, _)| rule)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_plan_s_own_worked_example_verbatim() {
        let json = r#"{
            "at": {
                "effect": "broker.ack",
                "subject": "orders.created",
                "occurrence": 1,
                "phase": "after"
            },
            "action": {"kind": "lose"}
        }"#;
        let rule: FailureRule = serde_json::from_str(json).expect("parses");
        assert_eq!(rule.at.effect, "broker.ack");
        assert_eq!(rule.at.subject.as_deref(), Some("orders.created"));
        assert_eq!(rule.at.occurrence, Some(1));
        assert_eq!(rule.at.phase, FailurePhase::After);
        assert_eq!(rule.action, FailureAction::Lose);
    }

    #[test]
    fn matches_the_exact_occurrence_and_ignores_others() {
        let rule = FailureRule {
            at: FailureSelector {
                effect: "broker.ack".into(),
                subject: Some("orders.created".into()),
                occurrence: Some(2),
                phase: FailurePhase::After,
            },
            action: FailureAction::Lose,
        };
        let mut engine = FailureEngine::new(vec![rule]);
        assert_eq!(
            engine.record_occurrence("broker.ack", Some("orders.created"), FailurePhase::After),
            None,
            "first occurrence doesn't match an occurrence:2 rule"
        );
        assert_eq!(
            engine.record_occurrence("broker.ack", Some("orders.created"), FailurePhase::After),
            Some(FailureAction::Lose),
            "second occurrence matches"
        );
        assert_eq!(
            engine.record_occurrence("broker.ack", Some("orders.created"), FailurePhase::After),
            None,
            "already matched -- does not fire again"
        );
        assert!(engine.unmatched_rules().is_empty());
    }

    #[test]
    fn before_and_after_phases_are_distinct_selectors() {
        let rule = FailureRule {
            at: FailureSelector {
                effect: "db.commit".into(),
                subject: None,
                occurrence: None,
                phase: FailurePhase::Before,
            },
            action: FailureAction::Error,
        };
        let mut engine = FailureEngine::new(vec![rule]);
        assert_eq!(
            engine.record_occurrence("db.commit", None, FailurePhase::After),
            None,
            "wrong phase never matches"
        );
        assert_eq!(
            engine.record_occurrence("db.commit", None, FailurePhase::Before),
            Some(FailureAction::Error)
        );
    }

    #[test]
    fn a_required_rule_that_never_matches_is_reported() {
        let rule = FailureRule {
            at: FailureSelector {
                effect: "broker.ack".into(),
                subject: Some("typo-subject".into()),
                occurrence: Some(1),
                phase: FailurePhase::After,
            },
            action: FailureAction::Lose,
        };
        let mut engine = FailureEngine::new(vec![rule]);
        engine.record_occurrence("broker.ack", Some("orders.created"), FailurePhase::After);
        let unmatched = engine.unmatched_rules();
        assert_eq!(unmatched.len(), 1);
        assert_eq!(unmatched[0].at.subject.as_deref(), Some("typo-subject"));
    }

    #[test]
    fn subject_none_selector_matches_any_subject() {
        let rule = FailureRule {
            at: FailureSelector {
                effect: "external_http.request".into(),
                subject: None,
                occurrence: None,
                phase: FailurePhase::After,
            },
            action: FailureAction::Timeout,
        };
        let mut engine = FailureEngine::new(vec![rule]);
        assert_eq!(
            engine.record_occurrence(
                "external_http.request",
                Some("payments"),
                FailurePhase::After
            ),
            Some(FailureAction::Timeout)
        );
    }

    #[test]
    fn delay_action_carries_its_duration() {
        let json = r#"{"at": {"effect": "worker.retry", "phase": "before"}, "action": {"kind": "delay", "by_ms": 5000}}"#;
        let rule: FailureRule = serde_json::from_str(json).unwrap();
        assert_eq!(rule.action, FailureAction::Delay { by_ms: 5000 });
    }
}
