# Handler-Body Expressions (v0.7, extended v0.14)

Through v0.6, a pipeline handler was always an opaque business-logic
unit: the compiler generated a typed constructor and a `handle` stub,
and you filled in the body by hand in the target language. v0.7 adds a
second option — writing the body *in CIaC itself* — without changing
anything about the first: every v0.1-v0.6 program still compiles
unchanged, since a `handler` with no signature is still the classic
form.

```ciac
// Classic (v0.1-v0.6): capability bindings only, body is a seeded stub
// you implement once in the target language.
handler StoreVideo {
    db: main;
}

// extern (v0.7): a typed signature with the same seeded-stub contract
// as classic — implement it yourself, regeneration never overwrites it.
extern handler StoreVideo(v: Video) -> Video;

// Inline body (v0.7): the compiler lowers this straight to Python/Rust
// on every build. This file is compiler-owned; there's nothing to
// implement by hand.
handler StoreVideo(v: Video) -> Video {
    let inserted = db.insert(Videos, v);
    return inserted;
}
```

All three forms plug into a pipeline identically (`StoreVideo` as a
step); which one you use only changes where the logic lives and how
`ciac build` treats the generated file (see `docs/regeneration.md` for
`Owned` vs `Seeded`).

## Why the body language is small and closed

The roadmap that introduced this ("07UpdatePlan.md") calls scope creep
the failure mode for this feature: the moment handler bodies can do
*anything*, CIaC stops being a compiler with a checkable contract and
becomes an ad hoc scripting language with two backends to keep in sync.
So the language is deliberately narrow — no loops, no user-defined
functions, no arbitrary I/O — and every construct below is the *whole*
list, not a representative sample. Anything outside it is `extern`.

## Statements

```ebnf
stmt      = let-stmt | expr-stmt | return-stmt | fail-stmt | publish-stmt ;
let-stmt    = "let" IDENT "=" expr ";" ;
expr-stmt   = expr ";" ;
return-stmt = "return" [ expr ] ";" ;
fail-stmt   = "fail" IDENT "(" [ expr { "," expr } ] ")" ";" ;
publish-stmt = "publish" IDENT "(" expr ")" ";" ;
```

- **`let <name> = <expr>;`** — single-assignment, block-scoped binding.
  A binding that's never read is `CIAC0045` (warning, not an error —
  the source language doesn't require you to use everything you name).
- **A bare expression statement** — almost always a capability verb
  call made for its side effect (`object_store.put(..)` with the
  result discarded).
- **`return <expr>?;`** — ends the handler with a value matching its
  declared return type (`CIAC0040` on mismatch). `return;` with no
  value is only valid when the handler returns nothing.
- **`fail <ErrorName>(<args>);`** — an early, typed error response.
  `<ErrorName>` must be a declared `error` record; `<args>` must match
  its fields positionally.
- **`publish <Stream>(<value>);`** — publishes `<value>` to a declared
  stream from inside a handler body, reusing the same stream
  resolution and payload-type checking (`CIAC0016`/`CIAC0017`) as the
  pipeline-level `publish <Stream>` step.

## Expressions

```ebnf
expr        = ident | number | string | bool
            | field-access | index | call | record-cons
            | binary | unary | if-expr | match-expr ;
field-access = expr "." IDENT ;
index        = expr "[" expr "]" ;
call         = expr "(" [ expr { "," expr } ] ")" ;
record-cons  = expr "{" [ field-init { "," field-init } ] "}" ;
field-init   = IDENT ":" expr ;
binary       = expr bin-op expr ;
bin-op       = "+" | "-" | "*" | "/" | "==" | "!=" | "<" | "<=" | ">" | ">="
             | "&&" | "||" ;
unary        = ( "-" | "!" ) expr ;
if-expr      = "if" expr "{" { stmt } "}" [ "else" "{" { stmt } "}" ] ;
match-expr   = "match" expr "{" { expr-arm } "}" ;
expr-arm     = ( IDENT | "_" ) "->" "{" { stmt } "}" ;
```

- **Literals**: numbers (int/float distinguished by type inference, not
  syntax), strings, `true`/`false`.
- **Field access** (`v.title`) and **index** (`payload["key"]` — `Json`
  fields only) read a record/JSON value.
