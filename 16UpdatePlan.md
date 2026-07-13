# CIaC v0.16 — Domain Semantics: Relations, Constraints, Transactions, and Validation (roadmap forecast)

> Forecast document. Assumes the shipped v0.15 surfaces described by the
> live code and documentation: Python and Rust are the bundled targets,
> OpenAPI and the TypeScript client are generated from the shared codegen
> model, table evolution is additive-only, and handler bodies are a closed
> effect language.
>
> This version supersedes v0.15's tentative suggestion that a full
> TypeScript backend might be the v0.16 headline. That suggestion was
> explicitly left for re-evaluation rather than committed. The TypeScript
> client remains supported and grows with the domain model; a third runtime
> backend does not enter this version.
>
> Direction-setting. The implementation planning pass freezes the relation
> wire shape, persistence ownership rules, validation profile, and
> transaction semantics before codegen begins. This is domain depth, not
> deployment maturity: online migrations, HA databases, rollout
> orchestration, and cloud provisioning remain outside the version.

## The gap this version closes

CIaC v0.15 can describe a multi-service topology, type-check a useful
closed set of handler effects, generate both bundled targets, and expose
the result through OpenAPI and a TypeScript client. It still cannot state
four ordinary facts found in almost every non-trivial domain:

1. **One record refers to another.** Authors represent a customer
   relationship as `customer_id: Uuid`; the compiler cannot know what it
   points at, whether it is one or many, or what deletion is supposed to
   do.
2. **A stored value is unique or intentionally indexed.** Outside the
   conventional primary key, database constraints are handwritten and
   therefore absent from the source model, migration differ, simulator,
   and generated documentation.
3. **Several database effects are one operation.** Python's generated
   typed handlers commit mutating verbs individually; Rust executes them
   through a pool and therefore autocommits each statement. A second
   failure can leave the first write visible.
4. **A field is valid for domain reasons, not merely because it has the
   right primitive type.** `String` cannot say non-empty, `Int` cannot say
   non-negative, and neither backend can derive those rules from `.ciac`.

These are not convenience features. A parent-child domain currently
falls through the language into seeded target code, handwritten SQL,
handwritten validation, and handwritten tests. At that point the compiler
no longer owns the meaning it claims to compile.

**v0.16 theme: make ordinary domain invariants compiler-owned. A
relationship, constraint, transaction boundary, or field rule stated in
`.ciac` must survive through semantic analysis, migrations, both runtimes,
OpenAPI, the TypeScript client, evolution checks, and verification.**

## Where v0.15 actually leaves the implementation

The plan starts from the live repository rather than older forecast
language:

- `ciac_ir::FieldType` is scalar-or-enum only. Nested records are not
  legal record fields; `[Type]` is accepted for handler values but
  rejected in stored records by `CIAC0053`.
- `ciac_codegen::model::FieldTypeKind` mirrors that closed set.
  `openapi.rs` and `ts_client.rs` therefore have no recursive type arm.
- A syntax `Field` has a name, type, and span, but no attributes.
- `table Name: Record;` and `crud Name: Record;` are separate storage
  systems. Explicit tables feed typed `db.*` verbs and incremental
  migrations; CRUD storage is created at startup and is not addressable
  by those verbs.
- `migrations.rs` snapshots tables as column names plus rendered SQL
  types. `evolution.rs` snapshots shallow records that cross service
  boundaries. Neither stores foreign keys, indexes, uniqueness, nested
  types, or validation rules.
- Python typed-handler mutation lowering emits `session.commit()` per
  verb. Python CRUD stores also commit per method.
- Rust typed-handler and CRUD SQL executes against a pool, giving each
  statement an implicit transaction.
- Pydantic performs typed request parsing on Python. Rust serde only
  deserializes; it is not a validation framework. Range/format rules are
  therefore not “free” on Rust and require generated validation calls at
  every typed boundary.
- Source spans exist on several graph objects but are omitted from
  serialized IR, and typed HIR statements do not retain spans. v0.16 does
  not solve full provenance, but new constructs must preserve enough
  location data for precise diagnostics.
- The external-backend protocol is versioned and exposes
  `FieldTypeKind`; extending that enum is a wire-contract change, not an
  internal refactor.

## Design stance

The following rules contain the scope.

