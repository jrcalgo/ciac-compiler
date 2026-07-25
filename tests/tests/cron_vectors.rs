//! Shared cron vector table: every schedule sema accepts must also parse
//! with the `cron` crate the Rust backend embeds (with a leading seconds
//! field), and every schedule sema rejects must fail with `CIAC0037`
//! (`InvalidCron`). This is the cross-host check the v0.6 plan called for
//! and the v0.6 drop skipped — see the D1 defect in the v0.6.1 review.

use ciac_diagnostics::ErrorCode;
use ciac_integration_tests::compile;
use std::str::FromStr;

const VALID_SCHEDULES: &[&str] = &[
    "0 3 * * *",
    "*/5 * * * *",
    "0,30 * * * *",
    "0 0 1 1 *",
    "0-29 * * * *",
    "0 9-17 * * 1-5",
    "0 0 * * 0",
    "0 0 * * 7",
    "0 0 * * 0-3",
    "0 0 * * 0,2,4",
    "0 0 * * 0/2",
];

const INVALID_SCHEDULES: &[&str] = &[
    "0 3 * *",     // 4 fields
    "0 3 * * * *", // 6 fields
    "60 * * * *",  // minute out of range
    "0 24 * * *",  // hour out of range
    "0 0 32 * *",  // day out of range
    "0 0 * 13 *",  // month out of range
    "abc * * * *", // garbage
    "*/0 * * * *", // zero step
];

fn job_program(schedule: &str) -> String {
    format!(
        r#"
service CronVectorProbe;

use {{
    scheduler jobs Cron;
}}

job Tick {{
    schedule: "{schedule}";
}}

handler Prune {{}}

pipeline Tick:
    Prune;
"#
    )
}

#[test]
fn sema_accepted_schedules_parse_with_the_generated_rust_runtime() {
    for schedule in VALID_SCHEDULES {
        let (ir, diags) = compile(&job_program(schedule));
        assert!(
            !diags.has_errors(),
            "sema should accept schedule {schedule:?}: {:?}",
            diags.codes()
        );
        assert!(ir.is_some(), "schedule {schedule:?} should produce IR");

        let translated = ciac_codegen::model::cron_crate_schedule(schedule);
        cron::Schedule::from_str(&translated).unwrap_or_else(|err| {
            panic!(
                "cron crate rejected sema-accepted schedule {schedule:?} \
                 (translated to {translated:?}): {err}"
            )
        });
    }
}

/// v0.25 M3: Spring's own `CronExpression` parser (unlike Rust's
/// `cron` crate) already accepts CIaC's weekday `0`-`7` convention
/// natively — verified live, separately, by actually parsing every
/// shape below with `org.springframework.scheduling.support.
/// CronExpression` before writing this translation. This Rust-side
/// test proves the *string transformation* `spring_cron` performs
/// (a literal `"0 "` seconds-field prefix, nothing else) is applied
/// correctly to every schedule sema accepts — the two proofs together
/// are this milestone's "equivalence-tested against the same cases
/// the Rust translation carries."
#[test]
fn sema_accepted_schedules_translate_to_the_expected_spring_cron_shape() {
    for schedule in VALID_SCHEDULES {
        let translated = ciac_backend_java::filters::spring_cron(schedule);
        assert_eq!(
            translated,
            format!("0 {schedule}"),
            "spring_cron must be exactly a literal seconds-field prefix for {schedule:?}"
        );
    }
}

#[test]
fn sema_rejects_invalid_schedules_with_invalid_cron_code() {
    for schedule in INVALID_SCHEDULES {
        let (_, diags) = compile(&job_program(schedule));
        assert!(
            diags.has_errors(),
            "sema should reject schedule {schedule:?}"
        );
        assert!(
            diags.codes().contains(&ErrorCode::InvalidCron),
            "schedule {schedule:?} should fail with InvalidCron, got {:?}",
            diags.codes()
        );
    }
}