- **Record construction and functional update** share one syntax:
  `Video { id: .., title: .. }` builds a new `Video` when the base
  names a record *type*; `v { status: Ready }` copies `v` with just
  `status` replaced when the base is a record *value*. Which one a
  given `base { .. }` is gets resolved during type checking, not
  parsing — the grammar is identical either way.
- **`if <cond> { .. } [else { .. }]`** and **`match <expr> { Variant ->
  { .. } _ -> { .. } }`** are expressions, not statements: both
  branches must produce the same type, and that value is what the
  `if`/`match` evaluates to (matching how Rust's own `if`/`match`
  work). `match` arms cover a declared `enum`'s variants; every variant
  must be handled directly or by a trailing `_` (`CIAC0021`-style
  exhaustiveness, reused from pipeline-level `match`).
- **Calls** (`callee(args)`) are how every capability verb and builtin
  is invoked — see below. There is no other kind of call: no
  user-defined functions, no recursion.
- **`[Type]`** is a list type (v0.14 M1) — valid as a handler parameter
  or return type (`items: [String]`, `-> [Note]`). It is *not* valid as
  a `record` field type yet (`CIAC0053`): lists only ever arise from a
  list-returning verb (`db.query`, `object_store.list`,
  `search.query`), and a handler passes one through whole — there is no
  loop construct to iterate it with, matching the "no loops" rule
  above.

## The closed verb set

A handler body can only call a *bound capability instance*'s verbs —
the exact set below, nothing else (`CIAC0043` for an unknown verb or
wrong arity/argument types; `CIAC0044` if the capability kind has no
bound instance in scope). Every verb below generates on **both**
bundled targets (`ciac build --target python|rust`) — there is no
front-end-only verb left as of v0.14 M7.

| Capability | Verb | Signature | Behavior | Python lowering | Rust lowering |
|------------|------|-----------|----------|------------------|----------------|
| `db` | `insert` | `(table, record) -> record` | Inserts a row into a declared `table`, returns it back. | `session.add` + `flush`, SQLAlchemy Core model | `sqlx::query` `INSERT ... RETURNING`, engine-aware placeholders |
| `db` | `get` | `(table, id) -> record?` | Fetches a row by primary key. | `session.get(Model, str(id))` | `sqlx::query_as` `SELECT ... WHERE id = ?/$1`, `Option<Row>` |
| `db` | `update` | `(table, id, record) -> record?` | Updates the row at `id` with `record`'s fields. | `session.get` then attribute assignment + `flush` | `UPDATE` with per-engine bind order (`?`-style binds fields first, id last) |
| `db` | `delete` | `(table, id) -> Bool` | Deletes the row at `id`; `true` if one was deleted. | `session.get` + `session.delete`, `rowcount`-style bool | `sqlx::query` `DELETE ... WHERE id = ?/$1`, `rows_affected() > 0` |
| `db` | `query` | `(table) [where <predicate>] -> [record]` | Every row, or every matching row when a `where` clause is given. | SQLAlchemy Core `select(Model).where(...)`, bound parameters | `sqlx::query_as` with a built `WHERE` clause, bound parameters |
| `db` | `count` | `(table) [where <predicate>] -> Int` | The number of rows, or matching rows. | `select(func.count()).select_from(Model).where(...)` | `SELECT COUNT(*) FROM ... WHERE ...`, `i64` |
| `db` | `delete_where` | `(table) [where <predicate>] -> Int` | Deletes every (matching) row, returns the count deleted. | Core `delete(Model).where(...)`, `result.rowcount` | `sqlx::query` `DELETE ... WHERE ...`, `rows_affected()` |
| `cache` | `get` | `(key) -> Json?` | Reads a cached value. | `redis.get` + `json.loads` if present | `redis` `GET` + `serde_json::from_str` if present |
| `cache` | `set` | `(key, value) -> Unit` | Writes a cached value. | `redis.set` with `json.dumps`-encoded value | `redis` `SET` with `serde_json::to_string`-encoded value |
| `cache` | `delete` | `(key) -> Unit` | Removes a cached value. | `redis.delete` | `redis` `DEL` |
| `object_store` | `put` | `(key, value) -> Unit` | Uploads a value under a key. | `put_object` (aioboto3), record → JSON bytes | `put_object` (rust-s3), record → JSON bytes |
| `object_store` | `get` | `(key) -> Json?` | Downloads a value by key. | `get_object` + `json.loads` | `get_object` + `serde_json::from_slice` |
| `object_store` | `delete` | `(key) -> Unit` | Removes an object by key. | `delete_object` | `delete_object` |
| `object_store` | `list` | `(prefix) -> [String]` | Every key under `prefix`. | `list_objects_v2` paginated, keys collected | `list_objects` paginated, keys collected |
| `email` | `send` | `(to, subject, body) -> Unit` | Sends an email. | `aiosmtplib.send` | `lettre` async transport `send` |
| `search` | `index` | `(doc_id, value) -> Unit` | Upserts `value` under `doc_id`. | OpenSearch client `index` against the service's fixed index name | OpenSearch client `index` against the service's fixed index name |
| `search` | `query` | `(query) -> [Json]` | Every matching document. | OpenSearch `search` with a `query_string` body | OpenSearch `search` with a `query_string` body |
| `external_http` | `request` | `(url, body) -> Json` | A synchronous POST, returning the response body. | `httpx.AsyncClient.post(...).json()` | `reqwest::Client::post(...).json()` |

