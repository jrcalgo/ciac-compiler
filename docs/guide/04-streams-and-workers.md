# Guide 4 — Streams, workers, jobs, and channels

*Reader: builder, continuing from [guide 3](03-handlers-and-logic.md).
Time: ~15 minutes. You need: a terminal — still no live broker, no
Docker; retry semantics and cron schedules are checked and generated
without either.*

Guide 3 gave `Ping` a stream (`MessageRead`) and a worker (`LogRead`)
consuming it. This guide adds the two capability families that round
out the ontology: a scheduled `job` (work with no request behind it)
and a `channel` (a live feed *to* a client, not a request from one).

<!-- ciac-verify:start id=new -->
```sh
ciac new my-app --template minimal
cd my-app
```
<!-- ciac-verify:end -->

## 1. A channel: pushing, not answering

Add `use { realtime live WebSocket; }` and one line —
`channel ReadFeed on MessageRead;` — to guide 3's final `main.ciac`:

<!-- ciac-verify:file id=main-ciac-v5 path=main.ciac -->
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

`channel ReadFeed on MessageRead;` is the third, independent consumer
of the same stream — alongside `LogRead` (a worker: retried,
at-least-once, no client attached) `ReadFeed` fans every
`MessageRead` event out to whatever clients are connected, over
WebSocket (or SSE — the provider named in `use`, not the channel
declaration, decides the transport). Nothing about `MarkReadRoute`'s
pipeline changed to add this; the stream is still the only thing
`publish` and its three consumers share.

## 2. A job: work with no request behind it

`job PruneReceipts { schedule: "0 3 * * *"; }` plus a handler bound
only to a capability (`db: default;`, no input/output types) is a
cron-scheduled task — real code (a container process on `--target
python`'s generated deploy; Kubernetes' own `CronJob` under
`--deploy k8s`), not a comment describing intent. `docs/language.md`
covers the full `schedule` grammar (standard 5-field cron); the
handler body here is a stub for you to fill in the same way any
generated handler is.

<!-- ciac-verify:start id=check-build-verify -->
```sh
ciac check main.ciac
ciac build main.ciac --target python --out ./build
ciac verify main.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

## Checkpoint

A green `ciac verify` on a service where one stream now has three
independent consumers (a worker, a channel, and — via
`MarkReadRoute`'s own pipeline — the publisher itself never blocks on
any of them), plus a scheduled job with no HTTP request behind it at
all. `docs/dev-loop.md` covers running the API, the worker process,
and the job scheduler together locally; `docs/deployment.md` covers
what each becomes in a real deploy.

From here: [guide 5](05-simulation.md) puts a real, injected failure
through the `transaction` in `MarkRead` and proves the retry/rollback
behavior this guide only described.
