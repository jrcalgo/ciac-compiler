#!/usr/bin/env bash
# Re-syncs crates/ciac-backend-rust/vendor/ciac-sim/ from ciac-sim's own
# source. The Rust backend vendors these five files byte-for-byte into
# every generated project (see the VENDORED_SIM_* doc comment in
# crates/ciac-backend-rust/src/lib.rs for why); the vendored copy has
# to live inside ciac-backend-rust's own crate directory for
# `cargo package`/`publish` to bundle it, so it can't just be an
# `include_str!` reaching into the sibling ciac-sim crate. Run this
# after any change to ciac-sim/src/{clock,cron,failure,scenario,
# world}.rs, then run `cargo test -p ciac-backend-rust` to confirm the
# vendored_sim_matches_source drift-guard test passes.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

SRC="crates/ciac-sim/src"
DST="crates/ciac-backend-rust/vendor/ciac-sim"

mkdir -p "$DST"
for name in clock cron failure scenario world; do
    cp "$SRC/$name.rs" "$DST/$name.rs"
    echo "synced $DST/$name.rs"
done
