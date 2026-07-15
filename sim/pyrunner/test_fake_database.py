"""Direct tests of `FakeDatabase`'s v0.17 M6 constraint checking:
reference existence, `unique`, and cascade/restrict delete.

Not run through any generated handler on purpose: `examples/domain-
orders.ciac` (the flagship this milestone targets) never calls a
delete verb from any of its own handlers -- there is no generated code
path to exercise cascade/restrict through. `run_domain_orders.py`
covers what domain-orders' real handlers do exercise (reference
existence and `unique` on insert, transaction rollback); this file
covers the delete-side behavior directly against the fake, using a
hand-built `Schema` shaped exactly like domain-orders.ciac's own
Customer -> Order (restrict) -> LineItem (cascade, unique) chain --
see 17UpdatePlan.md's M6 milestone entry for why this gap is
disclosed rather than silently left untested.

Run: `python3 sim/pyrunner/test_fake_database.py`
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from world import FakeDatabase, ReferenceColumn, ReferenceViolation, Schema, TableSchema, UniqueViolation


def domain_orders_schema() -> Schema:
    return Schema(
        {
            "customers": TableSchema(),
            "orders": TableSchema(
                references=[
                    ReferenceColumn(
                        python_attr="customer_id",
                        target_table_snake="customers",
                        on_delete="Restrict",
                        unique=False,
                    )
                ]
            ),
            "line_items": TableSchema(
                references=[
                    ReferenceColumn(
                        python_attr="order_id",
                        target_table_snake="orders",
                        on_delete="Cascade",
                        unique=True,
                    )
                ]
            ),
        }
    )


def test_reference_violation_on_insert_with_missing_target() -> None:
    db = FakeDatabase(domain_orders_schema())
    try:
        db.insert("orders", "order-1", {"id": "order-1", "customer_id": "missing-customer"})
    except ReferenceViolation:
        pass
    else:
        raise AssertionError("expected ReferenceViolation")
    assert db.count("orders") == 0, "a rejected insert must not partially apply"


def test_insert_succeeds_once_the_referenced_row_exists() -> None:
    db = FakeDatabase(domain_orders_schema())
    db.insert("customers", "cust-1", {"id": "cust-1", "name": "Ada", "email": "ada@example.com"})
    db.insert("orders", "order-1", {"id": "order-1", "customer_id": "cust-1", "total": 10.0})
    assert db.count("orders") == 1


def test_unique_reference_rejects_a_second_line_item_for_the_same_order() -> None:
    db = FakeDatabase(domain_orders_schema())
    db.insert("customers", "cust-1", {"id": "cust-1"})
    db.insert("orders", "order-1", {"id": "order-1", "customer_id": "cust-1"})
    db.insert("line_items", "li-1", {"id": "li-1", "order_id": "order-1", "sku": "A"})
    try:
        db.insert("line_items", "li-2", {"id": "li-2", "order_id": "order-1", "sku": "B"})
    except UniqueViolation:
        pass
    else:
        raise AssertionError("expected UniqueViolation")
    assert db.count("line_items") == 1


def test_restrict_blocks_delete_while_a_reference_exists() -> None:
    db = FakeDatabase(domain_orders_schema())
    db.insert("customers", "cust-1", {"id": "cust-1"})
    db.insert("orders", "order-1", {"id": "order-1", "customer_id": "cust-1"})
    try:
        db.delete("customers", "cust-1")
    except ReferenceViolation:
        pass
    else:
        raise AssertionError("expected ReferenceViolation (restrict)")
    assert db.count("customers") == 1, "a blocked delete must not partially apply"


def test_cascade_deletes_dependents_recursively() -> None:
    db = FakeDatabase(domain_orders_schema())
    db.insert("customers", "cust-1", {"id": "cust-1"})
    db.insert("orders", "order-1", {"id": "order-1", "customer_id": "cust-1"})
    db.insert("line_items", "li-1", {"id": "li-1", "order_id": "order-1", "sku": "A"})
    db.delete("orders", "order-1")
    assert db.count("orders") == 0
    assert db.count("line_items") == 0, "cascade must remove the dependent line item too"
    assert db.count("customers") == 1, "cascade must not reach past its own on_delete edge"


def test_commit_batch_is_all_or_nothing_across_multiple_rows() -> None:
    db = FakeDatabase(domain_orders_schema())
    db.insert("customers", "cust-1", {"id": "cust-1"})
    db.insert("orders", "order-1", {"id": "order-1", "customer_id": "cust-1"})
    # Second row's reference target doesn't exist -- the whole batch,
    # including the otherwise-valid first row, must be rejected.
    try:
        db.commit_batch(
            inserts=[
                ("line_items", "li-1", {"id": "li-1", "order_id": "order-1", "sku": "A"}),
                ("line_items", "li-2", {"id": "li-2", "order_id": "missing-order", "sku": "B"}),
            ],
            deletes=[],
        )
    except ReferenceViolation:
        pass
    else:
        raise AssertionError("expected ReferenceViolation")
    assert db.count("line_items") == 0, "a rejected batch must apply none of its rows"


def main() -> None:
    tests = [v for k, v in globals().items() if k.startswith("test_") and callable(v)]
    for test in tests:
        test()
        print(f"  ok  {test.__name__}")
    print(f"{len(tests)} tests passed")


if __name__ == "__main__":
    main()
