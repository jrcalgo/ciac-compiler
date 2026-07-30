//! A 5-field cron evaluator matching the exact grammar
//! `ciac-sema::build.rs`'s `valid_cron` already validates at compile
//! time (`CIAC0037`): `minute(0-59) hour(0-23) day(1-31) month(1-12)
//! weekday(0-7, both 0 and 7 = Sunday)`, each field a comma-separated
//! list of `*`, `N`, `N-M`, optionally suffixed `/step`.
//!
//! This is a from-scratch evaluator, not the generated Rust project's
//! own `cron` crate dependency (`Cargo.toml.j2`) -- that crate expects
//! a 6/7-field seconds-first expression, a different grammar than the
//! 5-field one CIaC's own sema already validates, so reusing it would
//! mean re-deriving the mapping rather than avoiding one. `chrono`
//! supplies calendar arithmetic (leap years, month lengths, weekdays);
//! field matching is CIaC's own grammar, hand-written to match
//! `valid_cron_part` exactly.

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronError(pub String);

impl std::fmt::Display for CronError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid cron expression: {}", self.0)
    }
}

impl std::error::Error for CronError {}

/// A parsed 5-field schedule. Each field is the closed set of matching
/// values (`*` expands to every value in range) rather than kept as
/// syntax, so matching a candidate instant is a handful of set lookups.
#[derive(Debug, Clone)]
pub struct CronSchedule {
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    days: BTreeSet<u32>,
    months: BTreeSet<u32>,
    /// Normalized to `0..=6`, Sunday = 0 (both the grammar's `0` and
    /// `7` collapse here, matching `chrono::Weekday::num_days_from_sunday`).
    weekdays: BTreeSet<u32>,
}

/// Bounds how far a search for the next fire (or a catch-up scan) may
/// look before giving up -- a schedule that can never fire (impossible
/// day-of-month/month combination, e.g. `* * 31 2 *`) must not spin
/// forever. Five years of minutes is generous for any real schedule and
/// still resolves in low-single-digit milliseconds.
const MAX_LOOKAHEAD_MINUTES: i64 = 5 * 366 * 24 * 60;

impl CronSchedule {
    pub fn parse(expr: &str) -> Result<CronSchedule, CronError> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(CronError(format!(
                "expected 5 whitespace-separated fields, found {}",
                fields.len()
            )));
        }
        let minutes = parse_field(fields[0], 0, 59)?;
        let hours = parse_field(fields[1], 0, 23)?;
        let days = parse_field(fields[2], 1, 31)?;
        let months = parse_field(fields[3], 1, 12)?;
        let raw_weekdays = parse_field(fields[4], 0, 7)?;
        let weekdays = raw_weekdays.into_iter().map(|d| d % 7).collect();
        Ok(CronSchedule {
            minutes,
            hours,
            days,
            months,
            weekdays,
        })
    }

    fn matches(&self, at: &DateTime<Utc>) -> bool {
        self.minutes.contains(&at.minute())
            && self.hours.contains(&at.hour())
            && self.days.contains(&at.day())
            && self.months.contains(&at.month())
            && self.weekdays.contains(&at.weekday().num_days_from_sunday())
    }

    /// The first matching minute-aligned instant strictly after
    /// `after_ms`, or `None` if none is found within
    /// [`MAX_LOOKAHEAD_MINUTES`].
    pub fn next_fire_after(&self, after_ms: i64) -> Option<i64> {
        let after = ms_to_datetime(after_ms);
        let mut candidate = truncate_to_minute(after) + Duration::minutes(1);
        for _ in 0..MAX_LOOKAHEAD_MINUTES {
            if self.matches(&candidate) {
                return Some(candidate.timestamp_millis());
            }
            candidate += Duration::minutes(1);
        }
        None
    }

    /// Every matching instant in `(from_ms, to_ms]`, oldest first,
    /// bounded by `cap` -- the `catch_up` ladder (see
    /// `docs/simulation.md` once M9 writes it) is explicit that catch-up
    /// work is bounded, never an unbounded backlog replay.
    pub fn due_instants(&self, from_ms: i64, to_ms: i64, cap: usize) -> Vec<i64> {
        let mut out = Vec::new();
        let mut cursor = from_ms;
        while out.len() < cap {
            match self.next_fire_after(cursor) {
                Some(fire_ms) if fire_ms <= to_ms => {
                    out.push(fire_ms);
                    cursor = fire_ms;
                }
                _ => break,
            }
        }
        out
    }
}

fn parse_field(field: &str, min: u32, max: u32) -> Result<BTreeSet<u32>, CronError> {
    let mut out = BTreeSet::new();
    for part in field.split(',') {
        let (base, step) = match part.split_once('/') {
            Some((base, step)) => {
                let step: u32 = step
                    .parse()
                    .map_err(|_| CronError(format!("invalid step in `{part}`")))?;
                if step == 0 {
                    return Err(CronError(format!("step cannot be zero in `{part}`")));
                }
                (base, step)
            }
            None => (part, 1),
        };
        let (start, end) = if base == "*" {
            (min, max)
        } else if let Some((s, e)) = base.split_once('-') {
            let s: u32 = s
                .parse()
                .map_err(|_| CronError(format!("invalid range start in `{part}`")))?;
            let e: u32 = e
                .parse()
                .map_err(|_| CronError(format!("invalid range end in `{part}`")))?;
            if !(min <= s && s <= e && e <= max) {
                return Err(CronError(format!(
                    "range `{part}` out of bounds {min}-{max}"
                )));
            }
            (s, e)
        } else {
            let v: u32 = base
                .parse()
                .map_err(|_| CronError(format!("invalid value `{base}`")))?;
            if !(min <= v && v <= max) {
                return Err(CronError(format!("value `{v}` out of bounds {min}-{max}")));
            }
            (v, v)
        };
        let mut v = start;
        while v <= end {
            out.insert(v);
            v += step;
        }
    }
    Ok(out)
}

