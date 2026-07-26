//! The `SIM` runtime-outcome registry (17UpdatePlan.md "Diagnostics and
//! limits"): a separately versioned code space from `ciac_diagnostics`'s
//! `CIAC` codes. `CIAC*` codes describe invalid *source*; `SIM*` codes
//! describe a simulation *run's* outcome — a scenario failing, a limit
//! hit, a replay mismatch. Deliberately mirrors
//! `ciac_diagnostics::code::ErrorCode`'s macro-generated shape so both
//! registries stay structurally consistent without sharing state.

macro_rules! sim_codes {
    ($($variant:ident = ($code:literal, $title:literal, $explanation:literal),)*) => {
        /// Stable, documented outcome codes for a simulation run.
        /// Append-only, like `CIAC` codes: once published, a code's
        /// meaning never changes.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize)]
        #[serde(into = "&'static str")]
        pub enum SimCode {
            $($variant,)*
        }

        impl SimCode {
            pub const ALL: &'static [SimCode] = &[$(SimCode::$variant,)*];

            /// The stable code string, e.g. `SIM0001`.
            pub fn code(self) -> &'static str {
                match self {
                    $(SimCode::$variant => $code,)*
                }
            }

            pub fn title(self) -> &'static str {
                match self {
                    $(SimCode::$variant => $title,)*
                }
            }

            pub fn explanation(self) -> &'static str {
                match self {
                    $(SimCode::$variant => $explanation,)*
                }
            }

            pub fn parse(s: &str) -> Option<SimCode> {
                let upper = s.to_ascii_uppercase();
                Self::ALL.iter().copied().find(|c| c.code() == upper)
            }
        }

        impl From<SimCode> for &'static str {
            fn from(code: SimCode) -> &'static str {
                code.code()
            }
        }
    };
}

sim_codes! {
    AssertionFailed = (
        "SIM0001",
        "assertion failed",
        "A scenario's `expect` step did not observe the state it \
         declared -- the message names which assertion and what was \
         observed instead."
    ),
    UnhandledEffectError = (
        "SIM0002",
        "unhandled handler/capability error",
        "Real generated or user-authored logic raised an error the \
         scenario did not declare as expected (via an injected failure \
         or an `expect` for a typed error response)."
    ),
    LimitExceeded = (
        "SIM0003",
        "effect/message/row/time/wall limit exceeded",
        "A bounded per-case limit (semantic effects, messages/delivery \
         attempts, rows, payload bytes, catch-up ticks, virtual-time \
         span, transcript bytes, or wall time) was reached. This is \
         always a failure, never a truncated success."
    ),
    Stalled = (
        "SIM0004",
        "pending work cannot progress",
        "The scheduler reached a state where scenario actions remain \
         but no event is eligible to run (no immediate action, no due \
         message/retry, no requested clock advance, no in-flight \
         effect) -- quiescence without completion."
    ),
    ReplayMismatch = (
        "SIM0005",
        "replay mismatch or divergence",
        "A `--replay` run produced a transcript that diverges from the \
         recorded one, or the replay artifact's plan/source/adapter/\
         scenario version doesn't match the current compiler."
    ),
    MissingExternalFixture = (
        "SIM0006",
        "missing external fixture",
        "A scenario exercised an external HTTP call (or other strict \
         ordered fixture) with no matching fixture declared -- \
         unmatched external calls fail closed rather than reaching the \
         network."
    ),
    RequiredFailureRuleUnmatched = (
        "SIM0007",
        "required failure rule unmatched",
        "A scenario declared a failure-injection rule that never \
         matched any real effect during the run. A required rule that \
         doesn't fire is a scenario error (most often a typo in the \
         selector), not a silently-skipped no-op."
    ),
    ReachedUnimplementedSeed = (
        "SIM0008",
        "reached unchanged TODO seed",
        "A selected scenario reached a classic or `extern` handler \
         whose seeded file is still byte-identical to its generated \
         TODO stub -- the behavior it would exercise was never written."
    ),
    EffectEscapedSeam = (
        "SIM0009",
        "effect escaped generated seam",
        "User-authored code performed a real capability effect (direct \
         SQL, HTTP, filesystem, subprocess, host clock, ...) that \
         bypassed the generated port instead of going through it. This \
         is a hard failure, not a silently-ignored gap -- simulation \
         cannot claim determinism for an effect it didn't observe."
    ),
    ReplayNotSupported = (
        "SIM0010",
        "--record/--replay not supported on this target",
        "27UpdatePlan.md M1: record/replay is its own capability, \
         decoupled from simulation depth -- a target can simulate every \
         verb the language has (`SimSupport::Full`) and still not \
         implement a replay tape. `TargetInfo::sim_replay` names which \
         targets do; today only Python's runner does. This is a \
         disclosed scope limit, not a bug: the target's own generated \
         runner has no plan/source-hash arguments and no transcript \
         format to replay against."
    ),
    UnknownService = (
        "SIM0011",
        "scenario references an unknown service",
        "28UpdatePlan.md M1: a scenario's own `request.service`, \
         `given.db[].service`, or `expect.row.service` named a service \
         the program's plan has no record of -- the message names the \
         unknown value and lists every service the plan actually knows, \
         checked once at `ciac sim` invocation, before any scenario \
         step runs."
    ),
}

impl std::fmt::Display for SimCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for code in SimCode::ALL {
            let s = code.code();
            assert!(s.starts_with("SIM") && s.len() == 7, "malformed code {s}");
            assert!(seen.insert(s), "duplicate code {s}");
        }
    }

    #[test]
    fn parse_roundtrip() {
        assert_eq!(SimCode::parse("sim0009"), Some(SimCode::EffectEscapedSeam));
        assert_eq!(SimCode::parse("SIM9999"), None);
    }
}
