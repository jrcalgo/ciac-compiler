#!/usr/bin/env bash
# Re-syncs crates/ciac-backend-rust/vendor/ciac-sim/ from ciac-sim's own
# source. The Rust backend vendors these files byte-for-byte into every
# generated project (see the VENDORED_SIM_* doc comment in
# crates/ciac-backend-rust/src/lib.rs for why); the vendored copy has
# to live inside ciac-backend-rust's own crate directory for
# `cargo package`/`publish` to bundle it, so it can't just be an
# `include_str!` reaching into the sibling ciac-sim crate. Run this
# after any change to an already-vendored ciac-sim/src/*.rs file, then
# run `cargo test -p ciac-backend-rust` to confirm the
# vendored_sim_matches_source drift-guard test passes.
#
# The file list to sync is *derived* from whatever already exists
# under vendor/ciac-sim/ (see the loop below), not hardcoded here --
# vendored_sim_matches_source (crates/ciac-backend-rust/src/lib.rs)
# derives its own list the same way, so the two can't independently
# drift out of sync with each other. Vendoring a *new* file for the
# first time still needs one manual step: `cp crates/ciac-sim/src/
# <name>.rs crates/ciac-backend-rust/vendor/ciac-sim/<name>.rs` once,
# plus the matching `include_str!`/const in lib.rs -- after that, this
# script and the test both pick it up automatically.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
shopt -s nullglob

SRC="crates/ciac-sim/src"
DST="crates/ciac-backend-rust/vendor/ciac-sim"

for path in "$DST"/*.rs; do
    name="$(basename "$path")"
    cp "$SRC/$name" "$DST/$name"
    echo "synced $DST/$name"
done
