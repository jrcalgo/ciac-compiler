# Guide 6 — Multi-service systems

*Reader: builder, continuing from [guide 5](05-simulation.md).
Time: ~20 minutes. You need: a terminal for the `check`/`build`/
`verify`/`sim` steps; Docker only for the one optional step that says
so.*

Every guide so far built one service. This guide is the one place
in the series that deliberately steps away from your own `Ping`
service to use a different, already-checked-in example —
[`examples/multi-service/sim-three-service.ciac`](../../examples/multi-service/sim-three-service.ciac) —
because a multi-service topology needs enough services to be worth
simulating (two can't distinguish "ordered" from "happened"; three
can), and this repository already has one, proven, in CI, since
28UpdatePlan.md's own arc built it. Reusing it beats inventing a
fourth service just for this guide.

## 1. `project`: one file, several services

```text
project ThreeService;

record Order { id: Uuid; total: Float; }

stream OrderAccepted: Order;

service Intake {
    use { queue NATS; }
    api SubmitOrder: Order { method: POST; path: "/orders"; }
    pipeline SubmitOrder:
        call Billing.Charge
        -> publish OrderAccepted
        -> Return;
}

service Billing {
    use { db Postgres; }
    // ... a Charge api that records a charge and returns
}

service Fulfillment {
    use { db Postgres; queue NATS; }
    worker Ship on OrderAccepted;
    // ... records a shipment when an order is accepted
}
```

(Trimmed for the page — the full file is the real one linked above.)
`project` instead of `service` at the top is the only new keyword:
everything inside is ordinary `service` blocks, and a record/stream
declared outside any one `service` block (`Order`, `OrderAccepted`
here) is shared vocabulary every service in the file can reference.
`call Billing.Charge` is a checked, typed cross-service call —
`ciac check` rejects it if `Billing` doesn't exist or `Charge` doesn't
accept an `Order` — compiled into a real typed HTTP client
(`app/clients/billing.py`, or the equivalent for any other target),
not a string URL you assemble by hand.

<!-- ciac-verify:start id=check -->
```sh
ciac check examples/multi-service/sim-three-service.ciac
```
<!-- ciac-verify:end -->

## 2. Building a multi-service program

<!-- ciac-verify:start id=build -->
```sh
ciac build examples/multi-service/sim-three-service.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

The output is a *system of deployables*: `intake/`, `billing/`, and
`fulfillment/` are each a complete, standalone project (exactly the
same shape a single-service `ciac build` produces), plus one root
`docker-compose.yml` wiring them together — one shared broker for the
shared stream, per-service databases, container DNS for the `call`.
Nothing about any one service's own generated code looks different
from what guides 1–5 already showed you; the composition lives in the
one file at the top.

## 3. Simulating the whole system — still no Docker

<!-- ciac-verify:start id=sim -->
```sh
ciac sim examples/multi-service/sim-three-service.ciac --target python --out ./build \
    --scenario sim/sim-three-service.ciac-sim.json
# [PASS] 28-m4-three-service-n3-global-ordering-and-call-seam-failure
```
<!-- ciac-verify:end -->

One shared in-process world backs all three services at once: the
`call Billing.Charge` is routed in-sim (no real HTTP, no real
network), the stream hop from `Intake` to `Fulfillment` is globally
ordered against the same virtual clock, and the checked-in scenario
injects a failure into the *call itself* (the second `Charge`
attempt errors) to prove `Fulfillment` never ships an order whose
charge failed — a claim that only means something once you have three
independently-owned tables to check across a real cross-service call
and a real cross-service stream hop, together, in one assertion.
`docs/simulation.md`'s composition section covers the full topology
rules (which target restated this first, the process-model decision,
what's still refused).

<!-- ciac-verify:start id=verify -->
```sh
ciac verify examples/multi-service/sim-three-service.ciac --target python --out ./build
```
<!-- ciac-verify:end -->

## 4. The real thing (Docker required)

<!-- ciac-verify:skip id=system reason="requires Docker; the compose-backed system suite (call reachability, broker delivery, per-service capability round-trips) is covered by the generated-system CI job instead" -->
```sh
ciac verify examples/multi-service/sim-three-service.ciac --target python --out ./build --system
```
<!-- ciac-verify:end -->

`--system` boots the real compose stack (Postgres ×2, NATS, all three
services) and runs the same claim the simulation just proved, against
real containers this time. [docs/deployment.md](../deployment.md)
covers the full compose/Kubernetes/Terraform path.

## Checkpoint

A real three-service system — `ciac check`, `ciac build`, `ciac sim`
(a cross-service call, a cross-service stream, and a failure injected
into the seam between two services, all in one scenario), and
`ciac verify` — all green against a file this repository has run
through CI since it was written. `--system` (Docker) is the one step
you'd run yourself to see it for real; the harness that checks this
guide counts it as a disclosed skip rather than pretending it doesn't
exist.

From here: your own `Ping` service could become one member of a
`project` the same way — nothing about `project`/`call`/shared
streams is special to this example, only the topology being large
enough to demonstrate them. [Guide 7](07-deployment-and-day-two.md)
takes a system like this one to a real deploy and covers what comes
after the first release.
