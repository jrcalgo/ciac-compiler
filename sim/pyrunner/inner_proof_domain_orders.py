"""Runs inside the generated `domain-orders` project's own `uv` venv
(invoked as `uv run python inner_proof_domain_orders.py`). v0.17 M6's
live proof: real generated Postgres-target Python code, driven through
`world.SimWorld`'s schema-aware `FakeDatabase`, exercising exactly what
domain-orders.ciac's own handlers do -- reference existence, `unique`,
and transaction rollback on insert. Cascade/restrict delete is *not*
exercised here since no domain-orders handler ever deletes anything;
see `sim/pyrunner/test_fake_database.py`'s module docstring.
"""

from __future__ import annotations

import asyncio
import json
from pathlib import Path

from world import ReferenceViolation, Schema, SimWorld, UniqueViolation

from app.state import AppState, set_current
from app.db import get_sessionmaker
from app.schemas import Customer, InvalidOrder, LineItem, Order
from app.api.create_customer_route import create_customer_route
from app.api.checkout import checkout
from app.api.items import items


async def call(route_fn, payload):
    async with get_sessionmaker("default")() as session:
        return await route_fn(payload, session)


async def main() -> None:
    plan = json.loads(Path("sim_plan.json").read_text())
    schema = Schema.from_plan_json(plan)
    world = SimWorld(schema=schema)
    set_current(AppState.simulation(world))

    results: dict[str, object] = {}

    customer = Customer(id="cust-1", name="Ada Lovelace", email="ada@example.com")
    await call(create_customer_route, customer)
    results["customer_inserted"] = world.db.count("customers") == 1

    good_order = Order(id="order-1", customer_id="cust-1", total=42.5)
    await call(checkout, good_order)
    results["good_order_committed"] = world.db.count("orders") == 1
    results["order_audit_committed"] = world.db.count("order_audits") == 1

    bad_order = Order(id="order-2", customer_id="cust-1", total=-5.0)
    try:
        await call(checkout, bad_order)
        results["negative_total_rejected"] = False
    except InvalidOrder:
        results["negative_total_rejected"] = True
    # The whole `transaction { db.insert(Orders, ..); if ..; db.insert(
    # OrderAudits, ..); }` block must roll back together: order-2 must
    # not exist even though its own Orders insert happened before the
    # `fail`.
    results["rollback_left_no_order_row"] = world.db.get("orders", "order-2") is None
    results["orders_count_unchanged_after_rollback"] = world.db.count("orders") == 1

    orphan_order = Order(id="order-missing-customer", customer_id="no-such-customer", total=1.0)
    try:
        await call(checkout, orphan_order)
        results["reference_violation_on_missing_customer"] = False
    except ReferenceViolation:
        results["reference_violation_on_missing_customer"] = True
    results["orders_count_unchanged_after_reference_violation"] = world.db.count("orders") == 1

    line_item = LineItem(id="li-1", order_id="order-1", sku="WIDGET", quantity=3)
    await call(items, line_item)
    results["line_item_committed"] = world.db.count("line_items") == 1

    second_line_item = LineItem(id="li-2", order_id="order-1", sku="GADGET", quantity=1)
    try:
        await call(items, second_line_item)
        results["unique_violation_on_second_line_item"] = False
    except UniqueViolation:
        results["unique_violation_on_second_line_item"] = True
    results["line_items_count_unchanged_after_unique_violation"] = (
        world.db.count("line_items") == 1
    )

    orphan_line_item = LineItem(id="li-3", order_id="no-such-order", sku="X", quantity=1)
    try:
        await call(items, orphan_line_item)
        results["reference_violation_on_missing_order"] = False
    except ReferenceViolation:
        results["reference_violation_on_missing_order"] = True

    results["all_pass"] = all(v is True for k, v in results.items() if k != "all_pass")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    asyncio.run(main())
