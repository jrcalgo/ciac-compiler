# CIaC v0.7 — Behavior: Handler Bodies in the Language (roadmap forecast)

> Forecast document. v0.6's regeneration + jobs/channels are assumed
> landed. Details below set direction and scope; exact grammar and codes
> get finalized in the v0.7 planning pass.

## The gap this version closes

Through v0.6, CIaC generates *all* wiring but no *behavior*: every
pipeline bottoms out in a handler stub whose body is
`# TODO: implement`. That caps the value proposition at scaffolding —
goal 3 ("fully implemented code") and goal 4 ("fully functioning
systems guaranteed by the compiler") require the compiler to be able to
compile *what handlers do*, not just *where they sit*.

**v0.7 theme: a typed, deliberately small expression language for
handler bodies**, lowered to both hosts, with the existing stub
mechanism preserved as the escape hatch (`extern handler`). The design
principle is the same one that has held since v0.1: CIaC only admits
constructs it can fully generate and validate. This is *not* a
general-purpose language — it is the 80% of backend handler bodies:
validate, transform, persist, look up, branch, call, publish, respond.

## Language design (direction)

### Inline handler bodies

```ciac
handler StoreVideo(v: Video) -> Video {
    let key = "videos/" + v.id;
    object_store.put(key, v);          // bound instance verbs
    db.insert(Videos, v);              // typed table from `table` decl
    cache.set("video:" + v.id, v, ttl: 60);
    return v { status: Ready };        // record update syntax
}
```

- **Declarations**: `let` bindings, single-assignment, block-scoped.
- **Expressions**: field access, record construction `Video { .. }` and
  functional update `v { status: Ready }`, string/arith/bool operators,
  comparisons, enum literals, `Uuid.new()`, `Timestamp.now()`,
  `Json` field indexing.
- **Control flow**: `if/else` expressions and `match` over enum fields
  (reusing the existing exhaustiveness machinery, CIAC0021), early
  `fail <ErrorName>` for typed error responses (lowers to 4xx/5xx with
  a declared error record).
- **Capability verbs** (the only effects): each binding's kind defines
  a closed verb set — `db.insert/get/update/delete/query(<table>, …)`
  over declared `table <Name>: <Record>;`; `cache.get/set/delete`;
  `object_store.put/get/delete/presign`; `email.send(to, subject,
  body)`; `search.index/search/delete`; `http.<method>(path, body?)`;
  `publish <Stream>(expr)` inside bodies. Verbs are typed against the
  binding and the record system — a misspelled field or wrong payload
  is a compile error, not a runtime 500.
- **`extern handler X(v: Video) -> Video;`** — exactly today's stub
  behavior, seeded file, user-implemented. Existing programs stay valid
  by treating bare pipeline handler references as extern (zero
  migration).

### Typing

A bidirectional checker over the closed `FieldType` set + records +
enums + `List<T>`/`Option<T>` (new, needed for `query`/`get` results).
New error family (~CIAC0040–0049): unknown name, type mismatch,
non-exhaustive match in expressions, wrong verb arity, verb on an
unbound capability, unused `let` (warning). Handler signature must
agree with the pipeline payload where invoked (extends CIAC0016).

## Compiler pipeline changes

- **`ciac-syntax`**: expression grammar (Pratt parser layered on the
  existing recursive descent), `table` declarations, `error` record
  declarations, `extern` keyword. AST gains an `Expr` tree.
- **New crate `ciac-typeck`** (or a sema module): expression checker
  producing a **typed HIR** — every node annotated, every verb resolved
  to (capability instance, operation, table). The HIR joins
  `NormalizedIr`; backends never see raw expression AST.
- **`ciac-ir`**: `HandlerBody { params, hir }` on Service components;
  `Table { name, record }` side table; DataFlow edges from verb usage
  replace/augment the v0.4 binding edges (bindings become *inferred*
  from verbs, with explicit `handler X { db: main; }` blocks retained
  for disambiguation only).
- **`ciac-codegen`**: a small **lowering layer per backend** — an
  `ExprCtx` tree the templates walk, or (likely cleaner) direct string
  lowering in Rust code with per-host emitters
  (`py_expr(hir) -> String`, `rust_expr(hir) -> String`) so templates
  stay presentational. Verb lowering reuses the v0.5.1 runtime wrappers
  exactly as they exist (ObjectStore.put, Email.send, …) — they were
  designed as this target.
- **DRY guarantee (goal 3)**: one HIR, two emitters, zero duplicated
  semantics; the shared model remains the single source of meaning.

## Generated-output shape

- Inline handlers emit into **compiler-owned** files
  (`app/logic/<snake>.py`, `src/logic/<snake>.rs`) — regeneration
  rewrites them freely under the v0.6 manifest rules; extern handlers
  stay seeded in `services/`.
- `db.*` verbs require real tables: v0.7 also brings **migrations**
  (`app/migrations/` via generated SQL + a tiny runner; Rust: sqlx
  migrations) replacing `create_schema`'s create-if-absent for tables
  declared in the language. Deterministic: migration files derive from
  the table set diff recorded in the manifest.
- Generated tests level up: for each inline handler the compiler emits
  a **behavioral test** exercising the body against fakes (in-memory
  cache/table implementations of the runtime wrappers' interfaces) —
  the compiler proves its own lowering per project (goal 4).

## Verification bar

- Golden: every example gains at least one inline handler; snapshots
  reviewed.
- Property: lowering equivalence suite — a table of HIR programs ×
  inputs run through the generated Python *and* Rust (via the emitted
  behavioral tests) asserting identical observable outputs. This is the
  heart of the release: two hosts, one meaning.
- Live: reprise the v0.5.1 round-trip with real behavior — upload
  pipeline computes a presigned URL via inline body against MinIO,
  billing computes a real field update, both hosts, identical
  responses.
- All v0.1–v0.6 programs compile unchanged (extern default).

## Milestones

1. Grammar + AST + parser recovery for expressions, `table`, `extern`.
2. Type checker + HIR; error codes; negative fixture suite (largest
   fixture addition since v0.1).
3. Python emitter + runtime fakes + behavioral tests.
4. Rust emitter + equivalence suite green.
5. Migrations for declared tables; manifest integration.
6. Examples, docs (`docs/expressions.md`, status table update), live
   proofs, version 0.7.0.

## Risks

- **Scope creep is the failure mode.** The verb set is closed and the
  language stays expression-oriented (no loops in v0.7; `query` +
  comprehension-style `map` only). Anything else is `extern`.
- Host semantic drift (string ops, numeric coercion, JSON nulls) —
  contained by the equivalence suite; where hosts genuinely differ,
  the language forbids the construct rather than papering over it.
- Migrations interact with regeneration; they ride the v0.6 manifest
  (owned, append-only) and get their own conflict code.
