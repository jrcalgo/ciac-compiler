//! The deterministic scheduler (17UpdatePlan.md Pillar 6, "Scheduler
//! order"): every effect yields a semantic key, eligible events sort by
//! a documented total order, and the host scheduler (asyncio/Tokio wake
//! order) never gets to decide anything observable -- the `SimPlan`
//! ordering, not the host runtime, defines behavior.
//!
//! Target-neutral: this module knows nothing about workers, jobs, or
//! Python/Rust -- it manages an ordered queue of caller-supplied
//! payloads keyed by [`SchedulingKey`], and reports quiescence. A
//! target adapter (M8/M9) is what gives the payloads meaning.

use serde::Serialize;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// What kind of scheduled action this is, at the top scheduler level --
/// distinguishes different effect categories competing for the same
/// virtual timestamp. Not specified verbatim by 17UpdatePlan.md's prose;
/// this ordering (a message/job becoming available strictly before any
/// delivery of it is processed) is this milestone's own resolution of
/// that gap, chosen because it matches the natural dependency order
/// (existence before delivery) and is documented here rather than left
/// implicit in field order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum Phase {
    /// A message becoming eligible for delivery: a `publish`, direct or
    /// scenario-issued.
    Publish,
    /// A job's cron tick becoming due.
    Tick,
    /// A worker actually processing one delivery attempt.
    Deliver,
}

/// The semantic key 17UpdatePlan.md's Pillar 6 specifies verbatim:
/// "virtual timestamp, phase, service declaration identity, actor
/// identity, stream/message sequence, delivery attempt, local
/// occurrence." Field declaration order is the comparison order (Rust's
/// derived `Ord` compares fields top-to-bottom) -- this is the
/// documented total order the plan requires, not an incidental
/// consequence of struct layout.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct SchedulingKey {
    pub virtual_timestamp_ms: i64,
    pub phase: Phase,
    /// `service/<Name>`, matching `SimPlan`'s own key scheme.
    pub service: String,
    /// `worker/<Name>` | `job/<Name>` | ..., matching `SimPlan`'s own
    /// key scheme.
    pub actor: String,
    /// `None` sorts before `Some` (Rust's derived `Ord`) -- a
    /// non-stream-backed actor (a job tick) simply has no sequence to
    /// compare, not a sequence of zero.
    pub stream_sequence: Option<u64>,
    pub delivery_attempt: u32,
    /// Final tie-breaker: a monotonically increasing counter assigned
    /// at scheduling time, guaranteeing a strict total order even when
    /// every other field ties.
    pub local_occurrence: u64,
}

#[derive(Debug)]
struct Scheduled<E> {
    key: SchedulingKey,
    payload: E,
}

impl<E> PartialEq for Scheduled<E> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl<E> Eq for Scheduled<E> {}
impl<E> PartialOrd for Scheduled<E> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<E> Ord for Scheduled<E> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

/// Fields needed to schedule one event, minus `local_occurrence` (the
/// scheduler assigns that itself, so two calls can never collide on it).
#[derive(Debug)]
pub struct ScheduleRequest {
    pub virtual_timestamp_ms: i64,
    pub phase: Phase,
    pub service: String,
    pub actor: String,
    pub stream_sequence: Option<u64>,
    pub delivery_attempt: u32,
}

/// An ordered queue of caller-supplied payloads, driven by a
/// [`crate::clock::VirtualClock`]. Popping only ever returns the
/// earliest-keyed event whose `virtual_timestamp_ms` has already been
/// reached -- the scheduler never reveals a future event early, and
/// never reorders two events beyond what their keys already specify.
#[derive(Debug)]
pub struct Scheduler<E> {
    now_ms: i64,
    queue: BinaryHeap<Reverse<Scheduled<E>>>,
    next_local_occurrence: u64,
}

impl<E> Scheduler<E> {
    pub fn new(start_at_ms: i64) -> Scheduler<E> {
        Scheduler {
            now_ms: start_at_ms,
            queue: BinaryHeap::new(),
            next_local_occurrence: 0,
        }
    }

    pub fn now_ms(&self) -> i64 {
        self.now_ms
    }

