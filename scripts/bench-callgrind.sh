#!/usr/bin/env bash
# `31UpdatePlan.md` M6: standalone callgrind instruction-count
# investigation tool, matching `scripts/bench-codegen.sh`'s own role
# for wall-clock timing -- a human-runnable, fast, narrower-scoped
# sibling to the actual gate. The gate itself
# (`tests/tests/perf_baseline.rs`) measures independently rather than
# shelling out to this script, mirroring the existing
# `bench-codegen.sh`/`tests/tests/perf_budget.rs` relationship: a
# script for people, a `#[test]` for CI, both measuring the same thing
# through separate, independently-correct code paths.
#
# `31UpdatePlan.md` M7 extends this script (rather than adding a new
# one) with `--metric allocations`: total heap bytes and block count
# via valgrind's DHAT tool, one number per example, for the *whole*
# `ciac build` invocation -- the same granularity M6's instruction-
# count measurement already uses, not a per-phase breakdown. A
# per-phase breakdown would need in-process client-request
# instrumentation (`VALGRIND_*` macros wired into `ciac` itself),
# which this arc avoids entirely rather than negotiating with the
# workspace's `unsafe_code = "forbid"` lint, per M7's own text.
# Reporting-only: no gate reads this output, matching Pillar 6's
# posture on newly-measured-for-the-first-time cost centres.
#
# Usage:
#   scripts/bench-callgrind.sh [--examples ping,order-system] [--target python] [--metric instructions|allocations]
#
# Requires valgrind on PATH. Exits non-zero only if a build or
# valgrind invocation itself fails -- this is a measuring instrument,
# not a gate (the gate is `31UpdatePlan.md` M6's own
# `tests/tests/perf_baseline.rs`).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

EXAMPLES="ping,order-system"
TARGET="python"
METRIC="instructions"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --examples)
            EXAMPLES="$2"
            shift 2
            ;;
        --target)
            TARGET="$2"
            shift 2
            ;;
        --metric)
            METRIC="$2"
            shift 2
            ;;
        *)
            echo "unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

case "$METRIC" in
    instructions|allocations) ;;
    *)
        echo "unknown --metric '$METRIC' -- expected 'instructions' or 'allocations'" >&2
        exit 1
        ;;
esac

if ! command -v valgrind >/dev/null 2>&1; then
    echo "valgrind not found on PATH -- required to measure $METRIC" >&2
    exit 1
fi

echo "building ciac (release, for stable measurements)..." >&2
cargo build -q --release -p ciac
CIAC="$(pwd)/target/release/ciac"

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

IFS=',' read -ra EXAMPLE_LIST <<<"$EXAMPLES"

if [[ "$METRIC" == "instructions" ]]; then
    echo ""
    echo "## Callgrind instruction counts: target=$TARGET"
    echo ""
    echo "| Example | Instructions |"
    echo "|---|---|"
    for name in "${EXAMPLE_LIST[@]}"; do
        example="examples/${name}.ciac"
        [[ -f "$example" ]] || { echo "missing: $example" >&2; exit 1; }
        out="$WORKDIR/${name}"
        cg_out="$WORKDIR/${name}.callgrind"
        rm -rf "$out"
        valgrind --tool=callgrind --callgrind-out-file="$cg_out" --cache-sim=no -q \
            "$CIAC" build "$example" --target "$TARGET" --out "$out" >"$WORKDIR/log" 2>&1
        instructions=$(grep "^summary:" "$cg_out" | awk '{print $2}')
        printf '| %s | %s |\n' "$name" "$instructions"
        rm -rf "$out" "$cg_out"
    done
else
    echo ""
    echo "## DHAT allocation totals (whole \`ciac build\` invocation): target=$TARGET"
    echo ""
    echo "| Example | Bytes | Blocks |"
    echo "|---|---|---|"
    for name in "${EXAMPLE_LIST[@]}"; do
        example="examples/${name}.ciac"
        [[ -f "$example" ]] || { echo "missing: $example" >&2; exit 1; }
        out="$WORKDIR/${name}"
        dhat_out="$WORKDIR/${name}.dhat"
        dhat_log="$WORKDIR/${name}.dhat.log"
        rm -rf "$out"
        valgrind --tool=dhat --dhat-out-file="$dhat_out" \
            "$CIAC" build "$example" --target "$TARGET" --out "$out" >/dev/null 2>"$dhat_log"
        # DHAT prints its own text summary to stderr, e.g.
        # "==PID== Total:     1,624,064 bytes in 8,871 blocks" -- parsed
        # here rather than the JSON sidecar file, matching M6's own
        # "parse the tool's plain-text summary line" convention.
        total_line=$(grep "Total:" "$dhat_log" | head -1)
        bytes=$(echo "$total_line" | sed -E 's/.*Total:\s*([0-9,]+) bytes.*/\1/' | tr -d ',')
        blocks=$(echo "$total_line" | sed -E 's/.*in\s*([0-9,]+) blocks.*/\1/' | tr -d ',')
        printf '| %s | %s | %s |\n' "$name" "$bytes" "$blocks"
        rm -rf "$out" "$dhat_out" "$dhat_log"
    done
fi

if [[ "$METRIC" == "instructions" ]]; then
    echo ""
    echo "Compare against the committed \`docs/perf/baseline.json\`'s own"
    echo "\`instruction_counts\` field by hand, or run the real gate:"
    echo ""
    echo '```sh'
    echo "cargo test -p ciac-integration-tests --test perf_baseline -- --ignored"
    echo '```'
    echo ""
    echo "To refresh the committed baseline's instruction counts (requires a"
    echo "justification in the commit message, per 31UpdatePlan.md's own"
    echo "discipline):"
    echo ""
    echo '```sh'
    echo "cargo run --release -p ciac-integration-tests --bin ciac-bench -- --update-baseline --with-callgrind"
    echo '```'
else
    echo ""
    echo "Allocation totals are reporting-only (31UpdatePlan.md M7) -- no"
    echo "gate reads this output and no baseline field stores it."
fi
echo ""
echo "(generated by scripts/bench-callgrind.sh; examples=$EXAMPLES target=$TARGET metric=$METRIC; $(date -u +%Y-%m-%dT%H:%M:%SZ))"
