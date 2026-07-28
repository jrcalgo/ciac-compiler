#!/usr/bin/env bash
# 29UpdatePlan.md M4: the guide-veracity harness. Extracts every
# annotated block from README.md and docs/guide/*.md and executes it
# for real against the real binary, in document order, in a clean
# workspace per document -- so a runnable-looking command block can
# never silently drift from a command that actually works.
#
# Annotation format (an HTML comment pair around a fenced code
# block -- invisible in rendered Markdown, trivially greppable):
#
#   <!-- ciac-verify:file id=NAME path=REL/PATH -->
#   ```text
#   ...file content, written to REL/PATH in the workspace...
#   ```
#   <!-- ciac-verify:end -->
#
#   <!-- ciac-verify:start id=NAME -->
#   ```sh
#   ...shell commands, run with `set -e` in the workspace...
#   ```
#   <!-- ciac-verify:end -->
#
#   <!-- ciac-verify:skip id=NAME reason="..." -->
#   ```sh
#   ...not executed; counted and reported by name+reason instead,
#      same honesty the Docker-delegation split already applies to
#      `ciac verify --system`...
#   ```
#   <!-- ciac-verify:end -->
#
# Deliberately dumb (per the plan): no output assertions beyond exit
# codes. The checkpoints inside each guide (`ciac check`/`build`/
# `verify`/`sim`) are themselves the real assertions; this harness's
# only job is proving the reader could have typed these blocks
# verbatim and had them work.
#
# Usage: scripts/check-guides.sh [FILE...]
#   (defaults to README.md + docs/guide/*.md)
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

if [[ $# -gt 0 ]]; then
    DOCS=("$@")
else
    DOCS=(README.md)
    for g in docs/guide/*.md; do
        [[ -e "$g" ]] && DOCS+=("$g")
    done
fi

cargo build -q -p ciac
CIAC_BIN="$(cd target/debug && pwd)"
export PATH="$CIAC_BIN:$PATH"
REPO_ROOT="$(pwd)"

TOTAL_RUN=0
TOTAL_SKIP=0
TOTAL_FAIL=0
SKIP_REPORT=()

check_one_doc() {
    local doc="$1"
    local workdir
    workdir=$(mktemp -d)
    # A checked-out repo's own `examples/`/`sim/` directories, so a
    # command block that says `examples/quickstart.ciac` resolves
    # exactly as it would for a reader who cloned this repository.
    ln -s "$REPO_ROOT/examples" "$workdir/examples"
    ln -s "$REPO_ROOT/sim" "$workdir/sim"

    local kind="" id="" path="" reason=""
    local -a content=()
    local in_block=0 in_fence=0
    local doc_failed=0

    while IFS= read -r line; do
        if [[ $in_block -eq 0 ]]; then
            if [[ "$line" =~ \<\!--\ ciac-verify:(file|start|skip)\ id=([A-Za-z0-9_-]+)(\ path=([^\ ]+))?(\ reason=\"([^\"]*)\")?\ --\> ]]; then
                kind="${BASH_REMATCH[1]}"
                id="${BASH_REMATCH[2]}"
                path="${BASH_REMATCH[4]}"
                reason="${BASH_REMATCH[6]}"
                content=()
                in_block=1
                in_fence=0
            fi
            continue
        fi
        if [[ "$line" == '<!-- ciac-verify:end -->' ]]; then
            case "$kind" in
                file)
                    mkdir -p "$workdir/$(dirname "$path")"
                    printf '%s\n' "${content[@]}" >"$workdir/$path"
                    ;;
                skip)
                    TOTAL_SKIP=$((TOTAL_SKIP + 1))
                    SKIP_REPORT+=("$doc: $id ($reason)")
                    ;;
                start)
                    TOTAL_RUN=$((TOTAL_RUN + 1))
                    local script
                    script=$(printf '%s\n' "${content[@]}")
                    if ! (cd "$workdir" && set -e && eval "$script") >"$workdir/.out-$id.log" 2>&1; then
                        TOTAL_FAIL=$((TOTAL_FAIL + 1))
                        doc_failed=1
                        echo "[FAIL] $doc: $id"
                        sed 's/^/    /' "$workdir/.out-$id.log"
                    else
                        echo "[PASS] $doc: $id"
                    fi
                    ;;
            esac
            in_block=0
            continue
        fi
        if [[ $in_fence -eq 0 ]]; then
            [[ "$line" == '```'* ]] && in_fence=1
            continue
        fi
        if [[ "$line" == '```' ]]; then
            in_fence=0
            continue
        fi
        content+=("$line")
    done <"$doc"

    rm -rf "$workdir"
    return $doc_failed
}

OVERALL_FAIL=0
for doc in "${DOCS[@]}"; do
    echo "=== $doc ==="
    if ! check_one_doc "$doc"; then
        OVERALL_FAIL=1
    fi
done

echo
echo "check-guides: $TOTAL_RUN block(s) run, $TOTAL_SKIP skipped, $TOTAL_FAIL failed"
if [[ ${#SKIP_REPORT[@]} -gt 0 ]]; then
    echo "skipped (disclosed, not silently ignored):"
    for s in "${SKIP_REPORT[@]}"; do
        echo "  - $s"
    done
fi

if [[ $OVERALL_FAIL -ne 0 || $TOTAL_FAIL -ne 0 ]]; then
    exit 1
fi
