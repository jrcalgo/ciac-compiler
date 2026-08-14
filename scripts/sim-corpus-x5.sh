#!/usr/bin/env bash
# 27UpdatePlan.md M5-M9: the corpus-runs-×N-identical harness (the "×5
# harness" -- ×2 at M4 (Python/Rust, the only targets at SimSupport::
# Full/gate-empty then), growing to all five targets as TS/Go/Java
# restated to full parity (M6-M8) and Python's own residual verb-family
# gap (db.update/query/count/delete_where predicates) closed at M9.
# "×5" names the target count, not repeated runs of one target --
# every scenario below is asserted to produce the *same* outcome on
# all five, per Pillar 4 ("structure may diverge; answers may not").
# Not a `cargo test` because it compiles a generated Rust project per
# (program, target) pair (the same cargo-build cost every manual M2-M9
# live-verification pass already paid) -- too slow for the default
# workspace test suite, so it stays a standalone script, matching this
# repo's existing pattern (`check-deny-ignores.sh`).
#
# Usage: scripts/sim-corpus-x5.sh [--targets python,rust,typescript,go,java]
#
# Exits non-zero if any (program, scenario, target) combination fails.
#
# `34UpdatePlan.md` M3 rewrote the tally onto per-combination status
# files (one `.out`, one `.rc` per slot under `$RESULTS`) instead of
# shell counters, and split the old single, unit-conflating `$FAILED`
# into `failed_combinations`/`failed_scenarios`, reported separately.
# `ciac sim` itself (the `commands.rs` CLI, not this script) reuses the
# same shared, persistent cargo target directory `ciac verify` already
# uses (M2), so repeat rust builds across programs/runs are cached --
# nothing here sets `CARGO_TARGET_DIR`. Parallelizing this script by
# target (M4) was implemented and measured but **not shipped**: on the
# 4-vCPU sandbox this arc was built and measured on, five concurrent
# streams slowed the dominant (rust) stream enough via CPU
# oversubscription that the cumulative gain missed its pre-registered
# threshold; see `34UpdatePlan.md`'s M4 and M6 entries for the full
# measurement and the reasoning. This script therefore remains
# structurally serial.
#
# `35UpdatePlan.md` M2-M6 removed a different cost: four of five
# targets were re-invoking their own build tool once per `--scenario`
# even after the program was already built (`cargo run`, `go run`,
# `uv run python`, `./mvnw exec:java`) -- work the build tool had
# already finished. Python (M5) now execs the synced venv's own
# interpreter directly; Java (M6) now assembles a classpath once and
# execs a bare `java -cp`, propagating `sim_drive_java_multi`'s own
# already-correct approach into the `_single` driver. Rust and Go's
# levers were measured and cut at M1 (ideal savings of 1.3s/1.2s
# against the whole corpus -- too small to justify their risk); their
# per-scenario `cargo run`/`go run` calls are unchanged. Measured
# cumulative effect at that arc's own M7 checkpoint: **~18% faster**
# on this script's own full run (see `docs/perf/README.md`), with the
# larger share of it coming from Java, whose `exec:java` bootstrap
# cost scales with a project's dependency graph rather than being a
# flat per-call constant.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

TARGETS="${SIM_CORPUS_TARGETS:-python,rust,typescript,go,java}"
if [[ "${1:-}" == "--targets" ]]; then
    TARGETS="$2"
fi
IFS=',' read -ra TARGET_LIST <<<"$TARGETS"

WORKDIR=$(mktemp -d)
# `34UpdatePlan.md` M3: per-combination results live here as one `.out`
# (captured stdout+stderr) and one `.rc` (exit code) file each, keyed
# by slot id -- the tally below reads these back rather than trusting
# any shell variable a combination's own invocation might have touched,
# which is what makes this design safe once M4 backgrounds combinations
# across process boundaries (a backgrounded subshell's variable writes
# are invisible to the parent; a file it wrote is not).
RESULTS="$WORKDIR/results"
mkdir -p "$RESULTS"
trap 'rm -rf "$WORKDIR"' EXIT

cargo build -p ciac >/dev/null
CIAC="./target/debug/ciac"

# (program, scenario...) pairs -- every corpus scenario paired with
# the single example program it drives, per the scenario files' own
# "service" fields (confirmed by direct inspection, not guessed).
# `query-verbs.ciac`/`order-system.ciac` (the flagship refusal case
# every M4-M8 Shipped note named, now closed at M9) joined the corpus
# at M9 alongside Python's own `db.update`/predicate-query closure --
# neither example was reachable by any target's simulator before this
# milestone. `sim-three-service.ciac`/`multi-service-media.ciac`/
# `inventory-system.ciac` (28UpdatePlan.md) joined the corpus once
# every target's own multi-service driver split (`_single`/`_multi`)
# landed at M3 (Python)/M6 (Rust)/M7 (TS, Go)/M8 (Java) -- each is a
# multi-project `--out` directory, exercising the shared-world call
# router and per-service table namespacing this arc's own M2 built,
# not just single-service depth the way every other row here does.
declare -A PROGRAM_SCENARIOS=(
    [examples/sim-peripherals.ciac]="sim/cache-ttl.ciac-sim.json sim/auth-scopes.ciac-sim.json sim/http-fixtures.ciac-sim.json sim/peripherals.ciac-sim.json"
    [examples/sim-vertical-slice.ciac]="sim/vertical-slice.ciac-sim.json sim/virtual-week.ciac-sim.json"
    [examples/sim-broker-slice.ciac]="sim/fanout.ciac-sim.json"
    [examples/domain-orders.ciac]="sim/relational-depth.ciac-sim.json sim/atomic-batch.ciac-sim.json"
    [examples/query-verbs.ciac]="sim/query-verbs.ciac-sim.json"
    [examples/order-system.ciac]="sim/order-system.ciac-sim.json"
    [examples/sim-three-service.ciac]="sim/sim-three-service.ciac-sim.json"
    [examples/multi-service-media.ciac]="sim/multi-service-media.ciac-sim.json"
    [examples/inventory-system.ciac]="sim/inventory-system.ciac-sim.json"
    [examples/quickstart.ciac]="sim/quickstart.ciac-sim.json"
)

