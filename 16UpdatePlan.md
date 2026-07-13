# CIaC v0.16 — The Domain Version: Relations, Constraints, Transactions (roadmap forecast)

> Forecast document. Assumes v0.13 (friction), v0.14 (expressiveness),
> and v0.15 (operations & reach) have landed. Direction-setting; the
> v0.16 planning pass finalizes the reference-field surface syntax and
> the per-engine transaction lowering. **Confidence label: structural.**
> Unlike every bet in v0.17–v0.21, this version is arithmetic, not
> hypothesis: a domain with parent–child data currently has *infinite*
> time-to-completion in ciac, because the IR cannot express a foreign
> key at all.

## The gap this version closes

By v0.15 a team (or one agent) can express, generate, run, verify,
trace, and CI a real multi-service system — as long as its *data* is
flat. Walk the type surface end to end and the ceiling is visible:

1. **No reference type exists anywhere in the compiler.**
   `FieldType` (`crates/ciac-ir/src/record.rs`) is
   `Str | Int | Float | Bool | Uuid | Timestamp | Json | Enum` —
   there is no variant that points at another record. `crud` expands
   to either a keyed-JSONB document store or (with `crud X: Record`)
   typed columns; `table X: Record` (v0.7 M5) gets a real migrated
   row. Neither can say "an `Order` belongs to a `Customer`", "a
   `Customer` has many `Orders`", or "deleting a `Customer` restricts
   while it still has `Orders`". That is not a missing convenience —
   it disqualifies the majority of real backends: e-commerce, CMS,
   ticketing, anything with a parent–child or many-to-many shape.
   The v0.14 flagship `order-system.ciac` fakes its domain with
   unlinked `Uuid` fields precisely because the language cannot do
   better.
2. **No unique constraints, no indexes.** v0.14's `where` predicates
   generate real SQL against columns that can never be indexed, and
   "email must be unique" — the second thing every registration flow
   needs — is inexpressible. Both are single-attribute-sized surface
   with real migration and codegen depth behind them.
3. **No transactions.** Every generated db statement runs in its own
   implicit transaction. A handler body that debits one row and
   credits another (two `db.update` verbs, v0.14 M2) is a
   partial-failure corruption bug that ciac *generates*. Multi-table
   writes without atomicity become actively dangerous the moment
   relations exist, so transactions must land in the same version as
   relations, not after.
4. **No declared field validation.** "status is one of …" exists
   (inline enums); "quantity ≥ 1", "email is an email", "title is
   non-empty" do not. Every one of these is a hand-written check an
   agent must write *and* test in seeded code today, per backend.

**v0.16 theme: the language can finally say what ordinary business
domains are shaped like — and the generated system keeps them
consistent under partial failure.**

## Pillar 1 — Reference fields (`ref`)

### Surface syntax

```ciac
record Customer {
    id: Uuid;
    email: String { unique: true; format: email; }
}

record Order {
    id: Uuid;
    customer: ref Customer { on_delete: restrict; }
    placed_at: Timestamp;
}

record LineItem {
    id: Uuid;
    order: ref Order { on_delete: cascade; }
    sku: String;
    quantity: Int { min: 1; }
}
```

- `ref <Record>` — required to-one reference. `ref? <Record>` —
  optional (nullable FK, composing with v0.14 M1's optional-type
  grammar). `[ref <Record>]` is **rejected with a dedicated
  diagnostic** pointing at the explicit-join-record idiom (see cut
  lines): implicit many-to-many is out of scope for v0.16.
- `on_delete: cascade | restrict | set_null` is a per-field attribute
  block entry, **required** on every `ref` (no silent default — a
  cascade the author never chose is exactly the footgun a
  "shape not vibes" compiler must not ship). `set_null` is only legal
  on `ref?` (new diagnostic otherwise).

### Front end

- `crates/ciac-syntax`: `ref` keyword token; `TypeExpr::Ref { target:
  Ident, optional: bool, span }`; field-level attribute blocks (the
  parser's `decl_tail` machinery already parses `{ attrs }` on
  declarations — fields gain the same tail, reusing `Attr`).
