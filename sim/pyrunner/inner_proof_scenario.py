"""Runs inside the generated `sim-vertical-slice` project's own `uv`
venv. v0.17 M9's live proof: the checked-in `sim/vertical-slice.ciac-
sim.json` -- the real file, read from disk and interpreted generically
by `scenario_runner.ScenarioRunner`, not the hand-written step-by-step
translation `inner_proof.py` (M5) used. This is the concrete
"generic JSON-scenario interpreter" every milestone since M5 disclosed
as missing.

`--record <path>` writes a `replay.py`-shaped Replay artifact after a
successful run; `--replay <path>` checks an existing artifact's
`source_hash`/`plan_hash` against the current build (refusing on
mismatch, never guessing) and then re-runs the scenario, comparing the
new transcript to the recorded one -- see `replay.py`'s module
docstring for what "equivalent" means here (effect/subject sequence,
not row-level data; `Uuid.new()` is not yet seeded).
"""

from __future__ import annotations

import argparse
import asyncio
import json
import time
from pathlib import Path

from cron import CronSchedule
from replay import ReplayError, build_replay, check_compatible
from scenario_runner import ApiEntry, JobEntry, ScenarioRunner, WorkerEntry
from world import SimWorld

from app.state import AppState, set_current
from app.db import get_sessionmaker
from app.schemas import Order
from app.api.place_order_api import place_order_api
from app.workers.process_order import MAX_RETRIES as PROCESS_ORDER_MAX_RETRIES
from app.workers.process_order import handle_message_once as process_order_handle_once
from app.workers.reconcile import handle_tick_once


async def call_place_order_api(payload: dict, _principal: dict | None = None) -> dict:
    async with get_sessionmaker("default")() as session:
        return await place_order_api(Order(**payload), session)


async def worker_handle_once(payload: dict) -> None:
    await process_order_handle_once(Order(**payload))


def build_runner(world: SimWorld) -> ScenarioRunner:
    return ScenarioRunner(
        world=world,
        apis={"PlaceOrderApi": ApiEntry(service="SimVerticalSlice", call=call_place_order_api)},
        workers={
            "ProcessOrder": WorkerEntry(
                service="SimVerticalSlice",
                subject="sim_vertical_slice.order_created",
                queue_group="process_order",
                handle_once=worker_handle_once,
                max_retries=PROCESS_ORDER_MAX_RETRIES,
            )
        },
        jobs={
            "Reconcile": JobEntry(
                service="SimVerticalSlice",
                schedule=CronSchedule.parse("0 3 * * *"),
                handle_tick=handle_tick_once,
            )
        },
    )


async def run_once(scenario: dict) -> tuple[SimWorld, ScenarioRunner, str | None, float]:
    # Only the vertical-slice scenario is written to exercise the
    # third-attempt-succeeds retry path; virtual-week is a clean
    # throughput/timing fixture (see 17UpdatePlan.md's M5 disclosure)
    # and must not have an unrelated failure injected into its first
    # processed_orders insert.
    failure_rules = (
        [("db.commit", "processed_orders", 1), ("db.commit", "processed_orders", 2)]
        if scenario["name"] == "v0.17-m5-vertical-slice"
        else []
    )
    world = SimWorld(failure_rules=failure_rules)
    set_current(AppState.simulation(world))
    runner = build_runner(world)

    error = None
    started = time.perf_counter()
    try:
        await runner.run(scenario)
    except Exception as exc:  # noqa: BLE001 -- reported, not swallowed
        error = str(exc)
    elapsed_ms = (time.perf_counter() - started) * 1000
    return world, runner, error, elapsed_ms


async def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("scenario_path", type=Path)
    parser.add_argument("--source-hash")
    parser.add_argument("--plan-hash")
    parser.add_argument("--record", type=Path)
    parser.add_argument("--replay", type=Path)
    args = parser.parse_args()

    scenario = json.loads(args.scenario_path.read_text())

    replay_error = None
    if args.replay is not None:
        recorded = json.loads(args.replay.read_text())
        try:
            check_compatible(recorded, source_hash=args.source_hash, plan_hash=args.plan_hash)
        except ReplayError as exc:
            print(json.dumps({"scenario": scenario["name"], "passed": False, "error": str(exc)}))
            return

    world, runner, error, elapsed_ms = await run_once(scenario)

    if args.replay is not None and error is None:
        if world.transcript != recorded["transcript"]:
            replay_error = "replayed transcript does not match the recorded one"

    if args.record is not None and error is None:
        replay = build_replay(
            source_hash=args.source_hash,
            plan_hash=args.plan_hash,
            scenario=scenario,
            transcript=world.transcript,
        )
        args.record.write_text(json.dumps(replay, indent=2))

    print(
        json.dumps(
            {
                "scenario": scenario["name"],
                "passed": error is None and replay_error is None,
                "error": error or replay_error,
                "elapsed_ms": elapsed_ms,
                "worker_attempts": runner._worker_attempts,
                "job_runs": runner._job_runs,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    asyncio.run(main())