`db.*`'s table argument names a `table <Name>: <Record>;` declaration
(`CIAC0042` if it isn't one); the record type is the table's declared
record. `search.index`/`search.query` use one fixed index name derived
from the service (there is no per-call index argument).

This table's shape reflects how the verb set actually grew:
`db.insert`/`db.get`, `cache.get`/`cache.set`, and
`object_store.put`/`object_store.get` shipped in v0.7 alongside the
handler-body language itself. Every other row — the `where`-clause
query family, `update`/`delete`-by-key, `cache.delete`,
`object_store.delete`/`list`, `email.send`, both `search` verbs, and
`external_http.request` — was added by v0.14: M1 landed the front-end
(grammar, typeck, the `where`-clause grammar below) for the whole set
at once so both backends target one frozen surface; M2 lowered the
`db` query/mutation family on both backends against live Postgres,
MySQL, and SQLite; M3 lowered `cache`/`external_http`; M4 lowered
`email`/`object_store`/`search`.

### `where` clauses (predicates)

`db.query`, `db.count`, and `db.delete_where` optionally take a
trailing `where` clause — a conjunction of comparisons against the
target table's columns:

```ebnf
query        = call "where" predicate ;
predicate    = pred-term { "&&" pred-term } ;
pred-term    = IDENT pred-op expr ;
pred-op      = "==" | "!=" | "<" | "<=" | ">" | ">=" | "contains" ;
```

```ciac
handler ActiveByAuthor(author: String) -> [Note] {
    return db.query(Notes) where author == author && active == true;
}
```

A `where` clause's left-hand side always names a column of the verb's
table — it is a *separate* namespace from the handler body's own
locals (unlike a normal expression, a bare identifier there can never
mean a `let`/param). Its right-hand side is a normal expression,
evaluated in the enclosing scope, so it can reference params and
`let`s. `contains` is substring matching and only applies to `Str`
columns. Attaching a `where` clause to any other verb is `CIAC0052`.

## Builtins

Two niladic functions are available in any handler body regardless of
capability bindings:

| Builtin | Returns |
|---------|---------|
| `Uuid.new()` | a fresh `Uuid` |
| `Timestamp.now()` | the current `Timestamp` |

## `table <Name>: <Record>;` and migrations

A `table` declaration (see `docs/language.md`) gives a handler body
something to `db.insert`/`db.get` against, and gets **incremental SQL
migrations** instead of a create-if-absent call: each `ciac build`
diffs the current program's tables against the schema recorded in the
previous build's manifest, emitting a numbered, additive-only
migration file for anything new (`CREATE TABLE` for a new table,
`ALTER TABLE ... ADD COLUMN` for a new column). A column being removed
or retyped, or a table disappearing, is refused as `CIAC0046` — write
that change by hand. Migration files are `Seeded` (see
`docs/regeneration.md`): once written, later builds never touch them
again. `crud <Name>: <Record>;` resources are unrelated to this — they
keep the pre-existing `create_schema`/`ensure_schema` create-if-absent
behavior.

## Errors

Every error this document mentions (`CIAC0038`-`CIAC0046`,
`CIAC0052`-`CIAC0053`) is documented in full in `docs/errors.md`,
including which are warnings vs. hard errors and how to fix each one.
