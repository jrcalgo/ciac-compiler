# Guide 2 — Records and CRUD

*Reader: builder, continuing from [guide 1](01-first-service.md).
Time: ~15 minutes. You need: a terminal — still no database, no
Docker; `crud` fakes nothing but needs no live Postgres to check,
build, or verify either.*

Guide 1 left off with `Ping`, one record, one echo endpoint. This
guide adds real, typed persistence — a database table your reader
never has to write a model or a migration for by hand.

<!-- ciac-verify:start id=new -->
```sh
ciac new my-app --template minimal
cd my-app
```
<!-- ciac-verify:end -->

## 1. `crud`: a typed resource, generated

Replace `main.ciac` with the record from guide 1, plus one line —
`crud Message: Message;` — and a `db` capability for it to persist to:

<!-- ciac-verify:file id=main-ciac-v2 path=main.ciac -->
```text
service Ping;

use {
    db Postgres;
}

record Message {
    id: Uuid;
    text: String;
    sent_at: String;
}
crud Message: Message;

api Echo: Message {
    method: POST;
    path: "/echo";
}

pipeline Echo: Return;
```
<!-- ciac-verify:end -->

`crud <Name>: <Record>;` expands into a full REST resource —
create/read/update/delete at `/messages` — with real typed columns,
backed by whichever `db` provider your `use` block names. It owns its
own table; don't also declare a separate `table` for the same record
(`table Messages: Message;` alongside the line above would collide —
two persistence layers fighting over one name). If you want a
hand-written table *instead of* generated CRUD, skip `crud` and
declare `table` directly — [docs/language.md](../language.md) covers
both.

<!-- ciac-verify:start id=check-and-build -->
```sh
ciac check main.ciac
ciac build main.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

<!-- ciac-verify:start id=verify -->
```sh
ciac verify main.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

Open `build/app/services/message_store.py` — the generated CRUD
handler, real SQLAlchemy, no ORM you didn't ask for.

## 2. Evolving the schema

Add a field to `Message`:

<!-- ciac-verify:file id=main-ciac-v3 path=main.ciac -->
```text
service Ping;

use {
    db Postgres;
}

record Message {
    id: Uuid;
    text: String;
    sent_at: String;
    read: Bool;
}
crud Message: Message;

api Echo: Message {
    method: POST;
    path: "/echo";
}

pipeline Echo: Return;
```
<!-- ciac-verify:end -->

Before rebuilding, preview exactly what changes:

<!-- ciac-verify:start id=diff -->
```sh
ciac diff main.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

`ciac diff` shows `app/models.py` and `app/schemas.py` updating (the
new column) and nothing else — every unrelated file stays `unchanged`.
This is the regeneration discipline the whole compiler is built
around: a schema change touches exactly the files whose shape
actually depends on that schema, and `ciac diff`/`ciac verify` let you
see that *before* trusting it. The schema itself is tracked in the
generated project's own `.ciac/manifest.json`; depth on how migrations
are derived from it lives in [docs/evolution.md](../evolution.md).

<!-- ciac-verify:start id=rebuild-and-verify -->
```sh
ciac build main.ciac --target python --out ./build
ciac verify main.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

## Checkpoint

A green `ciac verify` on a service with typed CRUD and a schema change
already survived once. If you see a `CIAC0035` warning about a
migration file "no longer produced" right after a fresh `build`, that
is expected, not a bug — migrations are diff-based, so the second
`build` in a row (with nothing new to migrate) correctly emits none;
the warning's wording is on this arc's own list of things to make
clearer, not a sign anything is broken here.

From here: [guide 3](03-handlers-and-logic.md) adds the one thing
`crud` can't give you for free — custom logic with its own transaction,
stream, and worker.