1. **Relations are explicit.** Cardinality, target storage, and
   referential actions are written, not inferred from a field suffix or
   pluralized name.
2. **No destructive default.** Both update and delete actions are
   required on a reference. The compiler never guesses cascade.
3. **The wire model and row model are distinct.** A relation appears as
   a nested typed value at API/handler boundaries while storage uses a
   foreign-key column or compiler-owned link table.
4. **The declaring field owns the edge.** v0.16 generates no inverse
   relation, lazy back-reference, or orphan-removal rule.
5. **Transactions are lexical and local.** Only database effects inside
   `transaction { ... }` share one database transaction.
6. **Non-database effects do not pretend to roll back.** `publish`,
   cache, HTTP, email, search, and object-store effects are rejected in a
   v0.16 transaction. v0.19 may deliberately reinterpret transactional
   `publish` as an outbox insert after the failure semantics exist.
7. **Validation is a portable closed profile.** No arbitrary regex,
   target-language callback, transform, or cross-field expression enters
   the language.
8. **Evolution refuses what needs production data knowledge.** The
   compiler can add a safe structure; it does not invent backfills,
   remove duplicates, or rewrite tables behind the user's back.
9. **Python and Rust land together.** A language construct is not
   released while one bundled backend gates it.
10. **Provider-specific behavior is explicit.** Shared semantics are
    target-neutral, but migration capabilities may differ between
    Postgres, MySQL, and SQLite and are refused where unsafe.

## Surface syntax

### Field attributes and references

A field gains the declaration-tail shape already used by APIs and other
declarations:

```ebnf
field          = IDENT ":" type field-tail ;
field-tail     = ";" | "{" { field-attr } "}" ;
field-attr     = IDENT ":" attr-value ";" ;

type           = primitive
               | "enum" "{" IDENT { "," IDENT } "}"
               | "[" type "]"
               | "Reference" "<" IDENT ">" ;

table-decl     = "table" IDENT ":" IDENT table-tail ;
table-tail     = ";" | "{" [ "db" ":" IDENT ";" ] "}" ;
```

`Reference<T>` names the nested target record. A stored reference also
names the explicit table that owns target rows:

```ciac
record Customer {
    id: Uuid;
    email: String {
        non_empty: true;
        max_length: 320;
        format: email;
        unique: true;
    }
}

record Order {
    id: Uuid;

    customer: Reference<Customer> {
        references: Customers;
        cardinality: one;
        on_delete: restrict;
        on_update: cascade;
        index: true;
    }

    total: Float {
        min: 0;
    }

    created_at: Timestamp {
        index: true;
    }
}

table Customers: Customer;
table Orders: Order;
```

In a single-service program those top-level table declarations remain
valid. In a multi-service project, tables are declared inside their
owning service:

```ciac
project Commerce;

record Order {
    id: Uuid;
    total: Float { min: 0; }
}

service Ordering {
    use {
        db primary Postgres;
        db analytics Postgres;
    }

    table Orders: Order {
        db: primary;
    }
}
```

The `db` attribute may be omitted when the service has exactly one or an
unambiguous `default` database. It is required when several named
instances would otherwise match. Top-level tables in a multi-service
project are rejected because they do not identify a service/database
ownership boundary.

The required reference attributes are:

| Attribute | Values | Meaning |
|-----------|--------|---------|
| `references` | explicit table name | Resolves the storage target and verifies that it is backed by the named record |
| `cardinality` | `one`, `many` | Nested target value or list of target values |
| `on_delete` | `restrict`, `cascade` | Action when a referenced target row is deleted |
| `on_update` | `restrict`, `cascade` | Action when the referenced target key changes |

Optional storage attributes are:

| Attribute | Valid on | Default | Meaning |
|-----------|----------|---------|---------|
| `unique` | scalar or `Reference` with `one` cardinality | `false` | Unique database constraint; on a reference, one-to-one rather than many-to-one |
| `index` | portable scalar or any reference | `false` | Deterministic secondary index |

`Reference`, `transaction`, cardinalities, actions, formats, and field
attribute names join the shared vocabulary used by `ciac describe`, LSP
hover/completion, and documentation.

### Validation attributes

The portable v0.16 profile is deliberately small:

