#!/usr/bin/env bash
# 26UpdatePlan.md M6: every deny.toml advisory ignore must carry a
# `reason` and a trailing `# expires: YYYY-MM-DD` comment, and the
# date must not have passed. An ignore without both is a CI failure.
set -euo pipefail

deny_toml="${1:-deny.toml}"
today=$(date -u +%Y-%m-%d)
fail=0

in_ignore=0
while IFS= read -r line; do
    trimmed=$(echo "$line" | sed -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//')

    if [[ "$trimmed" =~ ^ignore[[:space:]]*=[[:space:]]*\[ ]]; then
        in_ignore=1
        [[ "$trimmed" == *"]"* ]] && in_ignore=0
        continue
    fi
    (( in_ignore == 0 )) && continue
    if [[ "$trimmed" == "]" || "$trimmed" == "]," ]]; then
        in_ignore=0
        continue
    fi
    [[ -z "$trimmed" || "$trimmed" == \#* ]] && continue

    if [[ ! "$trimmed" =~ reason[[:space:]]*= ]]; then
        echo "FAIL: ignore entry missing reason: $trimmed" >&2
        fail=1
        continue
    fi
    if [[ ! "$trimmed" =~ \#[[:space:]]*expires:[[:space:]]*([0-9]{4}-[0-9]{2}-[0-9]{2}) ]]; then
        echo "FAIL: ignore entry missing '# expires: YYYY-MM-DD': $trimmed" >&2
        fail=1
        continue
    fi

    expiry="${BASH_REMATCH[1]}"
    if [[ "$expiry" < "$today" ]]; then
        echo "FAIL: ignore entry expired on $expiry (today: $today): $trimmed" >&2
        fail=1
    fi
done < "$deny_toml"

if (( fail != 0 )); then
    echo "check-deny-ignores.sh: one or more ignore entries missing reason/expiry, or expired." >&2
    exit 1
fi

echo "check-deny-ignores.sh: all ignore entries carry reason + unexpired expiry (or list is empty)."