fn ms_to_datetime(ms: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(ms).single().expect(
        "virtual time is always constructed from a valid epoch millisecond, never out of range",
    )
}

fn truncate_to_minute(at: DateTime<Utc>) -> DateTime<Utc> {
    at - Duration::seconds(at.second() as i64)
        - Duration::milliseconds(at.timestamp_subsec_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt_ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
            .single()
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn daily_three_am_fires_once_per_day() {
        let schedule = CronSchedule::parse("0 3 * * *").unwrap();
        let start = dt_ms(2030, 1, 1, 0, 0);
        let first = schedule.next_fire_after(start).unwrap();
        assert_eq!(first, dt_ms(2030, 1, 1, 3, 0));
        let second = schedule.next_fire_after(first).unwrap();
        assert_eq!(second, dt_ms(2030, 1, 2, 3, 0));
    }

    #[test]
    fn advance_24_hours_observes_the_0300_job_exactly_once() {
        // Direct fixture from 17UpdatePlan.md's own Pillar 6 list.
        let schedule = CronSchedule::parse("0 3 * * *").unwrap();
        let start = dt_ms(2030, 1, 1, 0, 0);
        let end = start + 24 * 60 * 60 * 1000;
        let due = schedule.due_instants(start, end, 100);
        assert_eq!(due.len(), 1);
        assert_eq!(due[0], dt_ms(2030, 1, 1, 3, 0));
    }

    #[test]
    fn catch_up_true_returns_every_missed_instant_oldest_first() {
        let schedule = CronSchedule::parse("0 * * * *").unwrap(); // hourly
        let start = dt_ms(2030, 1, 1, 0, 30);
        let end = dt_ms(2030, 1, 1, 5, 30);
        let due = schedule.due_instants(start, end, 100);
        // Fires at 01:00, 02:00, 03:00, 04:00, 05:00 -- five missed
        // instants, strictly increasing.
        assert_eq!(due.len(), 5);
        assert!(due.windows(2).all(|w| w[0] < w[1]));
        assert_eq!(due[0], dt_ms(2030, 1, 1, 1, 0));
        assert_eq!(due[4], dt_ms(2030, 1, 1, 5, 0));
    }

    #[test]
    fn catch_up_false_coalesces_to_the_single_latest_instant() {
        // The schedule evaluator always returns every due instant;
        // `catch_up: false`'s coalescing is the scheduler's policy over
        // this list (take the last, skip the rest) -- proven here by
        // showing the caller has everything needed to implement either
        // policy from the same due_instants() result.
        let schedule = CronSchedule::parse("0 * * * *").unwrap();
        let start = dt_ms(2030, 1, 1, 0, 30);
        let end = dt_ms(2030, 1, 1, 5, 30);
        let due = schedule.due_instants(start, end, 100);
        let coalesced = due.last().copied();
        assert_eq!(coalesced, Some(dt_ms(2030, 1, 1, 5, 0)));
    }

    #[test]
    fn due_instants_is_bounded_by_cap() {
        let schedule = CronSchedule::parse("* * * * *").unwrap(); // every minute
        let start = dt_ms(2030, 1, 1, 0, 0);
        let end = dt_ms(2030, 2, 1, 0, 0); // a month of minutes
        let due = schedule.due_instants(start, end, 10);
        assert_eq!(due.len(), 10, "bounded, not an unbounded backlog replay");
    }

    #[test]
    fn step_and_range_and_list_fields_parse_and_match() {
        let schedule = CronSchedule::parse("*/15 9-17 * * 1,3,5").unwrap();
        // 2030-01-07 is a Monday.
        let monday_9am = dt_ms(2030, 1, 7, 9, 0);
        assert!(schedule.next_fire_after(monday_9am - 1) == Some(monday_9am));
        let monday_915 = schedule.next_fire_after(monday_9am).unwrap();
        assert_eq!(monday_915, dt_ms(2030, 1, 7, 9, 15));
    }

    #[test]
    fn weekday_seven_and_zero_both_mean_sunday() {
        let zero = CronSchedule::parse("0 0 * * 0").unwrap();
        let seven = CronSchedule::parse("0 0 * * 7").unwrap();
        // 2030-01-06 is a Sunday.
        let sunday_midnight = dt_ms(2030, 1, 6, 0, 0);
        let start = sunday_midnight - 1;
        assert_eq!(zero.next_fire_after(start), Some(sunday_midnight));
        assert_eq!(seven.next_fire_after(start), Some(sunday_midnight));
    }

    #[test]
    fn impossible_schedule_bounds_out_rather_than_looping_forever() {
        // February never has a 31st.
        let schedule = CronSchedule::parse("0 0 31 2 *").unwrap();
        let start = dt_ms(2030, 1, 1, 0, 0);
        assert_eq!(schedule.next_fire_after(start), None);
    }

    #[test]
    fn rejects_malformed_expressions() {
        assert!(CronSchedule::parse("not a cron").is_err());
        assert!(CronSchedule::parse("0 0 0 0 0 0").is_err());
        assert!(CronSchedule::parse("60 * * * *").is_err());
    }
}