| Field kind | Attributes | Semantics |
|------------|------------|-----------|
| `String` | `non_empty`, `min_length`, `max_length`, `format` | Unicode scalar-count length; no trimming |
| `Int` | `min`, `max` | Inclusive signed integer bounds |
| `Float` | `min`, `max` | Inclusive finite decimal bounds |
| `Reference` | nested validation | Target record or list is recursively validated |
| `Uuid` | intrinsic | Canonical UUID parsing |
| `Timestamp` | intrinsic | RFC 3339/date-time parsing |
| enum, `Bool`, `Json` | type only | No custom validation attributes in this version |

The initial format registry is:

- `email`;
- `uri`.

`non_empty: true` normalizes to `min_length: 1`. Contradictory bounds
are compile-time errors. Signed integer and decimal attribute literals
must be supported; declaration attributes that currently require
non-negative integers continue to validate that restriction in sema.

### Transactions

```ciac
handler StoreOrder(input: NewOrder) -> Order {
    let order = Order {
        id: Uuid.new(),
        customer: input.customer,
        total: input.total,
        created_at: Timestamp.now()
    };

    let audit = OrderAudit {
        id: Uuid.new(),
        order_id: order.id,
        action: "created"
    };

    transaction {
        db.insert(Orders, order);
        db.insert(OrderAudits, audit);
    }

    return order;
}
```

`transaction` is a statement with a lexical block:

- outer locals are visible;
- inner locals do not escape;
- sequential transaction blocks are legal;
- nested blocks are rejected;
- every database verb in one block must resolve to the same capability
  instance;
- the default database is accepted only when unambiguous;
- `let`, `if`, `match`, pure expressions, database verbs, and `fail` are
  legal;
- `return` is rejected because it could bypass the generated
  commit/rollback epilogue;
- `publish` and every non-database capability verb are rejected in
  v0.16;
- an empty or database-free transaction is an error.

There is no configurable isolation level in this release. The selected
provider's default isolation applies.

## Pillar 1 — Resolved relation semantics

### Semantic identity and storage ownership

The type checker resolves every reference to:

- target `RecordId`;
- target `TableId`;
- owning service;
- concrete database capability instance;
- target identity field;
- cardinality and actions.

Both source and target tables must belong to the same service and
database instance. A relational foreign key cannot cross a network or
two physical databases.

To make that check meaningful:

- explicit tables gain service/database ownership in normalized IR;
- a single-service top-level table with one/default database keeps its
  current bare source form;
- a single-service program with several named databases must use the
  `{ db: <instance>; }` table tail for every otherwise ambiguous table;
- in a multi-service project, the table is written in the owning service
  block;
- `table Orders: Order { db: primary; }` binds it to a named database;
- a multi-database service must either name that instance or have an
  unambiguous `default`; naming conventions never decide storage.

Forward table references are legal. Sema therefore predeclares records,
tables, services, and database instances before resolving field
attributes.

The target table must be backed by the `Reference<T>` record, and that
record must contain `id: Uuid`. The source record must also have
`id: Uuid` when a many-valued relation needs a link table.

### One-valued storage

For:

```ciac
customer: Reference<Customer> {
    references: Customers;
    cardinality: one;
    on_delete: restrict;
    on_update: cascade;
}
```

the row model contains:

```text
customer_id UUID/TEXT NOT NULL
FOREIGN KEY customer_id REFERENCES customers(id)
  ON DELETE RESTRICT
  ON UPDATE CASCADE
```

The public model contains `customer: Customer`. Inserts and updates take
the nested target's `id`; non-identity target fields are validated as
wire data but do not mutate the target row. Reads hydrate the target from
its table so stored target state remains authoritative.

`unique: true` adds a unique constraint to the FK column, producing
one-to-one ownership from the source side. Without it, several source
rows may refer to one target.

### Many-valued storage

For:

```ciac
tags: Reference<Tag> {
    references: Tags;
    cardinality: many;
    on_delete: cascade;
    on_update: cascade;
}
```

the compiler owns a deterministic link table:

```text
orders__tags(
  source_id,
  target_id,
  PRIMARY KEY(source_id, target_id),
  FOREIGN KEY source_id REFERENCES orders(id) ON DELETE CASCADE,
  FOREIGN KEY target_id REFERENCES tags(id)
    ON DELETE CASCADE ON UPDATE CASCADE
)
```

