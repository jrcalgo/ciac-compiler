//! 27UpdatePlan.md M3 (Pillar 8): the fidelity ratchet's zero-Docker row
//! for relational semantics. The v0.17 ratchet compared fake vs real
//! for the vertical slice through `verify --system`'s compose
//! containers (Postgres/MySQL, real infrastructure); this crate has no
//! such harness of its own. SQLite is the one relational engine cheap
//! enough to stand up in-process, in-crate, with no Docker and no
//! network -- so this file runs the same script of operations against
//! a real embedded SQLite database and against [`SimWorld`]'s
//! schema-aware `FakeDatabase`, asserting they agree at every step:
//! insert, a dangling-reference rejection, a cascade delete, a unique
//! violation, and an all-or-nothing batch rollback. A disagreement
//! here is a fake bug, not a SQLite quirk -- the schema below is
//! deliberately the plainest shape both engines already understand
//! (`TEXT PRIMARY KEY`, one `REFERENCES ... ON DELETE CASCADE`, one
//! `UNIQUE` column), not an attempt to cover SQL's full surface.

use ciac_sim::world::{BatchOp, SimWorld, WorldRefAction, WorldReference, WorldTable};
use rusqlite::Connection;

/// `customers` (no references) / `orders` (cascade-deleted with its
/// customer) / `profiles` (one `customer_id` marked `unique`, so a
/// second profile for the same customer collides) -- mirrors
/// `world.rs`'s own `customers_and_orders_schema` test helper, extended
/// with `profiles` for the unique-violation row.
fn fake_schema() -> Vec<WorldTable> {
    vec![
        WorldTable {
            name: "customers".into(),
            references: vec![],
        },
        WorldTable {
            name: "orders".into(),
            references: vec![WorldReference {
                field_name: "customer_id".into(),
                target_table: Some("customers".into()),
                on_delete: WorldRefAction::Cascade,
                unique: false,
            }],
        },
        WorldTable {
            name: "profiles".into(),
            references: vec![WorldReference {
                field_name: "customer_id".into(),
                target_table: Some("customers".into()),
                on_delete: WorldRefAction::Cascade,
                unique: true,
            }],
        },
    ]
}

fn sqlite_schema() -> Connection {
    let conn = Connection::open_in_memory().expect("open in-memory sqlite");
    conn.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        CREATE TABLE customers (id TEXT PRIMARY KEY);
        CREATE TABLE orders (
            id TEXT PRIMARY KEY,
            customer_id TEXT NOT NULL REFERENCES customers(id) ON DELETE CASCADE
        );
        CREATE TABLE profiles (
            id TEXT PRIMARY KEY,
            customer_id TEXT NOT NULL UNIQUE REFERENCES customers(id) ON DELETE CASCADE
        );
        ",
    )
    .expect("create schema");
    conn
}

fn sqlite_order_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM orders", [], |row| row.get(0))
        .expect("count orders")
}

#[test]
fn insert_agrees() {
    let sqlite = sqlite_schema();
    let fake = SimWorld::with_schema(Vec::new(), fake_schema());

    sqlite
        .execute("INSERT INTO customers (id) VALUES ('c1')", [])
        .expect("sqlite accepts a customer with no references to check");
    fake.db_insert_checked("customers", serde_json::json!({"id": "c1"}))
        .expect("fake accepts a customer with no references to check");

    let sqlite_result = sqlite.execute(
        "INSERT INTO orders (id, customer_id) VALUES ('o1', 'c1')",
        [],
    );
    let fake_result = fake.db_insert_checked(
        "orders",
        serde_json::json!({"id": "o1", "customer_id": "c1"}),
    );
    assert!(sqlite_result.is_ok(), "sqlite: valid reference accepted");
    assert!(fake_result.is_ok(), "fake: valid reference accepted");
    assert_eq!(sqlite_order_count(&sqlite), 1);
    assert_eq!(fake.db.count("orders"), 1);
}

#[test]
fn dangling_reference_is_rejected_by_both() {
    let sqlite = sqlite_schema();
    let fake = SimWorld::with_schema(Vec::new(), fake_schema());

    let sqlite_result = sqlite.execute(
        "INSERT INTO orders (id, customer_id) VALUES ('o1', 'missing')",
        [],
    );
    let fake_result = fake.db_insert_checked(
        "orders",
        serde_json::json!({"id": "o1", "customer_id": "missing"}),
    );
    assert!(
        sqlite_result.is_err(),
        "sqlite: a real foreign-key constraint rejects the dangling reference"
    );
    assert!(
        fake_result.is_err(),
        "fake: schema-aware validation rejects the dangling reference"
    );
    assert_eq!(sqlite_order_count(&sqlite), 0);
    assert_eq!(fake.db.count("orders"), 0);
}

