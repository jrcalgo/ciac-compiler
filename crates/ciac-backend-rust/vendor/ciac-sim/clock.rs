//! Virtual time and deterministic entropy (17UpdatePlan.md Pillar 6,
//! "One clock"). Two independent streams, per the plan's own
//! distinction: virtual time drives `Timestamp.now()`, retry
//! eligibility, cache/token expiry, cron/`catch_up`, and event
//! timestamps; a *separate* seeded stream drives generated UUIDs and
//! scheduler tie-breaking, so advancing the clock never perturbs which
//! ID a handler generates and vice versa.
//!
//! Production adapters use host time/entropy; this module exists only
//! for the simulation side of the `production()`/`simulation()`
//! boundary `ciac-backend-python`'s `AppState` already established
//! (v0.17 M3).

use serde::Serialize;

/// Virtual time as integer epoch milliseconds -- an integer, not a
/// `TIMESTAMPTZ`-shaped value, so no timezone/precision ambiguity can
/// creep into scheduling comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct VirtualClock {
    now_ms: i64,
}

impl VirtualClock {
    pub fn new(start_at_ms: i64) -> VirtualClock {
        VirtualClock {
            now_ms: start_at_ms,
        }
    }

    pub fn now_ms(&self) -> i64 {
        self.now_ms
    }

    /// Advances the clock forward. A scenario's `{"advance": {"by":
    /// "7d"}}` step resolves to a millisecond delta before reaching
    /// here -- this type only enforces that time is monotonic, never
    /// interprets duration syntax itself.
    ///
    /// # Panics
    /// If `to_ms` is before the current time -- virtual time never
    /// moves backward; a caller computing a negative delta has a bug
    /// worth failing loudly on, not silently clamping.
    pub fn advance_to(&mut self, to_ms: i64) {
        assert!(
            to_ms >= self.now_ms,
            "virtual clock cannot move backward: at {}, asked for {to_ms}",
            self.now_ms
        );
        self.now_ms = to_ms;
    }
}

/// A deterministic, seeded stream for generated UUIDs and scheduler
/// tie-breaking -- splitmix64, chosen for being a small, dependency-
/// free, well-known deterministic generator with good avalanche
/// behavior; this is a reproducibility tool, not a security primitive,
/// so a cryptographic RNG would be the wrong tool for the job.
#[derive(Debug, Clone, Serialize)]
pub struct Entropy {
    state: u64,
}

impl Entropy {
    pub fn new(seed: u64) -> Entropy {
        Entropy { state: seed }
    }

    /// The next raw 64 bits of the stream.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// A deterministic value in the shape of a UUID (v4-like: version
    /// and variant bits set so it *looks* like `Uuid.new()`'s output to
    /// generated/user code that inspects those bits) but derived purely
    /// from this seeded stream, never the host's random source.
    pub fn next_uuid(&mut self) -> String {
        let hi = self.next_u64();
        let lo = self.next_u64();
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&hi.to_be_bytes());
        bytes[8..].copy_from_slice(&lo.to_be_bytes());
        bytes[6] = (bytes[6] & 0x0F) | 0x40; // version 4
        bytes[8] = (bytes[8] & 0x3F) | 0x80; // variant 10xx
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3],
            bytes[4], bytes[5],
            bytes[6], bytes[7],
            bytes[8], bytes[9],
            bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_advances_monotonically() {
        let mut clock = VirtualClock::new(1_000);
        assert_eq!(clock.now_ms(), 1_000);
        clock.advance_to(5_000);
        assert_eq!(clock.now_ms(), 5_000);
    }

    #[test]
    #[should_panic(expected = "cannot move backward")]
    fn clock_refuses_to_move_backward() {
        let mut clock = VirtualClock::new(5_000);
        clock.advance_to(1_000);
    }

    #[test]
    fn same_seed_produces_the_same_stream() {
        let mut a = Entropy::new(42);
        let mut b = Entropy::new(42);
        for _ in 0..10 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Entropy::new(1);
        let mut b = Entropy::new(2);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn uuids_look_like_uuid_v4_and_are_deterministic() {
        let mut a = Entropy::new(7);
        let mut b = Entropy::new(7);
        let u1 = a.next_uuid();
        let u2 = b.next_uuid();
        assert_eq!(u1, u2);
        assert_eq!(u1.len(), 36);
        assert_eq!(u1.chars().nth(14), Some('4'));
        let variant = u16::from_str_radix(&u1[19..20], 16).unwrap();
        assert!((0x8..=0xb).contains(&variant));
    }

    #[test]
    fn advancing_the_clock_never_perturbs_entropy() {
        // Two structs, no shared state: advancing one has no back-
        // channel into the other's stream position.
        let mut clock = VirtualClock::new(0);
        let mut entropy = Entropy::new(99);
        let before = entropy.next_u64();
        clock.advance_to(1_000_000);
        let mut fresh = Entropy::new(99);
        assert_eq!(before, fresh.next_u64());
    }
}
