"""v0.17 M9: writes and checks `ciac_sim::replay::Replay`-shaped JSON
artifacts from the Python runner's own scenario transcripts.

`plan_hash`/`source_hash` are never recomputed in Python -- reproducing
`serde_json::to_vec`'s exact byte output to match `SimPlan::plan_hash()`
would be fragile and pointless when `dump_plan --hash` (see
`crates/ciac-sim/examples/dump_plan.rs`) already computes the
canonical value in Rust.

A real, disclosed gap this module does not paper over: `Uuid.new()` in
generated handler bodies lowers to Python's real `uuid.uuid4()` (traced
empirically -- `app/logic/*.py` imports `from uuid import uuid4`
unconditionally), not a seeded entropy stream. `ciac-sim`'s own
`Entropy` (Rust, v0.17 M4) exists for exactly this, but nothing routes
generated Python's ID generation through it yet. Replay-equivalence as
checked here is therefore over the *transcript* -- ordered `(effect,
subject)` entries, which never carry a row's actual generated ID -- not
over row-level data. A scenario whose own `expect` steps asserted a
specific generated ID's value would not be reproducible today; none of
the checked-in scenarios do.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass
from typing import Any

REPLAY_VERSION = 1
PLAN_VERSION = 1
TARGET_ADAPTER = "python-0.17.0"


class ReplayError(Exception):
    pass


def transcript_hash(transcript: list[dict[str, Any]]) -> str:
    canonical = json.dumps(transcript, sort_keys=True, separators=(",", ":"))
    return "sha256:" + hashlib.sha256(canonical.encode()).hexdigest()


def build_replay(
    *,
    source_hash: str,
    plan_hash: str,
    scenario: dict[str, Any],
    transcript: list[dict[str, Any]],
) -> dict[str, Any]:
    return {
        "replay_version": REPLAY_VERSION,
        "plan_version": PLAN_VERSION,
        "target_adapter": TARGET_ADAPTER,
        "source_hash": source_hash,
        "plan_hash": plan_hash,
        "scenario": scenario,
        "seed": 0,  # disclosed placeholder -- see module docstring
        "start_at": scenario.get("start_at", ""),
        "transcript": transcript,
        "transcript_hash": transcript_hash(transcript),
    }


def check_compatible(replay: dict[str, Any], *, source_hash: str, plan_hash: str) -> None:
    """Mirrors `ciac_sim::Replay::is_compatible_with` -- refuses a
    mismatch rather than guessing compatibility."""
    if replay["replay_version"] != REPLAY_VERSION:
        raise ReplayError(
            f"replay_version {replay['replay_version']} is not supported (expected {REPLAY_VERSION})"
        )
    if replay["plan_version"] != PLAN_VERSION:
        raise ReplayError(f"plan_version {replay['plan_version']} does not match {PLAN_VERSION}")
    if replay["source_hash"] != source_hash:
        raise ReplayError(
            f"recorded source_hash {replay['source_hash']} does not match current {source_hash}"
        )
    if replay["plan_hash"] != plan_hash:
        raise ReplayError(
            f"recorded plan_hash {replay['plan_hash']} does not match current {plan_hash}"
        )
