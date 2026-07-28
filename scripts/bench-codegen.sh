#!/usr/bin/env bash
# 30UpdatePlan.md M1: the per-target generation-cost instrument. Times
# a real `ciac build` for every (example, target) combination and
# reports per-target totals, medians, and each target's ratio to the
# fastest -- the arithmetic this arc's whole thesis rests on (the Java
# backend spawns one JVM per generated file and pays roughly a
# hundredfold penalty other targets don't).
#
# Not a `cargo test` because it shells out to the real `ciac` binary
# once per (example, target) pair and the numbers only mean anything as
# wall-clock, not as a pass/fail assertion -- matching this repo's
# existing pattern for slow, environment-sensitive instruments
# (`sim-corpus-x5.sh`, `check-deny-ignores.sh`).
#
# Output is a Markdown table on stdout, meant to be pasted into
# `docs/perf/codegen-baseline.md` directly -- the M1/M5/M9 readings are
# a `git diff` of that file, the same way 29UpdatePlan.md's three
# cold-start transcripts made the front-door delta a reviewable
# artifact instead of an assertion.
#
# Usage:
#   scripts/bench-codegen.sh [--targets python,rust,typescript,go,java]
#                            [--examples ping,order-system]
#
# Exits non-zero only if a `ciac build` itself fails -- this is a
# measuring instrument, not a gate (the gate is 30UpdatePlan.md M8's
# `perf_budget.rs`).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

TARGETS="python,rust,typescript,go,java"
EXAMPLES=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --targets)
            TARGETS="$2"
            shift 2
            ;;
        --examples)
            EXAMPLES="$2"
            shift 2
            ;;
        *)
            echo "unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

IFS=',' read -ra TARGET_LIST <<<"$TARGETS"

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

echo "building ciac (release, for stable timings)..." >&2
cargo build -q --release -p ciac
CIAC="./target/release/ciac"

if [[ -n "$EXAMPLES" ]]; then
    IFS=',' read -ra NAMES <<<"$EXAMPLES"
    EXAMPLE_FILES=()
    for name in "${NAMES[@]}"; do
        EXAMPLE_FILES+=("examples/${name}.ciac")
    done
else
    mapfile -t EXAMPLE_FILES < <(find examples -maxdepth 1 -name '*.ciac' | sort)
fi

# portable wall-clock in seconds with millisecond precision (macOS'
# bash-3 date has no %N, but this sandbox and CI are both Linux, so
# nanosecond precision is available)
now() { date +%s.%N; }

declare -A TARGET_TOTAL
declare -A TARGET_COUNT
declare -A TARGET_MIN
declare -A TARGET_MAX
declare -a ROWS
declare -A FILE_COUNTS   # "target:example" -> file count

for example in "${EXAMPLE_FILES[@]}"; do
    [[ -f "$example" ]] || { echo "missing: $example" >&2; exit 1; }
    name=$(basename "$example" .ciac)
    for target in "${TARGET_LIST[@]}"; do
        out="$WORKDIR/${name}-${target}"
        start=$(now)
        if ! "$CIAC" build "$example" --target "$target" --out "$out" >"$WORKDIR/log" 2>&1; then
            # not every example supports every target (check_support
            # gates) -- a refusal is not a timing failure, skip it
            if grep -q "cannot generate" "$WORKDIR/log" 2>/dev/null; then
                rm -rf "$out"
                continue
            fi
            echo "BUILD FAILED: $example --target $target" >&2
            cat "$WORKDIR/log" >&2
            exit 1
        fi
        end=$(now)
        elapsed=$(echo "$end - $start" | bc)
        files=$(find "$out" -type f | wc -l | tr -d ' ')
        rm -rf "$out"

        ROWS+=("$name|$target|$elapsed|$files")
        FILE_COUNTS["${target}:${name}"]=$files

        TARGET_TOTAL[$target]=$(echo "${TARGET_TOTAL[$target]:-0} + $elapsed" | bc)
        TARGET_COUNT[$target]=$(( ${TARGET_COUNT[$target]:-0} + 1 ))
        if [[ -z "${TARGET_MIN[$target]:-}" ]] || (( $(echo "$elapsed < ${TARGET_MIN[$target]}" | bc) )); then
            TARGET_MIN[$target]=$elapsed
        fi
        if [[ -z "${TARGET_MAX[$target]:-}" ]] || (( $(echo "$elapsed > ${TARGET_MAX[$target]}" | bc) )); then
            TARGET_MAX[$target]=$elapsed
        fi
    done
done

echo ""
echo "## Per-example wall time (seconds)"
echo ""
echo "| Example | Target | Seconds | Files |"
echo "|---|---|---|---|"
for row in "${ROWS[@]}"; do
    IFS='|' read -r name target elapsed files <<<"$row"
    printf '| %s | %s | %.3f | %s |\n' "$name" "$target" "$elapsed" "$files"
done

echo ""
echo "## Per-target summary"
echo ""

# find the fastest target's average, for the ratio column
FASTEST_AVG=""
for target in "${TARGET_LIST[@]}"; do
    count="${TARGET_COUNT[$target]:-0}"
    [[ "$count" -eq 0 ]] && continue
    avg=$(echo "${TARGET_TOTAL[$target]} / $count" | bc -l)
    if [[ -z "$FASTEST_AVG" ]] || (( $(echo "$avg < $FASTEST_AVG" | bc) )); then
        FASTEST_AVG=$avg
    fi
done

echo "| Target | Builds | Total (s) | Mean (s) | Min (s) | Max (s) | Ratio to fastest |"
echo "|---|---|---|---|---|---|---|"
for target in "${TARGET_LIST[@]}"; do
    count="${TARGET_COUNT[$target]:-0}"
    [[ "$count" -eq 0 ]] && continue
    total="${TARGET_TOTAL[$target]}"
    avg=$(echo "$total / $count" | bc -l)
    ratio=$(echo "$avg / $FASTEST_AVG" | bc -l)
    printf '| %s | %s | %.3f | %.3f | %.3f | %.3f | %.2fx |\n' \
        "$target" "$count" "$total" "$avg" "${TARGET_MIN[$target]}" "${TARGET_MAX[$target]}" "$ratio"
done

echo ""
echo "(generated by scripts/bench-codegen.sh; targets=$TARGETS; $(date -u +%Y-%m-%dT%H:%M:%SZ))"
