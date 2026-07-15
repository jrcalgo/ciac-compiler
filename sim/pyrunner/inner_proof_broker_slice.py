"""Runs inside the generated `sim-broker-slice` project's own `uv`
venv. v0.17 M7's live proof: real generated Postgres/NATS/Redis-target
Python code, driven through `world.SimWorld`'s ordered `FakeBroker` and
virtual-clock-backed `FakeCache` -- ordering, independent queue groups
(fan-out), duplicate delivery via a lost ack after effects committed,
and cache TTL, none of it wall-clock-dependent.
"""

from __future__ import annotations

import asyncio
import json

from world import SimWorld

from app.state import AppState, set_current
from app.db import get_sessionmaker
from app.cache import get_cache
from app.schemas import Ping
from app.models import WidgetIn
from app.api.emit_api import emit_api
from app.api.widget import _store
from app.workers.consumer_a import handle_message as handle_message_a
from app.workers.consumer_b import handle_message as handle_message_b


async def main() -> None:
    world = SimWorld()
    set_current(AppState.simulation(world))

    results: dict[str, object] = {}

    # -- Ordering + independent queue groups (fan-out) --
    for seq in range(3):
        await emit_api(Ping(id=f"ping-{seq}", seq=seq))

    a_invocations = await world.deliver(
        "sim_broker_slice.pings", "consumer_a", lambda p: handle_message_a(Ping(**p))
    )
    b_invocations = await world.deliver(
        "sim_broker_slice.pings", "consumer_b", lambda p: handle_message_b(Ping(**p))
    )
    results["consumer_a_invocations"] = a_invocations
    results["consumer_b_invocations"] = b_invocations
    results["consumer_a_processed_count"] = world.db.count("processed_by_a_table")
    results["consumer_b_processed_count"] = world.db.count("processed_by_b_table")

    a_rows = list(world.db.snapshot().get("processed_by_a_table", {}).values())
    a_seqs = [row["seq"] for row in sorted(a_rows, key=lambda r: r["seq"])]
    results["consumer_a_saw_all_three_in_order"] = [
        row["seq"] for row in a_rows
    ] == sorted(a_seqs) == [0, 1, 2]
    b_rows = list(world.db.snapshot().get("processed_by_b_table", {}).values())
    results["consumer_b_saw_all_three_independent_of_a"] = len(b_rows) == 3

    # -- Duplicate delivery via a lost ack after effects committed --
    world2 = SimWorld(failure_rules=[("broker.ack", "sim_broker_slice.pings", 1)])
    set_current(AppState.simulation(world2))
    await emit_api(Ping(id="ping-dup", seq=99))
    invocations = await world2.deliver(
        "sim_broker_slice.pings", "consumer_a", lambda p: handle_message_a(Ping(**p))
    )
    results["lost_ack_caused_two_invocations"] = invocations == 2
    # HandleA has no idempotency key check, so the redelivered second
    # invocation tries to insert a second row with a *new* random
    # ProcessedByA.id -- it succeeds again, silently producing a
    # duplicate processed-row for one real ping. This is the exact
    # "duplicate-after-commit" hazard Pillar 5 says finding is the
    # point of a lost-ack test, not a bug in the fake.
    results["lost_ack_produced_a_duplicate_row"] = world2.db.count("processed_by_a_table") == 2

    # -- Cache TTL against virtual time, no wall-clock sleep --
    world3 = SimWorld()
    set_current(AppState.simulation(world3))
    async with get_sessionmaker()() as session:
        store = _store(session=session)
        created = await store.create(WidgetIn(name="Left-handed screwdriver"))
    results["widget_created"] = world3.db.count("widgets") == 1

    async with get_sessionmaker()() as session:
        store = _store(session=session)
        first_get = await store.get(created.id)
    results["cache_miss_on_first_get_still_returns_row"] = first_get is not None

    cache = get_cache()
    cache_key = store._key(created.id)  # the store's own key convention, not assumed
    results["cache_populated_after_first_get"] = await cache.get(cache_key) is not None

    world3.clock.advance_by(10_000)  # 10s: well inside the 30s TTL
    results["cache_still_hits_before_ttl"] = await cache.get(cache_key) is not None

    world3.clock.advance_by(25_000)  # 35s total: past the 30s TTL
    results["cache_expired_after_ttl_via_virtual_time_only"] = (
        await cache.get(cache_key) is None
    )
    async with get_sessionmaker()() as session:
        store = _store(session=session)
        after_expiry = await store.get(created.id)
    results["row_still_readable_after_cache_expiry"] = (
        after_expiry is not None and after_expiry.id == created.id
    )

    results["all_pass"] = all(v is True for v in results.values() if isinstance(v, bool))
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    asyncio.run(main())
