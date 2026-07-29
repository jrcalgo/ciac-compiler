# Guide 7 — Deployment and day two

*Reader: builder, continuing from [guide 6](06-multi-service.md).
Time: ~20 minutes. You need: a terminal for everything except the
one step marked Docker-required.*

Every guide so far ended at `ciac verify`. This guide covers what
comes after: real deploy artifacts, the compose-backed system check
those artifacts describe, and the two tools for changing a system
that's already shipped — rename and the backfill ladder — without
breaking whoever's already consuming it.

<!-- ciac-verify:start id=new -->
```sh
ciac new my-app --template minimal
cd my-app
```
<!-- ciac-verify:end -->

## 1. Deploy artifacts

Starting from guide 5's `main.ciac` (the full `Ping` service —
`crud`, a `transaction`-wrapped handler, a stream, a worker, a
channel, a job):

<!-- ciac-verify:file id=main-ciac-v7 path=main.ciac -->
```text
service Ping;

use {
    db Postgres;
    queue NATS;
    scheduler jobs Cron;
    realtime live WebSocket;
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

channel ReadFeed on MessageRead;

job PruneReceipts {
    schedule: "0 3 * * *";
}

handler PruneOldReceipts {
    db: default;
}

pipeline PruneReceipts:
    PruneOldReceipts;
```
<!-- ciac-verify:end -->

`--deploy` is repeatable — request as many artifact kinds as you need
in one build:

<!-- ciac-verify:start id=build-deploy -->
```sh
ciac build main.ciac --target python --out ./build --deploy k8s --deploy terraform --deploy ci
```
<!-- ciac-verify:end -->

`k8s/` gets a Deployment + Service per capability that needs one;
`terraform/` gets the infrastructure a `docker-compose.yml` can't
express (the stateful stuff — see [docs/deployment.md](../deployment.md)
for exactly where that line is drawn); `.github/workflows/ci.yml` in
the generated project mirrors `ciac verify` itself, so the generated
system's own CI can never drift from the one command you've been
running all series. Nothing here is a template you'll need to hand-
edit before it's usable — read one of the three the same way guide 1
had you read generated application code.

## 2. The compose-backed system check (Docker required)

<!-- ciac-verify:skip id=system reason="requires Docker; covered instead by CI's own generated-system job" -->
```sh
ciac verify main.ciac --target python --out ./build --system
```
<!-- ciac-verify:end -->

This is the one command in the whole series that needs a real
Postgres, a real NATS, and a real running service — `ciac verify`
without `--system` (every prior guide's own checkpoint) proves
regeneration correctness and the generated code's own tests; `--system`
proves the thing you'd actually deploy handles a real request over a
real network. [docs/deployment.md](../deployment.md) covers what it
boots and what it asserts.

## 3. Changing a system that's already shipped: rename

A field or record name picked in guide 1 doesn't have to be permanent.
`ciac rename` is a whole-program, multi-file symbol rename — not a
text search-and-replace, a resolution-based one that knows a
`Message` field reference from an unrelated identifier that happens
to share the name:

<!-- ciac-verify:start id=rename -->
```sh
ciac rename main.ciac --file main.ciac --line 10 --column 8 --to PingMessage
```
<!-- ciac-verify:end -->

Dry-run by default — it reports every site it would touch (nine, in
this file) without writing anything; add `--apply` to actually rewrite
them, and `--out DIR` to replay the same rename through an already-
generated project's own regeneration path instead of a fresh build.
[docs/evolution.md](../evolution.md) covers the full mechanism
(the same source index that also backs the LSP's own rename), the
`--out` replay's transactional guarantee, and ambiguous-name
resolution (`--line`/`--column` above exists for exactly the case
this file hits: `Message` names both a record and a `crud` binding).

## 4. Changing a system that's already shipped: the backfill ladder

A rename never changes what a value *means* — for that,
[docs/evolution.md](../evolution.md)'s expand → backfill → contract
ladder is the tool: `ciac backfill plan` walks a breaking storage
change (splitting a field, changing a type, merging two tables)
through the same three-phase migration a human would hand-write, with
each step's own generated code and a human-completed seed for the one
step that's inherently manual (the actual backfill logic). Not
repeated here — evolution.md's own worked example is the complete,
checked reference; this guide's job is knowing the tool exists and
when to reach for it (a breaking storage change, not a rename).

## Checkpoint

Real k8s/Terraform/CI artifacts generated and read, not just listed;
a real whole-program rename previewed against this series' own final
`main.ciac`; the compose-backed system check named and disclosed as
the one Docker-required step in the entire series, not silently
skipped. This is also the series' own last checkpoint: `Ping` started
as one echo endpoint in guide 1 and ends here with real persistence,
a transaction, a stream with three independent consumers, a scheduled
job, and a deploy path — the same one file, seven guides, no
step where the ground shifted under you.