The declared target action applies to the target-side link. Deleting a
target under `cascade` removes links; it does not delete source rows.
Deleting a source always removes compiler-owned links. Targets remain
independently owned.

Many-valued results have deterministic target-ID ordering. This is a
portable output rule, not a claim that an unordered SQL query happens to
return that order.

### Runtime verb behavior

The existing typed database verbs become relation-aware:

- `db.insert` writes the source row and relation links;
- `db.update` replaces the complete relation assignment represented by
  the full record value;
- `db.get` hydrates nested one/many targets;
- `db.query` hydrates each result without N+1 target queries;
- `db.delete` and `db.delete_where` rely on database-enforced actions.

One relation-bearing verb may expand to several SQL statements. It is
internally atomic even outside an explicit transaction: Python commits
after the complete logical verb; Rust opens a short SQLx transaction.
Inside `transaction`, the verb reuses the enclosing executor and never
nests.

Predicates remain local-column predicates. Relation traversal, joins,
and nested `where` expressions are out of scope.

### Relation graph restrictions

v0.16 rejects:

- unknown record or table targets;
- target-table/record mismatch;
- cross-service or cross-database references;
- missing `id: Uuid`;
- self-reference or a direct/indirect relation cycle;
- `unique` on a many relation;
- duplicate relation/storage attributes;
- a relation-backed record used by untyped CRUD;
- a relation whose target exists only as CRUD startup-created storage.

The CRUD restriction is an honest initial cut. The live compiler has two
different persistence/evolution paths. Applying a foreign key only on a
fresh CRUD install while omitting it from migrations would violate the
core build contract. Validation-only attributes do work on typed CRUD
records; relation-aware CRUD waits for storage unification.

## Pillar 2 — Unique constraints, indexes, and migration evolution

### Deterministic constraint identities

Generated logical names are stable:

```text
uq_<table>_<field>
ix_<table>_<field>
fk_<table>_<field>_<target>
pk_<source>__<field>
```

Names exceeding a provider limit are truncated with a stable semantic
hash. The semantic snapshot compares constraint meaning, not the rendered
name, so improving a truncation implementation does not masquerade as a
drop-and-add migration.

`unique: true` implies an index. `index: true` on the same field is
accepted and coalesced, not emitted twice. `Json` is neither uniquely
constrained nor indexed in the portable profile.

Composite, partial, expression, full-text, and provider-specific indexes
are deferred. v0.19's index lint initially reasons about the single-field
indexes declared here.

### One target-neutral storage snapshot

`evolution.rs` grows from boundary-record compatibility into the
target-neutral owner of semantic storage comparison. `migrations.rs`
remains responsible for rendering approved changes into dialect SQL.

The manifest snapshot becomes service/database scoped and records:

- logical column types;
- primary key;
- foreign keys with target, cardinality, and actions;
- unique constraints;
- indexes;
- compiler-owned link tables;
- validation/wire metadata separately from storage metadata.

The current debug-string representation of record field types is
replaced with a stable serialized type enum. A v0.15 manifest remains
readable: its shallow `tables` and `records` fields are imported into the
new shape on the first successful v0.16 build, and no old migration file
is rewritten.

Python and Rust must receive byte-identical migration SQL for the same
database engine. SQL type rendering occurs after semantic comparison so
the snapshot is not accidentally Postgres-shaped.

### Evolution safety matrix

| Change | v0.16 behavior |
|--------|----------------|
| Add ordinary index | Generate `CREATE INDEX` |
| Add unique constraint | Generate migration, with a warning that existing duplicates can fail it |
| Remove index or unique constraint | Refuse; manual migration |
| Add relation on a newly created table | Generate in initial schema |
| Add many relation to existing tables | Create an empty link table |
| Add required one relation to a populated table | Refuse; no backfill value |
| Remove relation | Refuse |
| Change relation target/cardinality/action | Refuse |
| Scalar ↔ reference conversion | Refuse as a retype |
| Drop/retype column or table | Preserve current `CIAC0046` refusal |
| Add FK to existing Postgres/MySQL table | Generate only when structural preconditions are safe |
| Add FK to existing SQLite table | Refuse in v0.16; do not guess a table rebuild |
| Rename field/table/constraint | Seen as remove+add and refused until v0.18's rename/evolution work |
| Relax validation rule | Wire-compatible |
| Tighten validation on a boundary type | Breaking |
| Change nested reference target/cardinality on a boundary type | Breaking |

