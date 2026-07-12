# Handler-Body Expressions (v0.7)

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

## The closed verb set

A handler body can only call a *bound capability instance*'s verbs —
the exact set below, nothing else (`CIAC0043` for an unknown verb or
wrong arity/argument types; `CIAC0044` if the capability kind has no
bound instance in scope):

| Capability | Verb | Signature | Behavior |
|------------|------|-----------|----------|
| `db` | `insert` | `(table, record) -> record` | Inserts a row into a declared `table`, returns it back. |
| `db` | `get` | `(table, id) -> record` | Fetches a row by primary key. |
| `cache` | `get` | `(key) -> Json` | Reads a cached value. |
| `cache` | `set` | `(key, value) -> Unit` | Writes a cached value. |
| `object_store` | `put` | `(key, value) -> Unit` | Uploads a value under a key. |
| `object_store` | `get` | `(key) -> Json` | Downloads a value by key. |

`db.insert`/`db.get`'s first argument names a `table <Name>: <Record>;`
declaration (`CIAC0042` if it isn't one); the record type on both sides
is the table's declared record.

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

Every error this document mentions (`CIAC0038`-`CIAC0046`) is
documented in full in `docs/errors.md`, including which are warnings
vs. hard errors and how to fix each one.
