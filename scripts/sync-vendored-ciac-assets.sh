#!/usr/bin/env bash
# Re-syncs the ciac/ciac-syntax crates' vendored copies from their real
# sources. `ciac` embeds sim/pyrunner/*.py (its Python simulation
# runner) and examples/*.ciac (ciac new's scaffold templates); ciac and
# ciac-syntax both embed the repo-root LANGUAGE_VERSION file. All of
# these have to live inside their own crate's directory for `cargo
# package`/`publish` to bundle them -- they can't stay `include_str!`ed
# straight from the repo root or a sibling directory. Run this after
# any change to sim/pyrunner/*.py, examples/*.ciac, or LANGUAGE_VERSION,
# then run `cargo test -p ciac -p ciac-syntax` to confirm the drift-
# guard tests (vendored_pyrunner_matches_source,
# vendored_examples_match_source, vendored_language_version_matches_
# source) pass.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

for name in world cron scenario_runner replay auto_driver multi_service multi_driver; do
    mkdir -p crates/ciac/vendor/pyrunner
    cp "sim/pyrunner/$name.py" "crates/ciac/vendor/pyrunner/$name.py"
    echo "synced crates/ciac/vendor/pyrunner/$name.py"
done

for name in crud-notes inventory-system kafka-pipeline ping; do
    mkdir -p crates/ciac/vendor/examples
    cp "examples/$name.ciac" "crates/ciac/vendor/examples/$name.ciac"
    echo "synced crates/ciac/vendor/examples/$name.ciac"
done

mkdir -p crates/ciac/vendor crates/ciac-syntax/vendor
cp LANGUAGE_VERSION crates/ciac/vendor/LANGUAGE_VERSION
echo "synced crates/ciac/vendor/LANGUAGE_VERSION"
cp LANGUAGE_VERSION crates/ciac-syntax/vendor/LANGUAGE_VERSION
echo "synced crates/ciac-syntax/vendor/LANGUAGE_VERSION"