#[test]
fn cascade_delete_agrees() {
    let sqlite = sqlite_schema();
    let fake = SimWorld::with_schema(Vec::new(), fake_schema());

    sqlite
        .execute("INSERT INTO customers (id) VALUES ('c1')", [])
        .unwrap();
    sqlite
        .execute(
            "INSERT INTO orders (id, customer_id) VALUES ('o1', 'c1')",
            [],
        )
        .unwrap();
    fake.db_insert_checked("customers", serde_json::json!({"id": "c1"}))
        .unwrap();
    fake.db_insert_checked(
        "orders",
        serde_json::json!({"id": "o1", "customer_id": "c1"}),
    )
    .unwrap();

    sqlite
        .execute("DELETE FROM customers WHERE id = 'c1'", [])
        .expect("sqlite: cascade delete of the customer succeeds");
    fake.db_delete_checked("customers", "c1")
        .expect("fake: cascade delete of the customer succeeds");

    assert_eq!(
        sqlite_order_count(&sqlite),
        0,
        "sqlite: ON DELETE CASCADE removed the dependent order"
    );
    assert_eq!(
        fake.db.count("orders"),
        0,
        "fake: WorldRefAction::Cascade removed the dependent order"
    );
}

#[test]
fn unique_violation_is_rejected_by_both() {
    let sqlite = sqlite_schema();
    let fake = SimWorld::with_schema(Vec::new(), fake_schema());

    sqlite
        .execute("INSERT INTO customers (id) VALUES ('c1')", [])
        .unwrap();
    sqlite
        .execute(
            "INSERT INTO profiles (id, customer_id) VALUES ('p1', 'c1')",
            [],
        )
        .unwrap();
    fake.db_insert_checked("customers", serde_json::json!({"id": "c1"}))
        .unwrap();
    fake.db_insert_checked(
        "profiles",
        serde_json::json!({"id": "p1", "customer_id": "c1"}),
    )
    .unwrap();

    let sqlite_result = sqlite.execute(
        "INSERT INTO profiles (id, customer_id) VALUES ('p2', 'c1')",
        [],
    );
    let fake_result = fake.db_insert_checked(
        "profiles",
        serde_json::json!({"id": "p2", "customer_id": "c1"}),
    );
    assert!(
        sqlite_result.is_err(),
        "sqlite: a real UNIQUE constraint rejects the second profile"
    );
    assert!(
        fake_result.is_err(),
        "fake: schema-aware uniqueness rejects the second profile"
    );
}

#[test]
fn batch_rollback_agrees() {
    let sqlite = sqlite_schema();
    let fake = SimWorld::with_schema(Vec::new(), fake_schema());

    // A two-statement transaction whose second insert violates the
    // dangling-reference check -- neither engine may leave the first
    // insert applied once the batch as a whole fails.
    sqlite
        .execute("INSERT INTO customers (id) VALUES ('c1')", [])
        .unwrap();
    fake.db_insert_checked("customers", serde_json::json!({"id": "c1"}))
        .unwrap();

    let sqlite_tx_result: rusqlite::Result<()> = (|| {
        sqlite.execute("BEGIN", [])?;
        sqlite.execute(
            "INSERT INTO orders (id, customer_id) VALUES ('o1', 'c1')",
            [],
        )?;
        sqlite.execute(
            "INSERT INTO orders (id, customer_id) VALUES ('o2', 'missing')",
            [],
        )?;
        sqlite.execute("COMMIT", [])?;
        Ok(())
    })();
    if sqlite_tx_result.is_err() {
        let _ = sqlite.execute("ROLLBACK", []);
    }

    let fake_result = fake.commit_batch_checked(vec![
        BatchOp::Insert {
            table: "orders".into(),
            row: serde_json::json!({"id": "o1", "customer_id": "c1"}),
        },
        BatchOp::Insert {
            table: "orders".into(),
            row: serde_json::json!({"id": "o2", "customer_id": "missing"}),
        },
    ]);

    assert!(
        sqlite_tx_result.is_err(),
        "sqlite: the transaction as a whole fails"
    );
    assert!(fake_result.is_err(), "fake: the batch as a whole fails");
    assert_eq!(
        sqlite_order_count(&sqlite),
        0,
        "sqlite: rollback leaves zero orders, not one partial row"
    );
    assert_eq!(
        fake.db.count("orders"),
        0,
        "fake: the overlay-validated batch leaves zero orders, not one partial row"
    );
}