Adding a required field to a record that crosses a service boundary is
classified conservatively as breaking. The old “any added field is
additive” rule is not defensible while records have no optional/default
field semantics.

### Transitive boundary compatibility

When a record contains a reference, the target record becomes part of
its wire contract. If `Order` crosses a `call` or shared stream and
embeds `Customer`, tightening `Customer.email` must name every consumer
of `Order`, even if `Customer` never appears as a top-level payload.

Validation rules normalize before comparison:

- `non_empty: true` equals `min_length: 1`;
- increasing a minimum, decreasing a maximum, or adding/changing a
  format tightens;
- the inverse relaxes.

This transitive closure becomes the foundation v0.18's general semantic
diff builds on.

## Pillar 3 — Explicit database transactions

### Python lowering

A transaction opens one session/transaction boundary and routes every
database operation in the block through it:

```python
async with sessionmaker() as __session:
    async with __session.begin():
        # generated block; no inner commit
        ...
```

The exact generated form must satisfy:

- one connection/session for the block;
- one commit after the block succeeds;
- rollback on `fail`, database error, validation error, or cancellation;
- no per-verb `commit()` inside the block;
- clean close after either outcome.

Lowering becomes executor-aware rather than allowing mutation helpers to
commit unconditionally. A handler that also performs database work
outside the transaction may still receive the existing request/message
session for that work.

### Rust lowering

Rust begins one SQLx transaction from the resolved concrete pool:

```rust
let mut tx = self.db.begin().await?;
let result = async {
    // generated DB verbs use &mut *tx
    Ok::<_, anyhow::Error>(())
}.await;

match result {
    Ok(()) => tx.commit().await?,
    Err(error) => {
        tx.rollback().await?;
        return Err(error);
    }
}
```

The lowerer carries an executor context:

- ordinary operations use the provider-specific pool;
- relation-bearing logical verbs may create an internal transaction;
- explicit blocks use the existing mutable transaction;
- `fail` returns through the wrapper so rollback is not skipped.

Postgres, MySQL, and SQLite remain generation-time branches, not one
dynamic database abstraction.

### The exact guarantee

v0.16 does not reinterpret all existing code:

- mutations outside a block keep current per-verb commit/autocommit;
- CRUD methods keep their existing per-operation transaction;
- a block does not cover preceding/following handlers or pipeline steps;
- no pipeline-level publish is included;
- no distributed transaction, saga, outbox, retry, or exactly-once claim
  is made;
- no isolation level is exposed.

The acceptance statement is narrow and testable:

> Every database statement lexically inside one valid `transaction`
> block uses one database transaction, and either all of its writes become
> visible or none do.

## Pillar 4 — Validation on every typed boundary

### Enforcement points

A declared validation rule applies at:

- typed HTTP request ingress;
- typed CRUD input for validation-only records;
- inline-handler record construction and update;
- classic/extern handler return before the next typed step;
- typed stream consumption;
- cross-service client response decoding;
- `db.insert` and `db.update`;
- row-to-domain hydration;
- typed publish and HTTP response egress.

Calling validation only in route extractors would allow invalid records
created inside handlers or read from a manually modified database to
escape. Both backends use one generated validation entry point at all
those boundaries.

### Python

Pydantic models use generated constrained/annotated fields and nested
models:

- length and numeric bounds;
- email/URI validators;
- forward references and final model rebuilds;
- recursive validation of relation values;
- deterministic field-path errors.

SQLAlchemy row models remain scalar. Relation loading is explicit async
code; serialization cannot trigger hidden lazy I/O.

### Rust

Serde remains responsible for serialization/deserialization only.
Generated Rust records use the `validator` derive/runtime crate for the
portable length, range, email, URI, and nested-validation profile;
generated routes, workers, clients, HIR constructors, and row conversions
call `Validate::validate` explicitly. Python uses Pydantic v2's annotated
constraints and validators.

Library-specific errors are normalized into the CIaC shape. The public
contract is a shared conformance corpus, not every edge of either
library's RFC parser; any rule the two selected libraries cannot agree on
is removed from the v0.16 format profile before M1 freezes it.

### Normalized runtime errors

Both targets return the same internal/public shape for request
validation failures:

