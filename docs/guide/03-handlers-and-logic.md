# Guide 3 — Handlers and logic

*Reader: builder, continuing from [guide 2](02-records-and-crud.md).
Time: ~20 minutes. You need: a terminal — still no live database or
broker; a `transaction` block is checked and generated without one.*

`crud` gave `Message` a full REST resource for free. This guide adds
the thing `crud` can't: a handler with its own business logic, wrapped
in a `transaction`, that tells the rest of the system what happened
through a stream — consumed by a worker that doesn't know or care who
published it.

<!-- ciac-verify:start id=new -->
```sh
ciac new my-app --template minimal
cd my-app
```
<!-- ciac-verify:end -->

## 1. A handler with a transaction

Starting from guide 2's final `main.ciac` (the `Message` record,
`crud`, and the `read` field), add a `ReadReceipt` — a second table,
separate from `crud`'s own, that a custom handler writes to inside a
`transaction`:

<!-- ciac-verify:file id=main-ciac-v4 path=main.ciac -->
```text
service Ping;

use {
    db Postgres;
    queue NATS;
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

record ReadReceipt {
    id: Uuid;
    message_id: Uuid;
}
table ReadReceipts: ReadReceipt;
stream MessageRead: Message;

handler MarkRead(message: Message) -> Message {
    transaction {
        db.insert(ReadReceipts, ReadReceipt { id: Uuid.new(), message_id: message.id });
    }
    return message;
}

api MarkReadRoute: Message {
    method: POST;
    path: "/messages/read";
}
pipeline MarkReadRoute: MarkRead -> publish MessageRead -> Return;

worker LogRead on MessageRead { max_retries: 2; }
handler RecordRead(message: Message) -> Message { return message; }
pipeline LogRead: RecordRead;
```
<!-- ciac-verify:end -->

Three new pieces, each doing exactly one job:

- **`transaction { ... }`** in `MarkRead` — the audit insert either
  fully happens or fully doesn't. A failure inside the block rolls
  back everything the block wrote, not just the last statement; see
  [docs/expressions.md](../expressions.md)'s `transaction` section
  for the full rules. This series' own simulation guide (later in
  the series) shows how to *prove* the rollback with an injected
  failure instead of trusting the description.
- **`stream MessageRead: Message;`** plus `publish MessageRead` in
  `MarkReadRoute`'s pipeline — `MarkRead` doesn't know or call
  anything downstream; it just returns, and the pipeline step after
  it publishes.
- **`worker LogRead on MessageRead`** — a separate, independently
  retried consumer (`max_retries: 2`) of that same stream. Nothing
  wires `MarkReadRoute` to `LogRead` directly; the stream is the only
  connection, and you could add a second, third, or tenth worker on
  `MessageRead` without touching the API handler at all.

## 2. Check, build, verify

<!-- ciac-verify:start id=check-build-verify -->
```sh
ciac check main.ciac
ciac build main.ciac --target python --out ./build
ciac verify main.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

Note what `use { queue NATS; }` changed in the build output: a
`start workers/jobs with ...` line. Generated services and generated
workers are separate runnable entry points from the same build — see
[docs/dev-loop.md](../dev-loop.md) for running both together.

## Checkpoint

A green `ciac verify` on a service using three capability families
together — `db` (both `crud`'s own table and the hand-declared
`ReadReceipts`), `queue` (the stream and its worker), and a
`transaction` — verified, not just parsed. This is also, structurally,
the same shape as the compact example the top-level
[README](../../README.md) walks through end to end, including the
failure-injected simulation this guide's own checkpoint doesn't
attempt — that's a later guide's job, not this one's.

From here: the rest of this series goes deeper on jobs and channels,
puts a real failure through the `transaction` above and proves the
rollback, and carries this same example into multi-service territory
and deployment. Those guides land alongside the rest of the series;
this one ends at its own real, working checkpoint either way.
