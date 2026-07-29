# Guide 5 — Simulation

*Reader: builder, continuing from [guide 4](04-streams-and-workers.md).
Time: ~20 minutes. You need: a terminal — this is the guide that
proves "no database, no broker, no Docker" isn't a slogan.*

Every guide so far has said `transaction` rolls back and `worker`
retries. This guide doesn't ask you to trust that — it injects a real
failure into the exact transaction from [guide 3](03-handlers-and-logic.md)
and shows you the two outcomes, deterministically, against real
generated code.

<!-- ciac-verify:start id=new -->
```sh
ciac new my-app --template minimal
cd my-app
```
<!-- ciac-verify:end -->

## 1. The program, unchanged from guide 4

<!-- ciac-verify:file id=main-ciac-v6 path=main.ciac -->
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

## 2. A scenario: given a failure, expect an outcome

A scenario is JSON: `given` sets up the world (here, one injected
failure), `steps` is a sequence of requests and assertions. This one
fails `MarkRead`'s own database commit exactly once, checks the
`ReadReceipt` never landed, retries the same request, and checks it
did the second time:

<!-- ciac-verify:file id=scenario path=sim/mark-read.ciac-sim.json -->
```json
{
  "simulation_version": 1,
  "name": "guide-05-mark-read",
  "start_at": "2030-01-01T00:00:00Z",
  "given": {
    "failures": [
      {
        "at": {"effect": "db.commit", "subject": "read_receipts", "occurrence": 1, "phase": "after"},
        "action": {"kind": "error"}
      }
    ]
  },
  "steps": [
    {
      "request": {
        "service": "Ping",
        "api": "MarkReadRoute",
        "json": {"id": "22222222-2222-4222-8222-222222222222", "text": "hi", "sent_at": "2030-01-01T00:00:00Z", "read": false},
        "save_as": "mark_failed"
      }
    },
    {
      "expect": {
        "row": {
          "service": "Ping",
          "table": "read_receipts",
          "where": {"message_id": "22222222-2222-4222-8222-222222222222"},
          "present": false
        }
      }
    },
    {
      "request": {
        "service": "Ping",
        "api": "MarkReadRoute",
        "json": {"id": "22222222-2222-4222-8222-222222222222", "text": "hi", "sent_at": "2030-01-01T00:00:00Z", "read": false},
        "save_as": "mark_retry"
      }
    },
    {"expect": {"response": {"of": "mark_retry", "status": 200}}},
    {
      "expect": {
        "row": {
          "service": "Ping",
          "table": "read_receipts",
          "where": {"message_id": "22222222-2222-4222-8222-222222222222"},
          "present": true
        }
      }
    }
  ]
}
```
<!-- ciac-verify:end -->

`"subject": "read_receipts"` is the table name `ReadReceipts` lowers
to (`ciac build` already showed you this shape in `app/models.py`
back in guide 3). The `where` clause in each `expect.row` matches the
same `message_id` both times, so the second assertion can only be
true if the retried request actually committed — a name collision
here would silently pass the first `expect` and fail the second, not
the other way around, which is why the scenario checks *absence*
before it checks presence.

## 3. Run it — no database, no broker, no Docker

<!-- ciac-verify:start id=sim -->
```sh
ciac sim main.ciac --target python --out ./build --scenario sim/mark-read.ciac-sim.json
# [PASS] guide-05-mark-read
```
<!-- ciac-verify:end -->

`ciac sim` builds the program (same as `ciac build`, same generated
code), then drives it against an in-process fake — no Postgres, no
NATS, nothing listening on a port. The failure it injected happens at
the exact effect (`db.commit`) and exact occurrence (`1`) the scenario
named; every other write in the system runs for real, in-process,
against the real generated handler code. Swap `--target python` for
any of the other four; the scenario file doesn't change, because the
scenario describes the *program's* behavior, not one target's.

`docs/simulation.md` covers the full scenario schema (`given.failures`'
other kinds — `Delay`/`Timeout`/`Lose`/`Duplicate`/`Disconnect` are
parsed but not yet actionable, disclosed there rather than silently
ignored here), virtual time for `job`s, and `ciac verify --sim` for
wiring a scenario into the same regeneration-drift check `ciac
verify` already runs.

## Checkpoint

A real rollback, then a real retry, then a real commit — proven, not
described, in milliseconds, with the exact same generated code
[guide 3](03-handlers-and-logic.md) told you to trust. `[PASS]
guide-05-mark-read` is the checkpoint; the assertion sequence above
*is* the proof.

From here: [guide 6](06-multi-service.md) puts a second service next
to `Ping` and simulates a call between them; [guide 7](07-deployment-and-day-two.md)
takes this same system to a real deploy.
