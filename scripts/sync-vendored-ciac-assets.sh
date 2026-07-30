#!/usr/bin/env bash
# Re-syncs the ciac/ciac-syntax crates' vendored copies from their real
# sources. `ciac` embeds sim/pyrunner/*.py (its Python simulation
# runner) and examples/*.ciac (ciac new's scaffold templates); ciac and
# ciac-syntax both embed the repo-root LANGUAGE_VERSION file. All of
# these have to live inside their own crate's directory for `cargo
# package`/`publish` to bundle them -- they can't stay `include_str!`ed
# straight from the repo root or a sibling directory. Run this after
# any change to an already-vendored sim/pyrunner/*.py or examples/*.ciac
# file, or to LANGUAGE_VERSION, then run `cargo test -p ciac -p
# ciac-syntax` to confirm the drift-guard tests
# (vendored_pyrunner_matches_source, vendored_examples_match_source,
# vendored_language_version_matches_source) pass.
#
# The pyrunner/examples file lists are *derived* from whatever already
# exists under each vendor/ subdirectory (sync_dir below), not
# hardcoded here -- vendored_pyrunner_matches_source (crates/ciac/src/
# commands.rs) and vendored_examples_match_source (crates/ciac/src/
# scaffold.rs) derive their own lists the same way, so script and test
# can't independently drift out of sync. Vendoring a *new* file for the
# first time still needs one manual step, e.g. `cp sim/pyrunner/
# <name>.py crates/ciac/vendor/pyrunner/<name>.py` once, plus the
# matching `include_str!`/const -- after that, this script and the test
# both pick it up automatically. LANGUAGE_VERSION is a group of one in
# each crate, so it stays a plain, explicit copy below rather than a
# derived loop.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
shopt -s nullglob

sync_dir() {
    local src_dir="$1" dst_dir="$2"
    for path in "$dst_dir"/*; do
        local name
        name="$(basename "$path")"
        cp "$src_dir/$name" "$dst_dir/$name"
        echo "synced $dst_dir/$name"
    done
}

sync_dir "sim/pyrunner" "crates/ciac/vendor/pyrunner"
sync_dir "examples" "crates/ciac/vendor/examples"

cp LANGUAGE_VERSION crates/ciac/vendor/LANGUAGE_VERSION
echo "synced crates/ciac/vendor/LANGUAGE_VERSION"
cp LANGUAGE_VERSION crates/ciac-syntax/vendor/LANGUAGE_VERSION
echo "synced crates/ciac-syntax/vendor/LANGUAGE_VERSION"