- `crates/ciac-ir/src/record.rs`: `FieldType::Ref { record: RecordId,
  optional: bool, on_delete: OnDelete }`. `RecordId` is already the
  interned identity records resolve through; the new work is a
  **second resolution pass** in `crates/ciac-sema/src/build.rs` —
  records must all be registered before `ref` targets resolve, so ref
  resolution runs after the existing record-registration loop, exactly
  where table resolution already runs ("own namespace, resolved after
  records").
- New sema checks + error codes (allocated append-only at
  implementation time; names indicative):
  - `UnknownRecordReference` — `ref` target is not a declared record;
  - `RefRequiresTypedStorage` — a record containing `ref` fields is
    used by an *untyped* keyed-document `crud X;` (the JSONB store has
    no columns to constrain — `ref` requires `crud X: Record` or
    `table`);
  - `RefCycleWithoutOptional` — a cycle of *required* refs
    (`A ref B`, `B ref A`) is uninsertable and rejected;
    self-reference and cycles through `ref?` are legal (category →
    parent category);
  - `SetNullOnRequiredRef`, `MissingOnDelete` as above;
  - `RefTargetHasNoId` — target record lacks the `id: Uuid` primary
    key convention the stores are built on.
- `blueprints.rs` hygiene: `ref` targets inside a `blueprint` body
  participate in the v0.8 M3 name-suffixing exactly like record
  mentions in field positions do today.

### Codegen

`crates/ciac-codegen/src/model.rs` grows per-field
`FieldCtx { ref_target: Option<RefCtx> }` where `RefCtx` carries the
target's class/table names, the FK column name (`<field>_id` — the
DSL field `customer` maps to column `customer_id`, disclosed in docs),
optionality, and `on_delete`. `RecordCtx` additionally carries
`incoming_refs: Vec<IncomingRefCtx>` (who points at me), because
DELETE codegen and the system-test generator both need the reverse
direction.

- **Python** (`ciac-backend-python`): SQLAlchemy `ForeignKey(...,
  ondelete=...)` on the column; `relationship()` with `back_populates`
  emitted on both sides for typed `crud`/`table` models; pydantic
  schemas carry the FK id field (`customer_id: UUID`) — **responses
  serialize FK ids, never embedded objects** (see cut lines). Store
  templates (`resource_store.py.j2`) gain FK-integrity error mapping:
  a violated FK on insert/update returns 422 with a structured error
  body, a `restrict` violation on delete returns 409.
- **Rust** (`ciac-backend-rust`): no ORM exists and none is adopted.
  Generated SQL gains `REFERENCES <table>(id) ON DELETE <action>` in
  `ensure_schema_*` / migration output; typed row structs carry
  `customer_id: Uuid` / `Option<Uuid>`; the v0.13 M1 placeholder-style
  discipline (Postgres `$N`, MySQL/SQLite `?` with the
  fields-first-id-last bind order) extends unchanged to the new
  columns. sqlx error mapping: FK violation → 422/409 via the database
  error-code branches (per-engine codes: PG `23503`, MySQL `1451/1452`
  — behind the existing engine switch).
- **Migration differ** (`crates/ciac-codegen/src/migrations.rs`):
  learns FK add/drop and `on_delete` changes as first-class diff ops,
  ordered correctly (columns before constraints; constraint drops
  before column drops). **Destructive discipline**: any generated
  migration that drops a column/constraint is emitted but the runner
  refuses it without an explicit `--allow-destructive` (CLI +
  generated runner flag) — the regeneration-sidecar philosophy applied
  to data.
- **Record evolution** (`evolution.rs`, v0.8 M5): changing a field
  to/from `ref`, changing the target, or changing `on_delete` is a
  consumer-visible change → extends the existing CIAC0051 machinery.
- **OpenAPI (v0.15 M1)**: FK fields emit `format: uuid` plus an
  `x-ciac-ref: <Record>` extension; **TS client (v0.15 M2)**:
  `customer_id: string` with a doc comment naming the target
  interface.
- **System tests** (`system_tests.rs`): `sample_json` currently
  synthesizes flat payloads; it becomes topological — the capability
  round-trip for `LineItem` first creates a `Customer`, then an
  `Order`, threading real generated ids. New generated check per
  `restrict` edge: create parent+child, DELETE parent, assert 409,
  DELETE child then parent, assert 204 — a *live* referential-
  integrity proof in `ciac verify --system`. New check per `cascade`
  edge: delete parent, direct-connection SELECT (the v0.9 M2 second
  connection) proves the child row is gone from the database itself.