```json
{
  "error": "validation_failed",
  "fields": [
    {
      "path": "customer.email",
      "rule": "format",
      "message": "must be a valid email"
    }
  ]
}
```

HTTP request validation uses 422. Entries sort by field path and rule.

Unique/reference violations map to a stable 409 shape on generated API
paths:

```json
{
  "error": "constraint_violation",
  "constraint": "uq_customers_email",
  "kind": "unique"
}
```

Workers and jobs feed the same normalized cause into their existing
failure/retry path after rolling back any open transaction.

Validation bounds are not translated into SQL `CHECK` constraints in
v0.16. A manual database write may violate them; hydration then fails
loudly rather than constructing an invalid domain record.

## Pillar 5 — Nested OpenAPI and TypeScript-client types

### Shared model

`FieldTypeKind` gains a language-neutral recursive variant:

```text
Reference {
    target_record,
    target_table,
    cardinality,
    on_delete,
    on_update
}
```

`FieldCtx` also carries normalized validation and storage flags. The
external protocol receives enums for cardinality/referential action and
must version the wire schema rather than asking external backends to
ignore an unknown exhaustive variant.

The protocol still does not expose enough HIR for an external backend to
lower all typed handler bodies. v0.16 does not solve that separate
limitation; bundled Python/Rust are the transaction proof targets.

### OpenAPI

Reference fields map to:

```text
one  -> $ref to the target record component
many -> array(items = $ref to the target record component)
```

The schema also emits:

- `minimum` / `maximum`;
- `minLength` / `maxLength`;
- standard `format` where applicable;
- 409 and 422 response components;
- `x-ciac-relation` containing target/cardinality/actions;
- `x-ciac-storage` for unique/index metadata.

OpenAPI 3.0 `$ref` siblings are avoided with `allOf` where metadata must
accompany a reference. The no-dangling-reference test walks nested
components and cycle rejection keeps recursion finite.

### TypeScript client

The v0.15 client becomes recursively typed:

```ts
export interface Order {
  id: string;
  customer: Customer;
  tags: Tag[];
  total: number;
  created_at: string;
}
```

Rules TypeScript cannot enforce structurally appear as JSDoc:

```ts
/** Non-empty; maximum length 320; format: email. */
email: string;
```

The client does not duplicate server-side runtime validation. It does
expose nested types and typed 409/422 error payloads. This is an
extension of the existing client, not a service backend.

## Diagnostics

The append-only registry receives a contiguous v0.16 block after
`CIAC0053`. Final numbers are assigned once the complete implementation
set is known; meanings are frozen before release.

Required new meanings include:

| Symbolic code | Trigger |
|---------------|---------|
| `UnknownReferenceTarget` | Referenced record/table does not exist or does not match |
| `InvalidReferenceDefinition` | Missing cardinality/action, invalid ID shape, or incompatible attribute |
| `CrossStorageReference` | Source and target differ by service or database instance |
| `CyclicReferenceGraph` | Self or indirect relation cycle |
| `InvalidStorageConstraint` | Unsupported/unused `unique` or `index` declaration |
| `InvalidFieldValidation` | Wrong attribute/type, contradictory bounds, or unknown format |
| `InvalidTransactionBlock` | Nested/empty/multi-database block or illegal control flow |
| `NonTransactionalEffect` | Publish/cache/http/email/store/search effect inside a v0.16 transaction |

Every code requires:

- registry title and explanation;
- `docs/errors.md`;
- one negative UI fixture with exact expected code;
- resolved JSON/LSP labels;
- a structured fix only where the edit is unambiguous.

Useful fixes may insert missing required reference attributes or rename a
near-miss table. Cycles and cross-database ownership remain prose-only.

## Implementation map

### `crates/ciac-syntax`

- `lexer.rs`: reserve `transaction`; parse signed numeric literals
  without weakening existing attribute validation.
- `ast.rs`: field attributes, `TypeExpr::Reference`, and
  `Stmt::Transaction`, each with precise spans.
- `parser.rs`: field tails, recursive reference type, transaction block,
  and service-local tables.
- `module.rs`: no new composition model; imported files remain textual
  splice inputs.
- `blueprints`: reference targets and field attributes must survive
  substitution/hygienic renaming.

### `crates/ciac-ir` and `crates/ciac-sema`

- Extend record fields with normalized validation/storage metadata and a
  resolved reference type.
