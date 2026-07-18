"""Runs inside the generated `sim-vertical-slice` project's own `uv`
venv (invoked as `uv run python inner_proof.py` from that project's
root). Executes a hand-written translation of
`sim/vertical-slice.ciac-sim.json`'s steps against the real generated
code, using `world.SimWorld` (importable via `PYTHONPATH`, set by
`run_vertical_slice.py`) as the `AppState.simulation()` seam.

See `sim/pyrunner/world.py`'s module docstring for what this checkpoint
does and does not attempt.
"""

from __future__ import annotations

import asyncio
import json
import sys
import time

from world import SimWorld

from app.state import AppState, set_current
from app.db import get_sessionmaker
from app.schemas import Order
from app.api.place_order_api import place_order_api
from app.workers.process_order import handle_message as worker_handle_message
from app.workers.reconcile import handle_tick_once


async def call_place_order_api(order: Order) -> dict:
    """`place_order_api` is a real FastAPI route function; this
    resolves its one dependency (`Depends(get_session)`) by hand
    instead of going through a running FastAPI/ASGI stack, matching
    17UpdatePlan.md's own "internal calls crossed an in-process
    framework request boundary" bar only partially -- HTTP-level
    request validation/routing is not exercised here, only the
    handler chain and its db/queue effects. Exercising the full ASGI
    boundary too is a fidelity gap this checkpoint discloses, not
    silently claims; see the M5 milestone entry in 17UpdatePlan.md."""
    async with get_sessionmaker("default")() as session:
        return await place_order_api(order, session)


ORDER_ID = "11111111-1111-4111-8111-111111111111"


async def run_vertical_slice() -> dict:
    """Executes `sim/vertical-slice.ciac-sim.json`'s steps -- request,
    drain (worker attempts happen inline here since M5's runner drives
    them directly rather than through a real subscription loop),
    advance 24h, drain (the cron job fires once), and returns the
    world's transcript plus the fake db's final contents for the
    caller to assert against.
    """
    world = SimWorld(
        failure_rules=[
            ("db.commit", "processed_orders", 1),
            ("db.commit", "processed_orders", 2),
        ]
    )
    set_current(AppState.simulation(world))

    order = Order(id=ORDER_ID, total=42.5)
    placed = await call_place_order_api(order)

    await worker_handle_message(order)

    await handle_tick_once()

    return {
        "placed_id": placed["data"]["id"],
        "orders": world.db.count("orders"),
        "audit_entries": world.db.count("audit_entries"),
        "processed_orders": world.db.count("processed_orders"),
        "processed_order_row": world.db.get("processed_orders", None)
        or next(iter(world.db.snapshot().get("processed_orders", {}).values()), None),
        "broker_published": [subject for subject, _ in world.broker.published],
        "failure_unmatched": world.failures.unmatched(),
        "transcript": world.transcript,
    }


async def run_virtual_week() -> dict:
    """A repeated-scenario stand-in for the checked-in
    `sim/virtual-week.ciac-sim.json` -- see `world.py`'s module
    docstring for why this isn't yet a generic JSON-scenario
    interpreter. Places 100 orders with no injected failures (each:
    transaction insert, audit insert, publish, one successful worker
    attempt -- 4 effects) and calls the cron job 7 times (one virtual
    week, daily schedule, one audit insert each) for 407 total
    effects -- the same order of magnitude as the plan's own "1,000
    semantic effects" language, not a literal match to it.
    """
    world = SimWorld()
    set_current(AppState.simulation(world))

    started = time.perf_counter()
    for i in range(100):
        order = Order(id=f"{i:08x}-0000-4000-8000-000000000000", total=float(i))
        await call_place_order_api(order)
        await worker_handle_message(order)
    for _ in range(7):
        await handle_tick_once()
    elapsed_ms = (time.perf_counter() - started) * 1000

    return {
        "elapsed_ms": elapsed_ms,
        "orders": world.db.count("orders"),
        "processed_orders": world.db.count("processed_orders"),
        "job_runs_effects": sum(
            1 for e in world.transcript if e.get("subject") == "audit_entries"
        ),
        "total_transcript_effects": len(world.transcript),
    }


async def measure_p95(fn, iterations: int) -> dict:
    samples_ms = []
    last_result = None
    for _ in range(iterations):
        started = time.perf_counter()
        last_result = await fn()
        samples_ms.append((time.perf_counter() - started) * 1000)
    samples_ms.sort()
    p95_index = max(0, int(len(samples_ms) * 0.95) - 1)
    return {
        "iterations": iterations,
        "p95_ms": samples_ms[p95_index],
        "max_ms": samples_ms[-1],
        "min_ms": samples_ms[0],
        "last_result": last_result,
    }


async def replay_equivalence_check(iterations: int = 5) -> bool:
    transcripts = []
    for _ in range(iterations):
        result = await run_vertical_slice()
        transcripts.append(json.dumps(result["transcript"], sort_keys=True))
    return len(set(transcripts)) == 1


async def main() -> None:
    vertical_slice_timing = await measure_p95(run_vertical_slice, iterations=20)
    replay_equivalent = await replay_equivalence_check()
    virtual_week_result = await run_virtual_week()

    report = {
        "vertical_slice": vertical_slice_timing,
        "replay_equivalent": replay_equivalent,
        "virtual_week": virtual_week_result,
    }
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    asyncio.run(main())