    /// Schedules `payload`, returning the key it was assigned (a target
    /// adapter includes this in the transcript entry it eventually
    /// writes for the effect this payload represents).
    pub fn schedule(&mut self, request: ScheduleRequest, payload: E) -> SchedulingKey {
        let key = SchedulingKey {
            virtual_timestamp_ms: request.virtual_timestamp_ms,
            phase: request.phase,
            service: request.service,
            actor: request.actor,
            stream_sequence: request.stream_sequence,
            delivery_attempt: request.delivery_attempt,
            local_occurrence: self.next_local_occurrence,
        };
        self.next_local_occurrence += 1;
        self.queue.push(Reverse(Scheduled {
            key: key.clone(),
            payload,
        }));
        key
    }

    /// The earliest-keyed event, if its time has already been reached
    /// (`virtual_timestamp_ms <= now_ms`) -- pops it from the queue.
    /// Returns `None` without popping anything if the earliest event is
    /// still in the future; advance the clock first (see
    /// [`Self::advance_to`]).
    pub fn pop_eligible(&mut self) -> Option<(SchedulingKey, E)> {
        let is_eligible = matches!(self.queue.peek(), Some(Reverse(s)) if s.key.virtual_timestamp_ms <= self.now_ms);
        if !is_eligible {
            return None;
        }
        self.queue.pop().map(|Reverse(s)| (s.key, s.payload))
    }

    /// The next scheduled event's virtual timestamp, regardless of
    /// whether it's eligible yet -- what a caller advances the clock to
    /// when nothing is eligible at the current instant but work
    /// remains.
    pub fn peek_next_time(&self) -> Option<i64> {
        self.queue
            .peek()
            .map(|Reverse(s)| s.key.virtual_timestamp_ms)
    }

    /// True when no event is eligible at the current virtual time --
    /// quiescence at *this instant*, not necessarily overall (more work
    /// may still be scheduled for a later time; see
    /// [`Self::peek_next_time`]).
    pub fn is_idle_at_current_time(&self) -> bool {
        !matches!(self.queue.peek(), Some(Reverse(s)) if s.key.virtual_timestamp_ms <= self.now_ms)
    }

    /// True when the queue is completely empty -- full quiescence, the
    /// condition a scenario run terminates on.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Advances virtual time forward. Mirrors
    /// [`crate::clock::VirtualClock::advance_to`]'s monotonic
    /// requirement; a scheduler wraps its own clock rather than sharing
    /// a `VirtualClock` instance so `now_ms` and the queue stay
    /// consistent through one type.
    pub fn advance_to(&mut self, to_ms: i64) {
        assert!(
            to_ms >= self.now_ms,
            "scheduler time cannot move backward: at {}, asked for {to_ms}",
            self.now_ms
        );
        self.now_ms = to_ms;
    }
}