## Pillar 2 — Unique constraints and indexes

- Field attribute `unique: true` (typed `crud`/`table` storage only —
  same `RefRequiresTypedStorage`-style gate for the document store).
- Top-level/service-level `index <Record>.<field>;` declaration (and
  composite: `index Order(customer, placed_at);`). Sema checks the
  fields exist and storage is typed; duplicate index declarations are
  the existing duplicate-declaration code.
- Migrations emit `CREATE UNIQUE INDEX` / `CREATE INDEX` with stable
  deterministic names (`ix_<table>_<fields>`); differ handles
  add/drop.
- Unique violations at runtime map to 409 with a structured body
  naming the field — both backends, same envelope.
- This pillar is deliberately thin here because it is load-bearing
  *later*: v0.19's "unindexed predicate" architecture lint needs
  indexes to exist as IR facts, and v0.14 `where` predicates finally
  get an answer to "is this query O(n)".

## Pillar 3 — Transactions

- Handler-body statement (v0.7 HIR): `transaction { <stmts> }` —
  every `db.*` verb inside lowers into one unit of work; any `fail`
  or error rolls back. Grammar is a new `Stmt::Transaction(Vec<Stmt>)`
  through `typeck.rs` (a scope frame, like blocks today) into a new
  `HirStmt::Transaction`.