# `34UpdatePlan.md` M3: a filename-safe id for one (program, target)
# combination's result files -- a program's basename (kept *with* its
# `.ciac` extension, matching the pre-M3 report's own `$(basename
# "$program")` column exactly) never contains `__`, so this is
# unambiguous to split back apart in the tally below.
slot_id() {
    printf '%s__%s' "$(basename "$1")" "$2"
}

# `34UpdatePlan.md` M3 (FM1): writes this combination's result to
# files instead of touching any counter -- called synchronously here
# (M3 introduces no concurrency; M4 is the only milestone that adds
# `&` around this same, already-verified function). `rm -rf "$out"`
# before and after are unchanged from the pre-M3 script.
run_combination() {
    local program=$1 target=$2
    local slot out
    slot=$(slot_id "$program" "$target")
    out="$WORKDIR/$(basename "$program" .ciac)-$target"
    # shellcheck disable=SC2086
    local scenarios=(${PROGRAM_SCENARIOS[$program]})
    local scenario_args=()
    for s in "${scenarios[@]}"; do
        scenario_args+=(--scenario "$s")
    done
    rm -rf "$out"
    local rc=0 result
    result=$("$CIAC" sim --target "$target" --out "$out" "${scenario_args[@]}" "$program" 2>&1) || rc=$?
    printf '%s\n' "$result" >"$RESULTS/$slot.out"
    printf '%d\n' "$rc" >"$RESULTS/$slot.rc"
    rm -rf "$out"
}

ALL_SLOTS=()
for program in "${!PROGRAM_SCENARIOS[@]}"; do
    for target in "${TARGET_LIST[@]}"; do
        ALL_SLOTS+=("$(slot_id "$program" "$target")")
        run_combination "$program" "$target"
    done
done

# --- tally: parent-side only, read back from files (FM1, FM9, FM10) --
failed_combinations=0
failed_scenarios=0
total_combinations=${#ALL_SLOTS[@]}
missing=0

printf '%-32s %-12s %-40s %s\n' "PROGRAM" "TARGET" "SCENARIO" "RESULT"
for slot in "${ALL_SLOTS[@]}"; do
    program="${slot%%__*}"
    target="${slot##*__}"
    # `34UpdatePlan.md` M3 (FM2, structural half): a slot with no `.rc`
    # file means its combination never finished -- a failure, not an
    # absence, though this branch cannot fire yet with everything run
    # synchronously; kept now so M4 adds no new logic here, only `&`.
    if [[ ! -f "$RESULTS/$slot.rc" ]]; then
        printf 'missing result for %s\n' "$slot" >&2
        missing=$((missing + 1))
        continue
    fi
    rc=$(<"$RESULTS/$slot.rc")
    result=$(<"$RESULTS/$slot.out")
    if [[ "$rc" -ne 0 ]]; then
        failed_combinations=$((failed_combinations + 1))
    fi
    while IFS= read -r line; do
        if [[ "$line" == \[PASS\]* || "$line" == \[FAIL\]* ]]; then
            name="${line#* }"
            mark="${line%% *}"
            printf '%-32s %-12s %-40s %s\n' "$program" "$target" "$name" "$mark"
            # `34UpdatePlan.md` M3 (FM10): explicit `if`, not the
            # `&&`-list that depended on `set -e`'s AND-list exemption.
            if [[ "$mark" == "[FAIL]" ]]; then
                failed_scenarios=$((failed_scenarios + 1))
            fi
        fi
    done <<<"$result"
    if [[ "$rc" -ne 0 ]]; then
        printf '%-32s %-12s %-40s %s\n' "$program" "$target" "(build/refusal)" "ERROR: $(echo "$result" | tail -1)"
    fi
done

echo
# `34UpdatePlan.md` M3 (FM9): two counters, two units, reported
# separately -- the pre-M3 script summed a combination-level count and
# a scenario-level count into one `$FAILED` and reported it against
# `$TOTAL` combinations, which could exceed the number of combinations
# that exist.
if [[ "$failed_combinations" -eq 0 && "$failed_scenarios" -eq 0 && "$missing" -eq 0 ]]; then
    echo "sim-corpus-x${#TARGET_LIST[@]}: all combinations green ($total_combinations program×target runs)."
else
    echo "sim-corpus-x${#TARGET_LIST[@]}: $failed_combinations failing combination(s), $failed_scenarios failing scenario(s), $missing missing result(s), out of $total_combinations combinations." >&2
    exit 1
fi