- Extend HIR with transaction blocks and recursive reference/list types.
- Give tables concrete service/database ownership.
- Predeclare records/tables before relation resolution.
- Add a relation-validation pass for ownership, identity shape, and
  acyclicity.
- Make type checking transaction-context aware.
- Traverse nested records when computing compatibility and capability
  use.
- Keep graph insertion deterministic.

### `crates/ciac-diagnostics`

- Append the new code block.
- Preserve stable code serialization and default severity.
- Extend fix-property tests for any new offered edit.

### `crates/ciac-codegen`

- `model.rs`: recursive `FieldTypeKind::Reference`, validation/storage
  contexts, table ownership, and relation operation contexts.
- `evolution.rs`: stable nested wire/storage snapshots, transitive
  consumers, FK/constraint/index diff.
- `migrations.rs`: logical-to-dialect rendering, topological table
  creation, link tables, deterministic names, SQLite refusal boundary.
- `manifest.rs`: versioned scoped storage snapshot with backward
  deserialization.
- `protocol.rs`: protocol/schema update and round-trip tests.
- `openapi.rs`: nested refs, validation keywords, error components.
- `ts_client.rs`: recursive interfaces and validation JSDoc.
- `system_tests.rs`: relation, cascade, uniqueness, validation, and
  rollback checks.

### Python backend

- Make lowerer mutation helpers executor/commit aware.
- Lower transaction blocks to one session boundary.
- Generate scalar FK/link row models separately from nested Pydantic
  records.
- Batch relation hydration rather than one query per nested row.
- Enable SQLite FK enforcement.
- Normalize 409/422 errors.
- Generate behavioral tests asserting one commit or one rollback.

### Rust backend

- Make SQL lowering generic over pool versus mutable transaction
  execution.
- Generate row/domain conversion for nested references.
- Generate and call validation at every typed boundary.
- Enable SQLite foreign keys.
- Normalize constraint/validation errors.
- Add fake/string/live tests until v0.17 supplies the common simulation
  seam.

### CLI and machine surfaces

- `commands.rs`: compare semantic storage before rendering migrations
  and place migration files under the owning service.
- `vocab.rs` / `describe.rs`: types, attributes, formats, actions.
- `lsp.rs`: completion/hover for field attributes and transaction rules.
- JSON commands expose the new diagnostics without human narration on
  stdout.
- External protocol schema and checked-in JSON remain byte-identical.

### Documentation

- Rewrite the record/table/attribute grammar in `docs/language.md`.
- Add transaction semantics and effect restrictions to
  `docs/expressions.md`.
- Update `docs/ir.md` and `docs/architecture.md`.
- Document migration/refusal behavior in `docs/regeneration.md`.
- Update OpenAPI/client and protocol guides.
- Append all errors to `docs/errors.md`.

## Verification strategy

### Flagship example

Add `examples/domain-orders.ciac` containing:

- one and many relations;
- restrict and cascade;
- unique and ordinary indexes;
- non-empty, length, numeric, email, and URI validation;
- a successful two-table transaction;
- a deliberate second-write failure;
- nested API/OpenAPI/TypeScript output.

Keep `examples/order-system.ciac` as a useful witness of the existing
CRUD/table split rather than silently changing its meaning.

### Compiler tests

- Parser recovery for malformed field attrs and transaction blocks.
- Forward-reference and ownership resolution.
- One negative fixture per new diagnostic.
- Relation-cycle and long-name determinism.
- HIR transaction/effect restrictions.
- Manifest v0.15 → v0.16 transition.
- Every storage-evolution matrix row.
- Transitive boundary consumers and validation tightening.
- Protocol schema staleness and round-trip.

### Generated behavior

For both targets:

1. insert target/source and read the nested target;
2. insert/read a many relation with deterministic ordering;
3. reject a missing target;
4. reject duplicate unique data;
5. prove restrict behavior;
6. prove one-reference cascade behavior;
7. prove link cleanup for many-reference cascade;
8. commit both writes on transaction success;
9. roll back the first write after a second failure;
10. reject invalid nested input before handler execution;
11. serve the same OpenAPI document;
12. type-check the generated TypeScript client.

Postgres is the full reference matrix. MySQL and SQLite run focused
parity fixtures, including SQLite FK enforcement and explicit evolution
refusals. A clean ephemeral database proves generated behavior, not the
duration or safety of applying a migration to production data.

