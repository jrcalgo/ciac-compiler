//! v0.17 M11: a Rust-native simulation world. Unlike Python -- which
//! must restate `ciac-sim`'s primitives narrowly, since it cannot call
//! Rust code directly (`sim/pyrunner/world.py`'s own `FailureEngine`
//! class says so in its docstring) -- generated Rust code depends on
//! `ciac-sim` itself (vendored source, `ciac sim`'s own job to write
//! out; see `crates/ciac/src/commands.rs`) and drives the real
//! [`crate::failure::FailureEngine`] this crate already owns, not a
//! second copy of it.
//!
//! Schema-agnostic like Python's `FakeDatabase`: rows are
//! [`serde_json::Value`], not compile-time-typed structs, because this
//! crate has no knowledge of any particular `.ciac` program's schema.
//! The bridge from a generated project's own typed records to these
//! JSON rows is one `serde_json::to_value`/`from_value` round-trip at
//! each world-guarded call site in the generated code itself --
//! mirroring exactly how Python's fakes bridge typed SQLAlchemy models
//! to plain dicts.
//!
//! Deliberately narrow, matching this milestone's own disclosed scope
//! (17UpdatePlan.md's M11 entry): only what `db.insert` and broker
//! `publish` need to drive the checkpoint's own vertical slice. Get/
//! update/delete/count/query verbs, cascades, constraints, and the
//! full broker semantics (independent queue-group cursors, ordering,
//! lost-ack) that Python's fakes already cover (Pillars 4/5, v0.17
//! M6/M7) are real, disclosed future work for this Rust world, not
//! silently claimed here.

use crate::failure::{FailureAction, FailureEngine, FailurePhase, FailureRule};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// An in-memory table store keyed by table name, rows as plain JSON
/// values. No constraints, no cascades, no indexes -- see the module
/// docs for what Python's fuller fake already covers that this one
/// does not yet.
#[derive(Debug, Default)]
pub struct FakeDatabase {
    tables: Mutex<BTreeMap<String, Vec<Value>>>,
}

impl FakeDatabase {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, table: &str, row: Value) {
        self.tables
            .lock()
            .expect("FakeDatabase mutex poisoned")
            .entry(table.to_owned())
            .or_default()
            .push(row);
    }

    /// Rows in `table` matching every key/value pair in `filter` (an
    /// empty filter matches every row in the table) -- the scenario
    /// runner's own `expect.row`/`expect.quiescence`-adjacent check,
    /// not something generated code itself ever calls.
    pub fn find_where(&self, table: &str, filter: &BTreeMap<String, Value>) -> Vec<Value> {
        self.tables
            .lock()
            .expect("FakeDatabase mutex poisoned")
            .get(table)
            .into_iter()
            .flatten()
            .filter(|row| filter.iter().all(|(k, v)| row.get(k) == Some(v)))
            .cloned()
            .collect()
    }

    pub fn count(&self, table: &str) -> usize {
        self.tables
            .lock()
            .expect("FakeDatabase mutex poisoned")
            .get(table)
            .map_or(0, Vec::len)
    }
}

/// Every publish, in arrival order -- the runner drains these directly
/// into a worker's own `handle_message_once`, the same "the runner
/// pumps delivery, there is no running subscription loop" architecture
/// Python's `FakeBroker` uses (v0.17 M7). No independent per-`(subject,
/// group)` cursors yet (every registered worker/job in this milestone's
/// vertical slice has exactly one consumer), so this is deliberately
/// narrower than Python's fan-out-capable broker fake.
#[derive(Debug, Default)]
pub struct FakeQueue {
    published: Mutex<Vec<(String, Vec<u8>)>>,
}

impl FakeQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&self, subject: &str, payload: Vec<u8>) {
        self.published
            .lock()
            .expect("FakeQueue mutex poisoned")
            .push((subject.to_owned(), payload));
    }

    /// Every message published since the last drain, removing them --
    /// the runner's own `drain` step calls this once per subject it
    /// knows how to deliver.
    pub fn take_all(&self) -> Vec<(String, Vec<u8>)> {
        std::mem::take(&mut self.published.lock().expect("FakeQueue mutex poisoned"))
    }
}

/// The simulation adapter `AppState::simulation(world)` constructs
/// from in generated Rust code -- one instance per running scenario,
/// shared (via `Arc`) across every cloned `AppState` handle exactly
/// like production's connection pools.
#[derive(Debug)]
pub struct SimWorld {
    pub db: FakeDatabase,
    pub queue: FakeQueue,
    failures: Mutex<FailureEngine>,
}

impl SimWorld {
    pub fn new(failure_rules: Vec<FailureRule>) -> Self {
        Self {
            db: FakeDatabase::new(),
            queue: FakeQueue::new(),
            failures: Mutex::new(FailureEngine::new(failure_rules)),
        }
    }

