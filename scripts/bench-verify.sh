#!/usr/bin/env bash
# `31UpdatePlan.md` M3: per-`ValidateStep` timing attribution.
#
# `ciac verify` already runs each target's own `TargetInfo::validate`
# steps in order (`crates/ciac/src/commands.rs`'s `validate_generated`/
# `run_validate_step`) and this repo already knows the *total* cost
# per target from `docs/perf/codegen-baseline.md`'s own numbers -- but
# nothing has ever reported which step inside that total is expensive.
# "Rust verify takes 300s" tells you Rust is slow; "cargo test takes
# 280s of it" tells you what to attack.
#
# This script cannot get that breakdown by calling into `ciac verify`
# itself: doing so would mean editing `crates/ciac/src/commands.rs`'s
# validate loop to emit per-step timing, which is a behavior change to
# `ciac verify`'s own output and out of scope for this arc's
# instrument-only contract (no compiler behavior changes at all, not
# even an extra line of stderr). Instead, this script independently
# runs each target's validate steps and times each one -- which means
# it duplicates the step lists each backend's own `TARGET_INFO.validate`
# already declares as the single source of truth
# (`crates/ciac-backend-<target>/src/lib.rs`). That duplication is
# disclosed here rather than hidden: if a target's own `validate` list
# changes, this script's copy must be updated to match, the same
# manual-sync discipline `scripts/sync-vendored-ciac-assets.sh`
# already asks of its own two vendored-copy call sites.
#
# Usage:
#   scripts/bench-verify.sh --target python [--example order-system] [--format table|json]
#   scripts/bench-verify.sh --target rust
#   scripts/bench-verify.sh --target typescript
#   scripts/bench-verify.sh --target go
#   scripts/bench-verify.sh --target java
#
# `31UpdatePlan.md` M8 adds `--format json`: a `[{"step","purpose",
# "seconds"}, ...]` array on stdout (everything else this script prints
# moves to stderr in that mode), consumed by
# `ciac_integration_tests::bench::measure_verify_steps` to wire this
# script's own numbers into `docs/perf/baseline.json` -- the
# already-disclosed step-list duplication against each backend's
# `TARGET_INFO.validate` stays in exactly one place (this script), and
# the Rust side reuses it by shelling out rather than adding a second,
# competing implementation of "what are Python's validate steps."
#
# Exits non-zero if any validate step itself fails -- this is a
# measuring instrument, not a gate.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

TARGET=""
EXAMPLE="order-system"
FORMAT="table"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --target)
            TARGET="$2"
            shift 2
            ;;
        --example)
            EXAMPLE="$2"
            shift 2
            ;;
        --format)
            FORMAT="$2"
            shift 2
            ;;
        *)
            echo "unknown argument: $1" >&2
            exit 1
            ;;
    esac
done

if [[ -z "$TARGET" ]]; then
    echo "usage: $0 --target <python|rust|typescript|go|java> [--example NAME] [--format table|json]" >&2
    exit 1
fi
case "$FORMAT" in
    table|json) ;;
    *)
        echo "unknown --format '$FORMAT' -- expected 'table' or 'json'" >&2
        exit 1
        ;;
esac

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

echo "building ciac (release, for stable timings)..." >&2
cargo build -q --release -p ciac
CIAC="./target/release/ciac"

OUT="$WORKDIR/project"
echo "generating $EXAMPLE for $TARGET..." >&2
"$CIAC" build "examples/${EXAMPLE}.ciac" --target "$TARGET" --out "$OUT" >&2

# Every generated project's marker file (`pyproject.toml`,
# `Cargo.toml`, `package.json`, `go.mod`, `pom.xml`) sits at $OUT's own
# root for every current target's example corpus -- this script does
# not need `find_project_dirs`'s own multi-service-subdirectory walk,
# since M3's own scope is a single representative pair, not the full
# corpus `validate_generated` handles.
now() { date +%s.%N; }
declare -a ROWS

run_step() {
    local purpose="$1"
    shift
    local start end elapsed
    start=$(now)
    if ! "$@" >"$WORKDIR/step-log" 2>&1; then
        echo "STEP FAILED ($purpose): $*" >&2
        cat "$WORKDIR/step-log" >&2
        exit 1
    fi
    end=$(now)
    elapsed=$(echo "$end - $start" | bc)
    ROWS+=("$*|$purpose|$elapsed")
}

# `cd` once, run every step directly (no `( cd ... && run_step ... )`
# subshell around each step): `ROWS+=(...)` inside `run_step` mutates
# an array, and an array mutation made inside a subshell is invisible
# to the parent shell the instant that subshell exits -- confirmed
# live during this milestone's own review pass, where the first
# version of this script (one subshell per step) ran every step
# successfully but printed an empty table, because every `ROWS+=`
# landed in a subshell nobody kept.
ORIG_DIR=$(pwd)
cd "$OUT"

case "$TARGET" in
    python)
        run_step "dependencies installed" uv sync -q
        run_step "lints" uv run ruff check .
        run_step "unit tests pass" uv run pytest -q
        ;;
    rust)
        # Mirrors the collapsed single-step ladder in
        # `crates/ciac-backend-rust/src/lib.rs`'s own `TARGET_INFO`
        # (30UpdatePlan.md-era `cargo check` + `cargo test` collapsed
        # into one `cargo test` carrying `-D warnings`, since the two
        # separate steps shared no build cache and the combined form
        # proves strictly more in less time) -- one step, not two.
        RUSTFLAGS="-D warnings" run_step "type-checks (deny warnings), unit and generated tests pass" cargo test -q --lib --tests
        ;;
    typescript)
        run_step "install dependencies from the checked-in lockfile" npm ci
        run_step "type-checks" npx tsc --noEmit
        run_step "lint" npx eslint .
        run_step "test" npx vitest run
        ;;
    go)
        CGO_ENABLED=0 run_step "compiles to a static binary" go build ./...
        CGO_ENABLED=0 run_step "lints" go vet ./...
        run_step "formatting is golden bytes, not a post-pass" gofmt -l .
        CGO_ENABLED=0 run_step "unit tests pass" go test ./...
        ;;
    java)
        run_step "compiles, formats (Spotless), and tests in one invocation" ./mvnw -q -B verify
        ;;
    *)
        cd "$ORIG_DIR"
        echo "unknown target: $TARGET (expected python, rust, typescript, go, or java)" >&2
        exit 1
        ;;
esac

cd "$ORIG_DIR"

if [[ "$FORMAT" == "json" ]]; then
    json_rows=()
    for row in "${ROWS[@]}"; do
        IFS='|' read -r step purpose elapsed <<<"$row"
        json_rows+=("$(jq -n --arg step "$step" --arg purpose "$purpose" --argjson seconds "$elapsed" \
            '{step: $step, purpose: $purpose, seconds: $seconds}')")
    done
    printf '%s\n' "${json_rows[@]}" | jq -s '.'
    exit 0
fi

echo ""
echo "## Per-step timing: $TARGET / $EXAMPLE"
echo ""
echo "| Step | Purpose | Seconds |"
echo "|---|---|---|"
total=0
for row in "${ROWS[@]}"; do
    IFS='|' read -r step purpose elapsed <<<"$row"
    printf '| `%s` | %s | %.3f |\n' "$step" "$purpose" "$elapsed"
    total=$(echo "$total + $elapsed" | bc)
done
echo ""
printf 'Total: %.3fs\n' "$total"
echo ""
echo "(generated by scripts/bench-verify.sh; target=$TARGET example=$EXAMPLE; $(date -u +%Y-%m-%dT%H:%M:%SZ))"