### Golden and CI discipline

- Existing examples without new attributes remain byte-identical where
  shared model changes permit.
- Review IR, DOT, Python, Rust, and TS snapshots intentionally.
- `cargo fmt --check`, warnings-denied clippy, and full workspace tests
  remain mandatory.
- `ciac verify` runs every example on both targets.
- The new flagship joins selected compose-backed verification for both
  targets.

## Milestones

1. **M1 — Surface freeze:** field tails, `Reference<T>`, validation
   profile, table ownership spelling, and `transaction`; parser/recovery
   tests and exact grammar.
2. **M2 — Resolved semantics:** relation/table predeclaration, IR
   metadata, transaction HIR, ownership/cycle checks, append-only
   diagnostics, blueprint preservation.
3. **M3 — Evolution and migrations:** target-neutral scoped snapshots,
   manifest transition, dialect rendering, deterministic constraints,
   complete safety-matrix tests.
4. **M4 — Validation and wire artifacts:** Python/Rust validation,
   normalized errors, nested OpenAPI, nested TypeScript client, external
   protocol update.
5. **M5 — Python relations and transactions:** FK/link persistence,
   eager/batched hydration, executor-aware lowering, rollback and
   cascade behavior.
6. **M6 — Rust relations and transactions:** SQLx row/domain split,
   mutable transaction executor, matching behavior and error shapes.
7. **M7 — Full provider matrix and reconciliation:** flagship,
   generated/system verification, docs, vocab/LSP/describe, golden
   review, version 0.16.0, and whole-version analysis.

No milestone is accepted because templates render plausible code. The
behavioral matrix is the release bar.

## Explicit cuts

- No full TypeScript backend.
- No relation-aware CRUD in the initial release.
- No optional/null fields or `SET NULL`.
- No cross-service/cross-database foreign keys.
- No inverse/back-reference generation.
- No self/cyclic relation graph.
- No composite/partial/expression/full-text indexes.
- No relation joins, nested predicates, aggregate traversal, or relation
  query planner.
- No partial relation patch; update replaces the complete assignment.
- No whole-handler, whole-pipeline, or distributed transaction.
- No non-database effect inside a transaction.
- No outbox, idempotency, saga, or exactly-once claim; those depend on
  v0.19's failure model.
- No automatic down migration, backfill, rename, duplicate cleanup, or
  table rebuild.
- No online/concurrent migration guarantee.
- No SQL `CHECK` generation for validation.
- No custom regex, cross-field rule, transform, or default value.
- No additional Kubernetes/Terraform/database deployment maturity.

## Risks

- **Records currently serve wire, handler, and persistence roles.**
  Relations increase that coupling. Mitigation: keep scalar row models
  separate from nested domain models and reject CRUD integration until
  its migration path is unified.
- **Nested loading can become expensive.** Mitigation: reject cycles,
  batch target loads, cap generated depth structurally, and make no
  production query-planner claim.
- **SQLite cannot add ordinary foreign keys with simple `ALTER TABLE`.**
  Mitigation: support complete fresh schemas and refuse unsafe existing
  evolution.
- **Adding unique/FK constraints can fail against dirty data.**
  Mitigation: deterministic migrations, explicit warnings, no claim that
  the compiler inspected production rows.
- **A legacy commit path can leak into a transaction.** Mitigation:
  explicit executor/commit mode, golden checks for forbidden inner
  commits, and live rollback tests.
- **Python and Rust validation libraries differ at standards edges.**
  Mitigation: a small closed profile and one shared conformance corpus.
- **The external protocol changes.** Mitigation: version negotiation,
  regenerated schema/docs, and loud refusal from older external
  backends.
- **The version is broad.** Mitigation: freeze semantics before backend
  work and keep CRUD relations, optionality, joins, composite
  constraints, and deployment work outside.

## Confidence and v0.17 handoff

Relations, constraints, explicit transactions, and declared validation
are structural: the missing semantics are observable in current
generated output, and parent-child domains cannot be completed honestly
without them.

v0.17 assumes this normalized domain model and turns it into the
strongest infrastructure-free feedback loop: a whole-system simulator
whose database fake enforces these references, cascades, constraints,
validations, and transaction boundaries rather than replacing them with
a dictionary.