    /// `db.commit`'s failure check plus the actual (fake) write,
    /// mirroring Python's `_Session.commit()`: a matched rule raises
    /// before the row is stored, same as a real transaction rolling
    /// back. Only the `error` action is implemented -- the same
    /// disclosed subset of the full `FailureAction` vocabulary
    /// Python's own restatement supports, for the same reason (this is
    /// the one action the checkpoint's own worked scenario needs).
    pub fn db_insert_checked(&self, table: &str, row: Value) -> anyhow::Result<()> {
        if self.should_fail("db.commit", Some(table))? {
            anyhow::bail!("simulated db.commit failure (injected by FailureEngine)");
        }
        self.db.insert(table, row);
        Ok(())
    }

    /// `broker.publish`'s failure check plus the actual (fake) publish
    /// -- checked for the same reason `db_insert_checked` checks
    /// `db.commit`, even though no scenario in this milestone's own
    /// worked example targets a publish effect.
    pub fn publish_checked(&self, subject: &str, payload: Vec<u8>) -> anyhow::Result<()> {
        if self.should_fail("broker.publish", Some(subject))? {
            anyhow::bail!("simulated broker.publish failure (injected by FailureEngine)");
        }
        self.queue.publish(subject, payload);
        Ok(())
    }

    fn should_fail(&self, effect: &str, subject: Option<&str>) -> anyhow::Result<bool> {
        let action = self
            .failures
            .lock()
            .expect("FailureEngine mutex poisoned")
            .record_occurrence(effect, subject, FailurePhase::After);
        match action {
            None => Ok(false),
            Some(FailureAction::Error) => Ok(true),
            Some(other) => anyhow::bail!(
                "failure rule for effect {effect:?} uses action {other:?}, which this Rust \
                 world's FailureEngine integration does not support yet (only `error` is \
                 implemented)"
            ),
        }
    }

    /// Failure rules the scenario declared that never matched a real
    /// occurrence -- `SIM0007` territory, surfaced to the runner rather
    /// than silently ignored.
    pub fn unmatched_failure_rules(&self) -> Vec<FailureRule> {
        self.failures
            .lock()
            .expect("FailureEngine mutex poisoned")
            .unmatched_rules()
            .into_iter()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::failure::FailureSelector;

    #[test]
    fn insert_then_find_where_round_trips() {
        let db = FakeDatabase::new();
        db.insert("orders", serde_json::json!({"id": "1", "total": 9.5}));
        db.insert("orders", serde_json::json!({"id": "2", "total": 3.0}));
        assert_eq!(db.count("orders"), 2);
        let mut filter = BTreeMap::new();
        filter.insert("id".to_owned(), serde_json::json!("2"));
        let found = db.find_where("orders", &filter);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["total"], 3.0);
        assert!(db.find_where("orders", &BTreeMap::new()).len() == 2);
    }

    #[test]
    fn queue_take_all_drains_in_publish_order() {
        let queue = FakeQueue::new();
        queue.publish("a", b"1".to_vec());
        queue.publish("a", b"2".to_vec());
        let drained = queue.take_all();
        assert_eq!(
            drained,
            vec![
                ("a".to_owned(), b"1".to_vec()),
                ("a".to_owned(), b"2".to_vec())
            ]
        );
        assert!(
            queue.take_all().is_empty(),
            "a second drain sees nothing new"
        );
    }

    #[test]
    fn db_insert_checked_fails_on_the_matched_occurrence_and_does_not_store_the_row() {
        let rule = FailureRule {
            at: FailureSelector {
                effect: "db.commit".into(),
                subject: Some("processed_orders".into()),
                occurrence: Some(1),
                phase: FailurePhase::After,
            },
            action: FailureAction::Error,
        };
        let world = SimWorld::new(vec![rule]);
        let row = serde_json::json!({"id": "1"});
        assert!(world
            .db_insert_checked("processed_orders", row.clone())
            .is_err());
        assert_eq!(
            world.db.count("processed_orders"),
            0,
            "failed commit stores nothing"
        );
        assert!(world.db_insert_checked("processed_orders", row).is_ok());
        assert_eq!(
            world.db.count("processed_orders"),
            1,
            "the second (unmatched) attempt commits"
        );
        assert!(world.unmatched_failure_rules().is_empty());
    }

    #[test]
    fn unsupported_failure_action_is_refused_not_silently_ignored() {
        let rule = FailureRule {
            at: FailureSelector {
                effect: "db.commit".into(),
                subject: None,
                occurrence: None,
                phase: FailurePhase::After,
            },
            action: FailureAction::Lose,
        };
        let world = SimWorld::new(vec![rule]);
        let err = world
            .db_insert_checked("orders", serde_json::json!({}))
            .unwrap_err();
        assert!(err.to_string().contains("does not support"));
    }
}
