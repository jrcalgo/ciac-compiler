#!/usr/bin/env bash
# 27UpdatePlan.md M5: the corpus-runs-×N-identical harness (the "×5
# harness", executing ×2 today — Python and Rust, the only targets at
# SimSupport::Full/gate-empty as of M4; TS/Go/Java join as M6-M8 land
# their own restatements, Python's remaining verb-family closure is
# M9's). Not a `cargo test` because it compiles a generated Rust
# project per (program, target) pair (the same cargo-build cost every
# manual M2-M4 live-verification pass already paid) — too slow for the
# default workspace test suite, so it stays a standalone script,
# matching this repo's existing pattern (`check-deny-ignores.sh`).
#
# Usage: scripts/sim-corpus-x5.sh [--targets python,rust]
#
# Exits non-zero if any (program, scenario, target) combination fails.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

TARGETS="${SIM_CORPUS_TARGETS:-python,rust}"
if [[ "${1:-}" == "--targets" ]]; then
    TARGETS="$2"
fi
IFS=',' read -ra TARGET_LIST <<<"$TARGETS"

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

cargo build -p ciac >/dev/null
CIAC="./target/debug/ciac"

# (program, scenario...) pairs -- every corpus scenario paired with
# the single example program it drives, per the scenario files' own
# "service" fields (confirmed by direct inspection, not guessed).
declare -A PROGRAM_SCENARIOS=(
    [examples/sim-peripherals.ciac]="sim/cache-ttl.ciac-sim.json sim/auth-scopes.ciac-sim.json sim/http-fixtures.ciac-sim.json sim/peripherals.ciac-sim.json"
    [examples/sim-vertical-slice.ciac]="sim/vertical-slice.ciac-sim.json sim/virtual-week.ciac-sim.json"
    [examples/sim-broker-slice.ciac]="sim/fanout.ciac-sim.json"
    [examples/domain-orders.ciac]="sim/relational-depth.ciac-sim.json sim/atomic-batch.ciac-sim.json"
)

FAILED=0
TOTAL=0
printf '%-32s %-12s %-40s %s\n' "PROGRAM" "TARGET" "SCENARIO" "RESULT"
for program in "${!PROGRAM_SCENARIOS[@]}"; do
    for target in "${TARGET_LIST[@]}"; do
        out="$WORKDIR/$(basename "$program" .ciac)-$target"
        # shellcheck disable=SC2086
        scenarios=(${PROGRAM_SCENARIOS[$program]})
        scenario_args=()
        for s in "${scenarios[@]}"; do
            scenario_args+=(--scenario "$s")
        done
        rm -rf "$out"
        TOTAL=$((TOTAL + 1))
        if result=$("$CIAC" sim --target "$target" --out "$out" "${scenario_args[@]}" "$program" 2>&1); then
            status="ok"
        else
            status="ERROR"
            FAILED=$((FAILED + 1))
        fi
        while IFS= read -r line; do
            if [[ "$line" == \[PASS\]* || "$line" == \[FAIL\]* ]]; then
                name="${line#* }"
                mark="${line%% *}"
                printf '%-32s %-12s %-40s %s\n' "$(basename "$program")" "$target" "$name" "$mark"
                [[ "$mark" == "[FAIL]" ]] && FAILED=$((FAILED + 1))
            fi
        done <<<"$result"
        if [[ "$status" == "ERROR" ]]; then
            printf '%-32s %-12s %-40s %s\n' "$(basename "$program")" "$target" "(build/refusal)" "ERROR: $(echo "$result" | tail -1)"
        fi
        # A generated Rust project's own `target/` dir (a full
        # dependency tree per program×target combination) is multiple
        # GB; deleting it immediately once this combination's result
        # is captured is what keeps a full corpus run from exhausting
        # disk mid-run (found live: an early run of this script hit
        # exactly that on its last combination).
        rm -rf "$out"
    done
done

echo
if [[ "$FAILED" -eq 0 ]]; then
    echo "sim-corpus-x${#TARGET_LIST[@]}: all combinations green ($TOTAL program×target runs)."
else
    echo "sim-corpus-x${#TARGET_LIST[@]}: $FAILED failing combination(s) out of $TOTAL." >&2
    exit 1
fi