- Sema rules:
  - all `db.*` verbs inside must bind the **same instance** (cross-
    instance transactions are 2PC — rejected with a diagnostic that
    says so);
  - `cache.*`, `http.*`, `email.*` verbs inside a transaction are a
    *warning* (side effects that won't roll back), not an error;
  - `publish` inside a transaction is **rejected** with a forward-
    looking diagnostic: "publish inside a transaction requires the
    transactional outbox (planned v0.19); publish after the
    transaction commits instead" — the dual-write footgun becomes
    inexpressible *now*, and the v0.19 outbox flips this arm from
    reject to lower.
- **Python lowering** (`lower.rs`): `async with
  session.begin(): ...` around the block's statements; the store/
  session plumbing (v0.4 named instances → sessionmaker keys) already
  threads the right session.
- **Rust lowering**: `let mut tx = state.<db_field>.begin().await?;`
  … `tx.commit().await?;` with all inner verbs executing against
  `&mut *tx` instead of the pool. sqlx transactions are typed
  per-engine (`Transaction<'_, Postgres>` vs `MySql` vs `Sqlite`) —
  the v0.13 M1 per-engine generation discipline
  (`ensure_schema_<engine>`) extends: verb lowering already knows
  `table_db_engine`/instance engine, so the emitted code is monomorphic
  per handler, no trait gymnastics.
- Typed `crud` write paths (create/update/delete) become
  transactional by construction where they touch >1 statement.
- **Live proof bar**: a new example with a two-account transfer
  handler; the generated project's behavioral test forces a failure
  after the first write (a `fail` branch) and asserts the first write
  rolled back — run against real Postgres in `ciac verify --system`,
  and against MySQL and SQLite in the per-project suites (the
  bind-order/engine matrix from v0.13 M1 re-proven under
  transactions).

## Pillar 4 — Declared field validation

- Field-attribute surface (same block as `unique`): `min`/`max`
  (Int/Float), `min_len`/`max_len` (String and v0.14 lists),
  `format: email | url` (String). Closed registry, per-type
  applicability checked in sema (a `min` on a `Bool` is the existing
  invalid-attribute code).
- Python: pydantic `Field(ge=, le=, min_length=, max_length=)` +
  `EmailStr`-equivalent validators (implemented as explicit
  validators, not an extra dependency, keeping the no-new-deps bar).
- Rust: generated `validate()` on the request structs, called in the
  extractor path; violations → 422 with per-field messages matching
  the Python shape (cross-target envelope parity is golden-tested).
- OpenAPI emits the matching `minimum`/`maxLength`/`format` keywords;
  the TS client inherits them as doc comments (compile-time TS
  enforcement is out of scope).

## Secondary items

- `ciac describe` / `vocab.rs`: `ref`, field-attribute vocabulary,
  `index` declaration kind — hover/completion pick them up through the
  shared tables as usual.
- Structured fixes (v0.15 M7) extensions where mechanical:
  `MissingOnDelete` offers three titled fixes (one per action);
  `UnknownRecordReference` reuses the Levenshtein nearest-record
  rename fix.
- New flagship: `examples/commerce.ciac` — Customer/Order/LineItem
  with `restrict`+`cascade`, a unique email, an indexed query, and a
  transactional checkout handler; becomes the golden/system-test
  showcase and replaces the flat fake in docs. `order-system.ciac` is
  remodeled onto real refs (a deliberate breaking-example change,
  golden-reviewed).
- Docs: `docs/language.md` relations section, `docs/expressions.md`
  transaction section, `docs/regeneration.md` destructive-migration
  discipline.

## Milestones

1. **M1 — syntax + IR + sema for `ref` and field attributes**: lexer/
   AST/parser, `FieldType::Ref`, resolution pass, the full new
   diagnostic set with negative fixtures (`tests/ui/`), blueprint
   hygiene test. No codegen yet — gated per backend via the existing
   `Backend::supports` / CIAC0011 path so `check` works before `build`
   does.
2. **M2 — relational codegen, Python**: model/schema/store templates,
   FK error mapping, migration differ FK ops, gate removal; golden
   trees; live `verify` against Postgres including the
   restrict-409/cascade-gone proofs.
3. **M3 — relational codegen, Rust**: schema/struct/store SQL with
   per-engine FK error codes; same live bar (Postgres locally;
   MySQL/SQLite through the same per-engine matrix v0.13 M1
   established).
4. **M4 — unique + index**: both backends, differ ops, deterministic
   index naming, 409 mapping, `x-ciac` OpenAPI keywords.
5. **M5 — transactions**: HIR/typeck/sema rules (including the
   publish-in-tx rejection), both lowerings, the transfer example
   with the forced-rollback behavioral test.
6. **M6 — validation attributes**: both backends, OpenAPI keywords,
   envelope-parity golden test.
7. **M7 — flagship, docs, 0.16.0**: `commerce.ciac`, order-system
   remodel, topological `sample_json`, docs, version bump, full
   workspace verification, arc notes. Per-milestone discipline
   throughout: fmt/clippy `-D warnings`/full test suite/`insta`
   review, live proof or explicitly disclosed CI delegation, commit +
   push.

## Risks

- **sqlx per-engine transaction types** could tempt a generic
  abstraction; the mitigation is the decision already made in v0.13
  M1 — generate monomorphic per-engine code, never abstract over
  engines inside one handler.
- **Migration ordering bugs** (constraint before column, etc.) are
  the classic differ failure. Mitigation: the differ's op-ordering is
  property-tested — for every fixture pair (old, new), applying the
  emitted migration to a real database (SQLite in-process, Postgres in
  the system job) must yield a schema that a fresh generate declares
  identical.
- **Destructive migrations** silently dropping data. Mitigation:
  `--allow-destructive` gate, sidecar-style refusal by default,
  loudly documented.
- **Scope creep toward an ORM** on the Rust side. Mitigation: the cut
  lines below are the milestone acceptance criteria; embedded/eager
  loading is explicitly out.
- **`order-system.ciac` remodel churns many goldens at once.**
  Mitigation: land it as the last milestone in isolation, so the diff
  is reviewable as one deliberate change.

## Cut lines (explicitly out of scope for v0.16)

- Implicit many-to-many (`[ref X]`): rejected with a diagnostic
  showing the explicit join-record idiom. Revisit only with usage
  evidence.
- Embedded/nested response serialization (`?embed=customer`) and any
  eager/lazy loading story: responses carry FK ids, clients make a
  second call. This is the single most important line against
  ORM-ification.
- Cross-instance / distributed transactions, savepoints, isolation-
  level surface: one instance, default isolation, whole-block
  atomicity only.
- Check constraints beyond the closed validation-attribute registry;
  computed/derived fields; database-level defaults beyond what
  evolution already requires.
- Data *backfills* for newly-required columns: v0.16 requires new
  non-optional fields on existing tables to declare a `default`; the
  richer backfill story is v0.18's.

## After v0.16

The language can express ordinary business domains and the generated
systems keep them consistent. That immediately raises the price of the
verification loop — bigger domains mean more tests, and every test
still needs Docker. v0.17 (the Simulation version) attacks exactly
that, and depends on this version: its in-process database fake must
honor these relational semantics to be worth trusting.