/// Retry eligibility (17UpdatePlan.md Pillar 6, "Retries"): "the live
/// language has `max_retries` but no declared backoff... first delivery
/// is attempt zero." `attempt` is 0-based; `max_retries` comes straight
/// from `SimWorker::max_retries`. `handle_message`'s own real generated
/// loop (`for attempt in range(MAX_RETRIES + 1)`, confirmed in M1's
/// findings) already implements exactly this bound -- this function
/// exists so the simulator's scheduler, which drives `handle_message_once`
/// directly rather than delegating to that generated loop (per Pillar
/// 2's "Actors, not infinite loops"), makes the identical decision.
pub fn retry_eligible(attempt: u32, max_retries: u32) -> bool {
    attempt <= max_retries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_eligible_matches_the_generated_loop_s_own_bound() {
        // MAX_RETRIES = 3 means attempts 0,1,2,3 are eligible (four
        // total, i.e. `range(MAX_RETRIES + 1)`), attempt 4 is exhausted.
        assert!(retry_eligible(0, 3));
        assert!(retry_eligible(3, 3));
        assert!(!retry_eligible(4, 3));
    }

    #[test]
    fn zero_max_retries_allows_only_the_first_attempt() {
        assert!(retry_eligible(0, 0));
        assert!(!retry_eligible(1, 0));
    }

    fn req(ts: i64, phase: Phase, actor: &str, attempt: u32) -> ScheduleRequest {
        ScheduleRequest {
            virtual_timestamp_ms: ts,
            phase,
            service: "service/Ops".into(),
            actor: actor.into(),
            stream_sequence: None,
            delivery_attempt: attempt,
        }
    }

    #[test]
    fn pops_events_in_virtual_timestamp_order() {
        let mut s: Scheduler<&str> = Scheduler::new(0);
        s.schedule(req(200, Phase::Deliver, "worker/A", 0), "second");
        s.schedule(req(100, Phase::Deliver, "worker/A", 0), "first");
        s.advance_to(200);
        assert_eq!(s.pop_eligible().map(|(_, p)| p), Some("first"));
        assert_eq!(s.pop_eligible().map(|(_, p)| p), Some("second"));
        assert!(s.pop_eligible().is_none());
    }

    #[test]
    fn future_events_are_not_eligible_before_the_clock_reaches_them() {
        let mut s: Scheduler<&str> = Scheduler::new(0);
        s.schedule(req(1_000, Phase::Tick, "job/Cleanup", 0), "later");
        assert!(s.pop_eligible().is_none());
        assert!(s.is_idle_at_current_time());
        assert_eq!(s.peek_next_time(), Some(1_000));
        s.advance_to(1_000);
        assert_eq!(s.pop_eligible().map(|(_, p)| p), Some("later"));
    }

    #[test]
    fn phase_orders_publish_before_tick_before_deliver_at_the_same_instant() {
        let mut s: Scheduler<&str> = Scheduler::new(0);
        s.schedule(req(0, Phase::Deliver, "worker/A", 0), "deliver");
        s.schedule(req(0, Phase::Publish, "worker/A", 0), "publish");
        s.schedule(req(0, Phase::Tick, "worker/A", 0), "tick");
        s.advance_to(0);
        assert_eq!(s.pop_eligible().map(|(_, p)| p), Some("publish"));
        assert_eq!(s.pop_eligible().map(|(_, p)| p), Some("tick"));
        assert_eq!(s.pop_eligible().map(|(_, p)| p), Some("deliver"));
    }

    #[test]
    fn local_occurrence_breaks_ties_deterministically_in_schedule_order() {
        let mut s: Scheduler<&str> = Scheduler::new(0);
        s.schedule(req(0, Phase::Deliver, "worker/A", 0), "first-scheduled");
        s.schedule(req(0, Phase::Deliver, "worker/A", 0), "second-scheduled");
        s.advance_to(0);
        assert_eq!(s.pop_eligible().map(|(_, p)| p), Some("first-scheduled"));
        assert_eq!(s.pop_eligible().map(|(_, p)| p), Some("second-scheduled"));
    }

    #[test]
    fn same_schedule_sequence_produces_the_same_pop_order_every_time() {
        fn run() -> Vec<&'static str> {
            let mut s: Scheduler<&str> = Scheduler::new(0);
            s.schedule(req(50, Phase::Deliver, "worker/B", 1), "b1");
            s.schedule(req(50, Phase::Deliver, "worker/A", 0), "a0");
            s.schedule(req(10, Phase::Tick, "job/X", 0), "x0");
            s.advance_to(1_000);
            let mut out = Vec::new();
            while let Some((_, p)) = s.pop_eligible() {
                out.push(p);
            }
            out
        }
        assert_eq!(run(), run());
    }

    #[test]
    fn full_quiescence_is_an_empty_queue() {
        let mut s: Scheduler<&str> = Scheduler::new(0);
        assert!(s.is_empty());
        s.schedule(req(0, Phase::Deliver, "worker/A", 0), "x");
        assert!(!s.is_empty());
        s.advance_to(0);
        s.pop_eligible();
        assert!(s.is_empty());
    }

    #[test]
    #[should_panic(expected = "cannot move backward")]
    fn scheduler_time_refuses_to_move_backward() {
        let mut s: Scheduler<&str> = Scheduler::new(1_000);
        s.advance_to(0);
    }
}
