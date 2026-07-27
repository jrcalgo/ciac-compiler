# CIaC v0.28-file — Multi-Service Simulation: One World, N Services (implementation plan)

> Implementation plan. Document number ≠ release number (standing
> precedent; expected to ship as **0.26.0**). Assumes 26UpdatePlan.md
> and 27UpdatePlan.md shipped: five targets at full simulation
> depth, the corpus-identity harness running everything everywhere,
> the replay flag decoupled and honest, the ledger's Open table
> down to the rows this arc and its successors own. This arc
> removes the last structural refusal in the simulation surface:
> the per-target drivers' single-service bail. The decision made in
> the planning discussion was explicit and ambitious: **full
> N-service coordination**, not a two-service special case — the
> scenario schema has been service-qualified since v0.17 in
> anticipation, `SimPlan` has carried an unconsulted
> `multi_service` flag just as long, and the multi-service examples
> this arc must satisfy are the project's own deployment flagships.
>
> **Parity contract:** every checked-in multi-service example
> (`multi-service-media.ciac`, `inventory-system.ciac`, plus a new
> three-service example this arc adds so N>2 is proven, not
> presumed) simulates on **all five targets** with identical exact
> outcomes under the 27 corpus discipline; one shared world per
> simulation — single broker, single virtual clock, per-service
> database namespaces — so cross-service delivery and time are
> globally ordered and deterministic; typed cross-service call
> clients routed in-world (no sockets, no ports, zero
> infrastructure — the claim boundary `ciac sim` has always drawn,
> now system-wide); the single-service bail deleted from all five
> drivers and replaced by real topology handling; scenario service
> addressing validated with its own diagnostics; and the
> simulation story's last "except" — "except multi-service
> systems" — retired from the documentation.
>
> **Confidence:** high on the world side — the shared world's
> system-scoping (namespacing, routing, cross-service delivery
> order) is a bounded extension of machinery 27 just built and
> unit-tested, designed once in `ciac-sim` and inherited by Rust
> through the standing vendoring lever. Deliberately medium on the
> arc's one genuinely novel engineering problem: **composition** —
> how N generated services execute inside one deterministic
> process per target. Python composes trivially (import N apps);
> the compiled targets need their generated projects consumable as
> libraries by a generated system runner, which is a
> project-shape question with real per-target sharp edges (crate
> lib targets, Go module replaces, N Spring contexts in one JVM).
> The plan pre-registers the composition design per target, runs
> Python first as the pathfinder, and gates the compiled targets
> behind an M5 checkpoint that prices the lib-ification cost
> against measured reality before committing three more targets to
> it. The fallback ladder is written down (Pillar 3), ends in an
> honest partial ship, and — as with every checkpoint arc before
> it — is priced before the sunk cost exists, not after.

## The gap this version closes

Every real system with more than one service cannot use `ciac sim`
at all today. The refusal is not a capability gate that better
worlds could shrink — 27 shrank those to nothing — it is five
copies of the same hard bail (`ciac sim only supports a
single-service … project`) in the per-target drivers, checked
against the count of generated project directories before a
scenario is ever read. The irony the punch-list review named: the
language's proudest structural feature is multi-service systems —
`project` declarations, per-service deployables, cross-service
streams with consumer-aware evolution, typed call clients,
generated compose/k8s topology — and the moment a user exercises
it, the fastest test loop in the toolchain disappears. The
examples that show CIaC at its best (`multi-service-media`,
`inventory-system`) are exactly the examples `ciac sim` refuses.
Single services get deterministic virtual-time testing; systems —
the things that actually need it, where the bugs live in the
seams between services — get Docker or nothing.

The user story the punch-list phrased abstractly, concretely: a
team builds two services with a stream between them. Yesterday
(one service) their inner loop was `ciac sim` — milliseconds,
deterministic, failure-injected. Today (two services) it is
`docker compose up` and a prayer, because the second service's
existence — not its content — flipped the tool off. The cliff is
at exactly the wrong place: the two-service moment is when
cross-service ordering, retry across a boundary, and "what if the
downstream call fails on the third attempt" become the questions
that matter, and those are precisely the questions deterministic
simulation answers better than a live stack ever will.

The machinery has been waiting for this arc for a while. The
scenario schema speaks service names (`RequestStep.service`,
`expect.row`'s `service` field) since v0.17; `SimPlan` carries
`multi_service: bool` (plan.rs:41) that nothing consults; the
typed call clients built in the parity arcs' M7 milestones are
precisely the seams cross-service simulation needs to fake; and
27's shared world, delivery-order specification, and ×5 identity
harness are the substrate a system-scoped world extends rather
than invents. What was missing was never the design surface — it
was the budget for the composition problem, and this arc is that
budget.

What this arc does not attempt: distributed-systems *failure
modeling* beyond what the failure engine already does (no network
partitions, no partial delivery between services, no
per-service clock skew — the shared-world model deliberately makes
cross-service communication reliable and time global, because the
simulation's claim is business-outcome correctness under
deterministic adversity, not chaos engineering). The Explicit cuts
section holds that line; the fidelity-boundary docs state it.

## Pillar 1 — The topology contract: what a system means to the simulator

### What exists

A `project` program lowers to N per-service deployables; the
generated output directory holds N project directories (the
`find_project_dirs` walk the drivers already use to count — and
bail). `SimPlan` is derived from the same `NormalizedIr` and
already sees the whole system; its `multi_service` flag is set and
ignored. Cross-service edges exist in the IR (streams consumed
across service boundaries — the evolution machinery's
consumer-aware diffing depends on them; call edges via the typed
call clients). The scenario schema addresses services by name in
requests and row expectations but has never had those names
validated against a real multi-service plan (deferred "M5"
name-resolution work from v0.17 that single-service usage never
forced).

### The contract this arc fixes

M1 turns the implicit topology into specified simulation
semantics, all recorded in docs/simulation.md's scenario
reference:

- **Service addressing.** Every `request` step's `service` must
  name a service in the plan (new diagnostic, reserved SIM0011:
  unknown service, with the known-service list in the message —
  the SIM-code discipline of specific, actionable refusals);
  single-service programs keep the existing behavior where the
  field is optional and defaulted.
- **Database namespacing.** Each service's tables live in that
  service's namespace — two services may each have an `orders`
  table without collision, matching production reality (each
  service gets its own database/schema in generated compose).
  `given.db` and `expect.row` gain/require the `service`
  qualifier in multi-service scenarios (additive schema change,
  same discipline as 27's additions; `expect.row` already has the
  field). A cross-service row assertion is just a row assertion
  with the right service name — there is no cross-service SQL,
  because there is none in production either.
- **Broker scope.** One broker, system-wide — matching production
  (the generated compose runs one NATS/Kafka for the system).
  Subjects/streams keep their existing global names; consumer
  groups are already service-qualified by construction (the
  generated group names embed the service). Cross-service
  streams therefore work with zero new mechanism — the log and
  cursors are 27's, the groups just happen to belong to
  different services.
- **Clock scope.** One virtual clock, system-wide. `advance`
  advances the system; every service's schedules fire in the
  27-specified (due-time, then declaration order) order, with
  service declaration order as the outer tiebreak. Deterministic
  cross-service time is the entire reason the shared-world model
  was chosen (Pillar 2).
- **Call edges.** A typed call from service A to service B's API
  is, under simulation, a routed in-world invocation of B's
  handler (Pillar 4) — synchronous, deterministic, no transport.
  The plan records call edges so the router can validate them
  (a call to a service the plan doesn't know is a plan bug, not
  a runtime surprise).
- **Outcome scoping.** The one-line outcome JSON's business
  counts become service-qualified keys (`"media.ProcessUpload":
  4` rather than bare handler names) in multi-service runs —
  additive for single-service (bare names remain), exact-outcome
  discipline unchanged, canonicalization rules from 27 apply.

### The plan's derived facts, enumerated

What `SimPlan` must carry for the contract above, and where each
fact comes from in `NormalizedIr` (M1 verifies which already
exist — the plan was built system-aware in v0.17, so several
should — and adds the rest additively):

| Fact | Derived from | Consumed by |
| --- | --- | --- |
| Service list in declaration order | the project's service nodes | scheduler tiebreak, registration order, outcome qualification |
| Table → owning service | each service's db/table declarations | world namespacing, `given.db`/`expect.row` validation |
| Stream → publisher/consumer services + group names | stream nodes + consumer edges (the evolution machinery's own edge set) | cross-service delivery, group registration |
| Call edges (A → B.api) | typed call-client usage in handler HIR | router validation, acyclicity check (SIM0012) |
| API → service | api/crud nodes | `request.service` validation (SIM0011), router registry |
| Schedule entries with owning service | worker/job nodes | the (due-time, declaration, service) firing order |

The acyclicity check runs at plan derivation (a `ciac sim`-time
refusal with the cycle spelled out), not at scenario runtime —
topology problems are program problems and get diagnosed before
any scenario executes.

## Pillar 2 — One world, N services

The architecture decision that everything else hangs from, made
here rather than discovered mid-arc: the simulation holds **one
`SimWorld` for the whole system**, not N per-service worlds with
a coordination layer. The reasons, in order of weight:

1. **Determinism is a global property.** Cross-service delivery
   order and virtual time cannot be locally owned — N worlds
   with N clocks and N logs would need a coordinator that
   re-serializes them into exactly what a single world gives for
   free, and every coordinator is a place where two targets
   could serialize differently and break ×5 identity.
2. **Production topology agrees.** The generated system runs one
   broker; services share it. Databases are per-service; the
   world's namespacing mirrors that. The world's shape following
   production's shape is the fidelity argument, not just the
   convenience one.
3. **27 already built it.** The deep world's cursor log is
   already multi-group; groups already encode services; the
   clock/scheduler is already system-wide in spirit (nothing in
   it is service-scoped). System scope is namespacing plus
   routing, not new machinery.

The concrete `ciac-sim` changes (M2): tables keyed by
`(service, table)` instead of bare table name (with the
single-service degenerate case preserved — bare keys when the
plan has one service, so 27's corpus and goldens are untouched);
the scheduler's entries carrying their service for the outer
tiebreak; a call-router registry (`register_api(service, api,
handler)` / `call(service, api, request) -> response`) that the
composed runners populate at startup and the call-client guards
invoke; and `SimPlan` growing the call-edge and
service-order facts the router and scheduler need. Unit-tested
in-crate like every 27 fake, inherited by Rust via vendoring as
always.

### The world's surface additions, as drafted

The method deltas on `SimWorld` (signatures translated per
restatement as in 27; effect names in parentheses):

```text
// namespacing: every relational method gains the service key,
// with the single-service degenerate form preserved:
db_insert_checked(service, table, row)        (db.commit)
//  ...same for update/delete/get/count/find_where/seed_db —
//  27's signatures, one leading parameter wider; bare-key
//  compatibility path when plan.services.len() == 1

// call routing (new):
register_api(service, api, handler)              [runner-only]
call_checked(caller, callee_service, api, req)  (call.request)
//  inline-synchronous; depth-guarded; returns the callee's
//  typed response or error envelope verbatim

// registration bookkeeping (new, runner-only):
register_consumer(service, subject, group, handler)
register_schedule(service, entry)
```

`call_checked`'s effect name (`call.request`) enters the failure
vocabulary's reachable set — occurrence-counted like every other
effect, so "fail the third cross-service call" is one `failures`
rule. Broker and peripheral methods are unchanged (subjects are
global; peripheral instances are service-owned already by their
names — M1 confirms no peripheral collision case exists across
the corpus and records the finding).

## Pillar 3 — The composition problem: N services, one process

### The decision space, priced before the arc commits

A deterministic simulation wants a single OS process: one world,
one memory space, no IPC in the outcome path. The alternative —
N runner processes with the world behind a socket/pipe protocol —
was considered and is rejected as the primary design for three
recorded reasons: the world's every method call becomes a
serialization boundary (performance is survivable; the
*determinism bookkeeping* — global delivery order across N
process-local schedulers — is the coordinator problem from
Pillar 2 wearing a different hat); failure modes multiply (N
process lifetimes, partial startup, orphaned children) in exactly
the tool whose pitch is "no infrastructure, no flakes"; and the
one-line-stdout child protocol the CLI drivers speak would need a
multiplexing redesign. It remains the recorded fallback if
single-process composition hits a structural wall on some target
(the M5 checkpoint's no-go branch), because it *is* always
possible — just worse.

So the primary design: **a generated system runner per target** —
one more generated artifact, emitted at sim time like the
existing runners, that links/imports all N services' handler and
wiring code into one process, builds one world, registers every
service's routes/consumers/jobs with it, and executes scenarios
under the same child protocol as today. Per target, the known
sharp edges and the intended shape:

- **Python (M3, the pathfinder):** the pyrunner already
  constructs a service's app around the world; the system runner
  constructs N. The sharp edge is import identity — N generated
  projects with identically named top-level modules cannot be
  naively imported into one interpreter. Intended shape:
  pyrunner's system driver loads each service's modules under a
  package alias (importlib machinery it already half-has for
  single-service loading), keeping generated code untouched.
  If aliasing fights something (module-level state, relative
  imports), the fallback is a sim-only `__init__` shim emitted
  per service — generated-code-visible but sim-scoped.
- **Rust (M6):** generated projects are binary crates. The system
  runner needs them as libraries — the intended shape is the
  standing lib+bin split (each generated project gains a
  `lib.rs` re-exporting what `main.rs` and `sim_runner.rs`
  already share; the sim workspace's system-runner crate depends
  on the N service crates by path). This is the arc's most
  golden-visible change on any target: every Rust project's
  crate shape shifts (uniformly, mechanically, and arguably as a
  standalone improvement — bins that are thin shells over libs
  is idiomatic). Priced at M5 against Python's measured
  experience before it is attempted.
- **TypeScript (M7):** N package directories in one node
  process — imports are path-based and collision-free by
  construction; the system runner is a sim-only entry module
  importing N `app` factories. Expected to be the cheapest
  compiled-family port; its sharp edge is dependency-version
  skew across the N generated `package.json`s (identical by
  construction today — asserted, and the runner installs once at
  the system root).
- **Go (M7):** N modules; the system runner module uses
  `replace` directives to the N service module paths (the
  mechanism the repo's own external-backend tooling already
  exercises). Generated packages are import-path-distinct by
  service name already. Sharp edge: none expected beyond build
  wiring — Go's explicitness is for once the easy case.
- **Java (M8):** one JVM, N services — the intended shape is N
  isolated `AnnotationConfigApplicationContext`s (the
  construction 25's SimRunner already uses per service, N-fold),
  each scanning only its service's packages, sharing the one
  world bean by explicit registration. Spring context isolation
  is designed-for; the sharp edge is classpath assembly (N
  generated Maven projects on one test classpath — the system
  runner is a sim-only aggregator POM with N module
  dependencies, or the exec-plugin arrangement generalized;
  decided at M8 against 25's packaging decisions, recorded).

### The composition matrix, summarized

The per-target analysis above, compressed to the table M5's
checkpoint re-prices:

| Target | Mechanism | Sharp edge | Fallback |
| --- | --- | --- | --- |
| Python | import N apps under package aliases | module identity/import collisions | sim-only per-service shim |
| Rust | lib+bin split; system-runner crate with N path deps | biggest golden churn in the arc | pre-ship the reshaping as its own reviewed step (M5 decides) |
| TypeScript | sim entry module importing N app factories | dependency-version skew across N package.jsons (identical by construction — asserted) | none expected |
| Go | system-runner module with `replace` directives | build wiring only | none expected |
| Java | N isolated Spring contexts, one JVM, shared world bean | classpath assembly across N Maven projects | aggregator POM vs generalized exec arrangement (M8 decides) |

### The child protocol at system scope

Unchanged, deliberately: one process, scenarios in, **one JSON
outcome line on stdout** — the contract every driver already
speaks. What changes is only what the runner does before the
first scenario: construct the world from the plan, then for each
service *in declaration order*, register its APIs, consumers
(with their group names), and schedule entries. Registration
order is contract (it feeds the delivery spec's tiebreaks), which
is why it is stated here and asserted by the N=3 scenario's
frozen outcomes rather than left to whatever iteration order a
target's map type fancies — the exact class of accident 27's
canonicalization rules exist to catch.

### The rejected N-process design, sketched for the record

Because M5's fallback branch may need it, the shape it would
take is written down now rather than designed under pressure: the
CLI process hosts the world behind a line-oriented local protocol
(the child-protocol idiom, inverted); each service runner
registers over it at startup and blocks on a `step` instruction;
the CLI owns the delivery loop and doles out single-handler
executions one at a time, preserving 27's ordering by
construction (determinism lives in the only place that sees
everything). Every world method becomes a request/response pair;
scenario wall-time grows by the round-trip count; the process-
lifecycle handling (startup barriers, teardown, crash surfacing
through the outcome line) is the bulk of the new code. It works,
it is uniform across targets, and it is strictly more machinery
executing strictly slower — which is why it is the fallback. If
a target ever takes it, the protocol is specified once in
ciac-sim (a `world-proto` module with fixture tests) rather than
per target, and the outcome contract is unchanged — ×5 identity
would not know which composition shape produced a line, which is
the point of having outcome contracts.

### The uniformity rule

Whatever shape each target's composition takes, the *observable
contract* is uniform: same scenario files, same child protocol,
same outcome JSON, same ×5 identity bar. Composition is allowed
to differ per target (it must — it is made of project-shape
idiom); outcomes are not. And one process-shape rule is
non-negotiable across all five: scenario execution remains
single-threaded through the world (27's delivery loop), no
matter how many services are loaded — parallelism is a
performance feature simulations do not want.

## Pillar 4 — Call clients and cross-service seams

The typed call clients (arcs 23–25 M7: service A's generated
client for service B's API, used by handlers via `call`
expressions) are the transport of cross-service systems, and
under simulation they become the world-guard with the highest
leverage in the arc: guard the client's request method — world
present: route through the world's call router to service B's
registered handler, synchronously, returning B's typed response
(including B's error envelope on failure — the client's
production error mapping runs unchanged); world absent: the real
HTTP path, byte-identical to today. `http.request` failure
injection applies at the seam (`call` effect name reserved in
M1), so a scenario can fail the third cross-service call — the
cross-service adversity case single-service sim could never
express, and the first genuinely *new* testing capability this
arc hands users (everything else is "what worked for one service
now works for N").

Two semantic decisions fixed now: routed calls execute
**inline** on the caller's logical thread (synchronous call
semantics match production's request/response; the callee's own
publishes land in the shared log for the delivery loop, exactly
like any handler's); and a routed call that itself makes calls
recurses through the router with a depth guard (the plan's call
edges are checked acyclic for the simulated path at M1 —
production may tolerate cycles via timeouts; simulation refuses
them with a specific diagnostic, reserved SIM0012, because a
cycle under synchronous inline semantics is a hang).

The guard's emitted shape, illustrated on two targets (the other
three follow their own guard idioms per 27's inventory):

```text
// Rust — inside the generated call client's request method:
if let Some(world) = &self.world {
    return world.call_checked("media", "transcode", "SubmitJob", req);
} else {
    // the real reqwest path, byte-identical to today
}

// TypeScript — same seam:
if (world) {
  return world.callChecked("media", "transcode", "SubmitJob", req);
}
// real fetch path unchanged below
```

The client's response typing does the same work it does in
production (decode-through-schema, 27's rule): the router hands
back the callee's response value, and the caller's generated
decoding validates it — a type-level tripwire against router
bugs on every routed call.

Channels/realtime cross service boundaries the same way they
work today in-service (the channel machinery is
broker-backed where it is broker-backed, and the shared log
covers it); anything channel-specific that proves service-local
in a target's generated wiring is a finding for the relevant
milestone, recorded against the uniformity rule.

### Identity propagation through routed calls

One semantic gap the routing surface exposes and this arc must
answer rather than inherit by accident: when service A handles a
request from principal P and calls service B, who is B's caller?
The answer follows production: whatever the generated call
clients do today with auth context (forwarded bearer context vs
unauthenticated service-to-service calls — per-target reality
checked at M1 against the generated client code, since the call
clients were built before FakeAuth existed), the routed path does
the same. If production forwards, the router carries P's
principal into B's enforcement (and a scenario can assert a
cross-service 403 — a genuinely sharp new test); if production
does not forward, neither does the router, and the fidelity note
says so. What the simulator must not do is invent an identity
model production lacks — M1 records the per-target finding, the
router implements exactly it, and any *dissatisfaction* with
production's answer is a language/feature question for a future
arc, filed in the ledger, not smuggled into a fake.

## Pillar 5 — Scenario semantics and the system corpus

The corpus discipline extends to systems with the same rules
(exact outcomes, ×5 identity, canonical ordering) and new files:

| Scenario file (sim/) | Program | What it proves | Authored in |
| --- | --- | --- | --- |
| media-system.ciac-sim.json | multi-service-media.ciac | request → cross-service publish → worker in another service; per-service row assertions | M4 |
| inventory-system.ciac-sim.json | inventory-system.ciac | call-client round trip; cross-service failure injection on the call seam; scoped auth across services (27's FakeAuth at system scope) | M4 |
| three-service.ciac-sim.json | new sim-three-service.ciac | N=3: A calls B, B publishes, C consumes; global delivery order across three services; the N>2 proof | M4 |

The new example's topology, sketched (final program at M4;
deliberately minimal — topology showcase, not feature showcase):

```text
project three_service {
  service intake {          // A: receives requests
    api SubmitOrder { ... calls billing.Charge ...
                      publish OrderAccepted }
  }
  service billing {         // B: called synchronously by A
    api Charge { ... db.insert(charges, ...) ... }
  }
  service fulfillment {     // C: consumes A's stream
    worker on OrderAccepted { ... db.insert(shipments, ...) }
  }
}
```

and its scenario asserts, in one file: the routed A→B call's
effect (a `charges` row in billing's namespace), the
cross-service stream delivery (a `shipments` row in
fulfillment's), an injected `call.request` failure on the Nth
submit (error envelope at A, **no** charge row, **no** shipment —
the cross-service atomicity-of-refusal story told exactly), and
quiescence. Global delivery order across three services is what
freezes the outcome counts.

Like every corpus program, the example verifies ×5 as a normal
example. Single-service corpus files run unchanged
throughout the arc — the degenerate-case preservation in
Pillar 2 is what makes that sentence cheap. The two standing
canonical anchors remain byte-exact, as always, at every
milestone.

### A system scenario, illustrated

The media-system scenario's skeleton, showing every
service-qualified surface at once (counts frozen at M4):

```text
{
  "simulation_version": 1,
  "name": "media system: upload, transcode, notify",
  "start_at": "2027-01-01T00:00:00Z",
  "given": {
    "db": [
      { "service": "media", "table": "profiles",
        "rows": [ { "id": "u1", "plan": "pro" } ] }
    ],
    "failures": [
      { "effect": "call.request", "subject": "transcode.SubmitJob",
        "occurrence": 3, "action": { "kind": "error" } }
    ]
  },
  "steps": [
    { "request": { "service": "media", "api": "Upload",
                   "as": { "sub": "u1", "scopes": ["media:write"] },
                   "json": { ... } } },
    ... more uploads ...
    { "drain": {} },
    { "advance": { "by": "1h" } },
    { "drain": {} },
    { "expect": { "row": { "service": "transcode",
                            "table": "jobs",
                            "where": { "status": "done" },
                            "present": true } } },
    { "expect": { "worker_attempts": { "worker": "transcode.ProcessJob",
                                        "count": 4 } } },
    { "expect": { "quiescence": {} } }
  ]
}
```

Note what did *not* change: step kinds, the failure-rule shape,
the exact-count discipline — a system scenario is a scenario. The
qualified surfaces (`given.db[].service`, `request.service`,
worker names in `service.Worker` form, outcome keys likewise) are
the entire syntactic delta, which is the measure of how much of
this arc v0.17's schema design pre-paid.

Schema deltas (M1, additive per the 27 discipline):
`given.db` rows gain the `service` qualifier (required when the
plan is multi-service, rejected-with-SIM0011 when unknown);
`publish` steps gain optional `service` disambiguation only if
M1's plan-resolution work finds stream-name ambiguity is
representable (streams are globally named today — expected
answer: not needed, recorded either way); outcome keys
service-qualify in multi-service runs (Pillar 1). Version
decision inherits 27's Open-question-1 resolution.

## Pillar 6 — Drivers, protocol, and CI

The five `sim_drive_*` functions in commands.rs lose their
project-count bail and gain topology handling: enumerate the N
project directories, map them to plan services (directory-name ↔
service-name mapping validated with a specific error when a
directory is missing or extra — generated-output drift is a
regeneration problem and gets said plainly), emit/build the
system runner (per-target composition from Pillar 3), and run
scenarios through the unchanged child protocol (one process, one
JSON line). Build strategy per target follows the existing
single-service driver's shape, build once + run per scenario:
Python needs no build; Rust builds the system-runner crate once
(cargo builds the N path deps as a graph — incremental across
scenarios and runs); TS installs once at the system root and
runs the entry module; Go builds the runner module once
(`replace`-resolved); Java compiles once (`test-compile` on the
aggregator or the M8-decided arrangement) and execs per
scenario, the 25 shape N-fold. The compiled targets' system
builds are the arc's wall-clock cost center, and each driver
milestone records cold/warm system build times the way 25's
Pillar 8 recorded validate latency — data first, CI-scoping
decisions from data.

`--record`/`--replay`: multi-service record/replay is **out of
scope** (Explicit cuts) — the flag machinery from 27 already
distinguishes replay support; multi-service runs on Python
refuse record/replay with a specific message this arc adds
(replay's single-service Python support is unchanged). The
ledger row stays open with its scope widened ("record/replay:
single-service Python only").

CI: `generated-sim` gains the three system scenarios ×5 —
budgeted as the job's cost roughly doubling (the three-service
example is small; the build-once discipline contains it), with
the same honesty as every CI addition: measured in the
milestone, recorded, and narrowed deliberately rather than
quietly if the wall-clock demands it.

## Pillar 7 — Fidelity at system scope

The ratchet question for multi-service sim: does the simulated
system agree with the real one? The cheap, high-value rows: the
three system scenarios' *business outcomes* re-asserted against
the live compose stack via the existing `verify --system`
machinery (the system tests already probe capability round
trips; the ratchet rows assert the scenario's specific counts
through real HTTP against real services with real broker —
Docker-delegated per the standing honesty, runnable locally
where compose is available). Divergences land where they always
land: the fake is corrected, or the boundary is documented as a
fidelity note. The known-in-advance boundary this arc creates,
stated in docs from day one: in-world call routing is
synchronous and reliable; production cross-service HTTP is
neither under failure — the simulation models call *failure* via
injection (the seam's `error` action), not call *latency or
partial connectivity* (no Delay action — 27's cut, inherited).
That is the same class of disclosure the single-service fakes
have always carried, now stated at the seam where a distributed
system's hardest bugs live, so nobody mistakes `ciac sim` for a
partition-tolerance oracle.

## Pillar 8 — Wall-clock and the sim-latency budget

`ciac sim`'s value is proportional to its speed — a simulation
that takes as long as compose isn't an inner loop — and system
composition is the first change since v0.17 that could genuinely
threaten the latency story on the compiled targets. So the arc
carries an explicit budget discipline, modeled on 25's
validate-latency pillar:

- **The measured quantities**, recorded per target at each
  composition milestone and reconciled at M9: cold system build
  (first `ciac sim` on a fresh generation), warm system build
  (repeat run), and per-scenario execution time for the system
  corpus.
- **The working budgets** (predictions, not promises — the point
  is to notice, loudly, if reality disagrees): Python
  system-scenario execution within 2× its single-service times;
  compiled-target warm builds within ~1.5× their single-service
  sim builds (the N services were already built once — path
  deps/replaces/modules should make warm rebuilds incremental);
  cold builds are allowed to be honest (N services compile) and
  are reported, not hidden.
- **The levers if a budget blows**, in preference order: build-
  graph hygiene (ensure the system runner reuses the services'
  existing build artifacts rather than recompiling — the likely
  first finding on Rust and Java); per-target driver caching
  keyed on the generation manifest (the machinery `ciac diff`
  already understands); and only then CI scoping (Open question
  6) — the user-facing latency is the thing the levers protect,
  CI cost is the thing they may trade.
- **The reporting surface**: timings land in each milestone's
  Shipped note and, at M9, as a small table in
  docs/simulation.md's system section — users deciding whether
  to reach for sim-first workflows on a compiled target deserve
  the numbers, which is the same users-deserve-data reasoning
  behind 25's image-size disclosures.

## Implementation map

| Area | Changes |
| --- | --- |
| `crates/ciac-sim/src/world.rs` | (service, table) namespacing with single-service degenerate case; call-router registry; service-aware scheduler tiebreak |
| `crates/ciac-sim/src/plan.rs` | `multi_service` finally consulted; call edges + service order carried; acyclicity check for routed calls |
| `crates/ciac-sim/src/scenario.rs` | `given.db` service qualifier; service-name validation hooks; outcome-key qualification rules |
| `crates/ciac-sim/src/codes.rs` | SIM0011 (unknown service), SIM0012 (call cycle), the multi-service record/replay refusal message |
| `sim/pyrunner/` | system driver: N-service loading under package aliases; router population; outcome qualification |
| `crates/ciac-backend-rust` | lib+bin project shape (the arc's biggest golden churn); system-runner crate emission; call-client guard |
| `crates/ciac-backend-ts` | system entry module emission; call-client guard |
| `crates/ciac-backend-go` | system-runner module with replaces; call-client guard |
| `crates/ciac-backend-java` | N-context system runner (aggregator packaging per M8 decision); call-client guard |
| `crates/ciac/src/commands.rs` | five drivers: bail removed, topology handling, system-runner build/run, timing capture |
| `sim/` + `examples/` | three system scenarios + `sim-three-service.ciac` |
| `tests/` | system-corpus ×5 identity; namespacing/router unit tests ride ciac-sim |
| CI | `generated-sim` system rows; timings recorded |
| docs | simulation.md system section + fidelity note; backends.md ledger row closed; scenario reference updates |

## Capability parity checklist

| Surface | All five targets at M9 |
| --- | --- |
| Multi-service programs accepted by `ciac sim` | yes — bail deleted, topology handled |
| Shared world: one broker, one clock, namespaced DBs | yes, via ciac-sim (Rust vendored; three restatements extended) |
| Cross-service streams | delivered in the specified global order |
| Typed call clients | world-routed, failure-injectable, production branch byte-identical |
| Scenario service addressing | validated (SIM0011), documented |
| System corpus (3 scenarios) | identical outcomes ×5 |
| Single-service behavior | unchanged — degenerate case preserved, anchors byte-exact |
| Record/replay | single-service Python only, refusal specific, ledger row scoped |
| N>2 | proven by the three-service example, not asserted |

## Determinism and supply chain

No new dependencies on any target for any composition shape (the
system runners are generated code linking generated code; Go's
`replace` directives and Rust's path deps are toolchain
features, not packages) — asserted per milestone as in 27. The
single-threaded execution rule keeps determinism structural.
The lib+bin Rust reshaping is golden-visible everywhere and
behavior-neutral by construction (the bin delegates to the lib);
its review is exactly the 26-style invariant check: production
binaries byte-identical in behavior, `cargo` outputs identical,
only the file shape moves. Build wall-clock for composed systems
is the arc's honest cost; it is measured and recorded per
target, never hidden.

## Diagnostics, gating, and docs impact

New SIM codes: SIM0011 (scenario names unknown service — with
the known list), SIM0012 (routed-call cycle refused), plus the
scoped record/replay refusal message; all registered in
docs/simulation.md's code table. No CIAC-code changes; no
`SimSupport` changes (depth is done; this arc is topology). The
five drivers' refusal-message deletions are themselves
docs-visible: docs/simulation.md's "single-service only"
sentences (five of them — one per target's driver paragraph,
plus the per-target bail messages in commands.rs that quote the
same limitation with their version stamps, "v0.17"/"v0.23"/
"v0.24"/"v0.25") retire together, replaced by the system
section; backends.md's Open-table row "multi-service programs
refused" closes with proof at M9. The scenario reference gains
the service-addressing rules, the outcome-key qualification
convention, and the identity-propagation finding from Pillar 4 —
whatever production does, stated once, per target if they
differ.

## Relationship to the forecast documents

The second of the two simulation rows the punch-list forced and
the discussion resolved ambitiously ("full N-service
coordination"). Sequenced after depth (27) for the recorded
reason: composition orchestrates *worlds*, and orchestrating
full worlds once beats orchestrating narrow ones and re-plumbing
after. Consumes 27's delivery-order spec, identity harness, and
corpus discipline wholesale; consumes 26's ledger as the
scoreboard it closes its row on. Hands 29UpdatePlan.md a
simulation story with no structural refusals left — the front
door describes a tool that simulates systems, because it will
actually do that.

## What this arc is predicted to cost

Predictions, reconciled at M5 (composition) and M9
(retrospective):

| Workstream | Predicted size |
| --- | --- |
| Plan/world system scope (M1–M2) | modest: namespacing is a key-type change with a compatibility path; the router is a registry + dispatch; the validation set is the largest new *code* but smallest risk |
| Python composition (M3) | the pathfinder's cost is mostly the import-identity fight; the driver rewrite is mechanical |
| Rust composition (M6) | the arc's peak: lib+bin reshaping touches every Rust golden; the system-runner crate itself is small |
| TS/Go compositions (M7) | cheap by analysis — the milestone exists to prove it and record the timings |
| Java composition (M8) | mid-sized; the cost is packaging decisions, not code |
| Corpus + example (M4) | three scenarios + one deliberately thin example verifying ×5 |
| Drivers (M3, M6–M8) | five bail deletions + topology handling on a shared shape |

### Predicted golden churn

| Milestone | Expected churn |
| --- | --- |
| M1–M2 | none in generated projects (plan/schema/crate) |
| M3 | Python: driver-side only if aliasing wins; per-service shim files if the fallback is taken (recorded either way) |
| M4 | the new example's goldens ×5 |
| M6 | every Rust example (lib+bin reshaping — the arc's review center of gravity) + sim-only system-runner files on multi-service examples |
| M7 | sim-only system entry/module files on multi-service examples, TS + Go |
| M8 | sim-only aggregator/runner files, Java |
| M9 | docs + version churn only |

### The config/env surface

None — same sentence, same enforcement as 27: composition is
build-shape, the world reads scenario data, and no generated
system gains an environment variable or config row from this
arc. Asserted per milestone.

## Milestones

Nine milestones: contract, world, then Python end-to-end as the
pathfinder (M3–M4), the checkpoint that prices compiled-target
composition (M5), the three compiled compositions (M6–M8), and
the ×5 close (M9). Standing per-milestone discipline throughout
(full verification, golden review, canonical anchors, commit +
push, in-place Shipped notes).

1. **M1 — The topology contract.** Pillar 1 executed: service
   addressing validation against the plan (SIM0011), namespacing
   rules, clock/scheduler tiebreak order, call-edge recording +
   acyclicity check (SIM0012), outcome-key qualification,
   `given.db` service qualifier (additive schema), the
   composition decision matrix recorded per target with the
   process-shape rule fixed. `SimPlan.multi_service` consulted
   for the first time — by validation, before any driver
   accepts. docs/simulation.md's scenario-reference updates
   drafted alongside (docs move with schema, the standing rule).

   **Shipped (v0.28 M1) — a course correction, not a straight
   execution.** `SimPlan` (`crates/ciac-sim/src/plan.rs`) grew
   `apis: Vec<SimApi>` and `call_edges: Vec<SimCallEdge>`, derived
   from `ir.nodes_of_kind(NodeKind::Api)` and
   `ir.edges().filter(|e| e.kind == EdgeKind::ServiceCall)` — both
   already existed in the IR (`ciac-sema::build::wire_steps` wires
   a `ServiceCall` edge for every `Call` step; no new IR modeling
   was needed, only consuming what was already there), sorted by
   stable key like every other `Sim*` collection so `plan_hash`
   stays architecture-order-independent. **SIM0012 (routed-call
   cycle) was reserved, implemented, then removed before
   shipping.** Investigation before wiring it in found
   `ciac-sema`'s existing `CycleDetection` pass
   (`crates/ciac-sema/src/passes/cycles.rs`) already treats
   `EdgeKind::ServiceCall` as a flow edge in its combined request-
   flow/message/call/dependency cycle check, run on every `ciac
   check`/`build`/`sim` invocation via `front_end` ->
   `ciac_sema::analyze` — so a program with a call cycle already
   fails compilation with `CyclicDependency` (`CIAC*`, not `SIM*`)
   before a `SimPlan` can exist at all. A sim-layer
   `check_acyclic()` (built, unit-tested, then deleted along with
   the `CallCycle` type and the `SIM0012` registry entry) would
   have been permanently unreachable dead code duplicating a check
   that already runs earlier and is already mandatory — see
   `docs/simulation.md`'s "Multi-service topology" section for the
   disclosed reasoning. **SIM0011 shipped as designed.**
   `Scenario::validate_against_plan` (`crates/ciac-sim/src/scenario.rs`)
   checks every `request.service`, `given.db[].service`, and
   `expect.row.service` against `SimPlan.services`; wired into
   `sim_inner` (`crates/ciac/src/commands.rs`) as a preflight over
   every `--scenario` file — parse, structural `validate()`, then
   `validate_against_plan(&plan)` — right after `SimPlan::from_ir`,
   before any target's driver runs. Live-verified against the real
   CLI: the unmodified flagship (`order-system.ciac` + its own
   scenario) still passes with the new preflight in place, and a
   scenario with a deliberately wrong `request.service` fails with
   `unknown service "NoSuchService" (known services: OrderSystem)
   (SIM0011)` — before any project is built or driven, not after.
   **A second finding, carried over from this same investigation:**
   the scenario schema's `service` fields (`RequestStep.service`,
   `GivenTableRows.service`, `ExpectStep::Row.service`) were
   already required, non-optional fields with no
   `#[serde(default)]` — not "optional and defaulted" as some
   earlier prose described them — so this milestone's addition is
   the *validation* against a real plan, not a schema change.
   Namespacing/ordering/clock-tiebreak rules and the `given.db`
   service qualifier are unchanged from Pillar 1/2's own design (no
   correction needed there); the composition decision matrix and
   process-shape rule (Pillar 3) are affirmed here as M1's
   committed record, unchanged from their pre-registered form — M2
   onward builds on them as drafted. 6 new unit tests added
   (`ciac-sim`'s suite: 81 -> 87, all passing); full `cargo test
   --workspace --no-fail-fast` clean except the same disclosed
   pre-existing `ruff`-version-drift failure in `backfill_cli` (27
   M9's own finding, reconfirmed unrelated to this milestone's
   changes).

2. **M2 — The world at system scope.** ciac-sim: (service,
   table) namespacing with the single-service degenerate case
   (27's corpus green untouched — the proof the degeneracy
   works), call-router registry with inline-synchronous
   semantics and depth guard, service-aware scheduler tiebreak.
   Unit tests: two-service namespacing collisions, router
   round-trip incl. error-envelope pass-through, cross-service
   delivery order, cycle refusal. Rust inherits by vendoring as
   always (unexercised until M6 — recorded, familiar from 27's
   M2/M3 shape).

   **Shipped (v0.28 M2) — as designed, plus a found-and-fixed M1
   regression.** `SimWorld::namespaced_table_key(service: Option<&str>,
   table: &str) -> String` (`crates/ciac-sim/src/world.rs`) is the
   whole namespacing scheme: `None` (or, once a runner is driving,
   the single-service degenerate case) returns the bare table name
   unchanged, `Some(service)` returns `"{service}/{table}"`; every
   `db_*` entry point funnels through it, so the single-service
   corpus is byte-identical to before this milestone by construction,
   not by a separate code path. `SimWorld` grew an `apis:
   Mutex<ApiRegistry>` (`BTreeMap<(String, String), Arc<ApiHandler>>`,
   `ApiHandler = dyn Fn(Value) -> anyhow::Result<Value> + Send +
   Sync`) plus `register_api`/`call_checked`: routing is inline and
   synchronous on the caller's own logical thread (production's own
   typed call-client shape, just against a registered handler instead
   of real HTTP), an unregistered `(service, api)` is a clear `Err`
   rather than a silent no-op, and `call.request` participates in the
   existing failure-injection vocabulary the same way `db.commit`/
   `broker.publish` already do. The handler map is cloned out and its
   lock dropped *before* invoking the handler (`std::sync::Mutex`
   isn't reentrant; holding the lock across a call that might itself
   call back in would deadlock the first recursive call on the same
   thread) — a `CallDepthGuard` RAII-decrements a `call_depth: Mutex<u32>`
   bounded at `MAX_CALL_DEPTH = 64`, unwinding correctly even through a
   handler panic. That guard is deliberately *not* a cycle detector:
   `ciac-sema`'s `CycleDetection` pass already treats
   `EdgeKind::ServiceCall` as a flow edge in its combined cycle check
   (M1's own finding, reconfirmed here), so a compiled program can
   never reach it — it exists only for a hand-built `SimWorld` (this
   crate's own tests, or a future non-generated driver) whose handlers
   call each other in a way the compiler never saw, turning what would
   otherwise be an unrecoverable stack overflow into a graceful `Err`.
   The negative fixture proving that compile-time refusal
   (`tests/ui/service-call-cycle.ciac`, `expect: CIAC0006`, two
   services calling each other's apis) was added this milestone, not
   M1 — M1's Shipped note recorded the *investigation* that found
   `CycleDetection` already sufficient; M2 is what actually landed the
   fixture proving it, closing that coverage gap. `SchedulingKey`'s
   `service` field (`crates/ciac-sim/src/schedule.rs`) needed **no
   code change at all** — it was already part of 17UpdatePlan.md
   Pillar 6's documented total order (`virtual_timestamp_ms`, `phase`,
   `service`, `actor`, ...), ahead of `actor` in field-declaration
   order, which is comparison order for a derived `Ord`. "Service-aware
   scheduler tiebreak" was a verification item, not an implementation
   item: the new
   `events_from_different_services_tiebreak_by_service_before_actor`
   test schedules two same-timestamp, same-phase events from different
   services with actor names that would sort the *other* way, and
   asserts service identity wins the tie — proving the M1-era design
   was already correct rather than adding behavior that wasn't there.

   **The found-and-fixed regression:** running this milestone's own
   required live Rust-target sim proof (`ciac sim --target rust` against
   `examples/sim-vertical-slice.ciac`) — needed because M2's own changes
   live in `world.rs`, which Rust inherits via `include_str!`
   vendoring — failed to compile with `E0433: could not find 'plan' in
   the crate root`. Root cause, unrelated to any M2 code: 28's M1 added
   `Scenario::validate_against_plan(&self, plan: &crate::plan::SimPlan)`
   directly inside `crates/ciac-sim/src/scenario.rs`, but `scenario.rs`
   is one of the files `ciac-backend-rust` vendors verbatim
   (`include_str!`) into *every* generated Rust project's own crate
   (`VENDORED_SIM_SCENARIO` in `crates/ciac-backend-rust/src/lib.rs`),
   while `plan.rs` is never vendored and cannot easily be — it hard-
   depends on the compiler-only `ciac_ir` crate and `sha2`, neither
   available nor appropriate in a generated runtime project's own
   dependency tree. This broke compilation of every generated Rust
   project's own `sim_runner` binary for any Rust-target `ciac sim`
   invocation since M1 shipped (M1's own verification apparently
   exercised only the Python-target flagship and compile-time
   cycle-detection checks, never a live Rust-target proof — the same
   class of gap 26/27 have surfaced before: a milestone's own
   verification not reaching every path its change touches). Fixed by
   relocating the method (renamed `SimPlan::validate_scenario(&self,
   scenario: &crate::scenario::Scenario) -> Result<(), ScenarioPlanError>`,
   receiver/argument swapped, otherwise behavior-identical) plus
   `ScenarioPlanError` into `plan.rs`, where `ciac_ir` is already a
   legitimate dependency — confirmed via grep that the method is
   called only from the compiler-side CLI preflight
   (`crates/ciac/src/commands.rs`'s `sim_inner`, before any target's
   driver runs), never from within any vendored/generated code path,
   making the relocation semantically safe. `lib.rs`'s re-export and
   the `commands.rs` call site (`plan.validate_scenario(&scenario)`,
   swapped from `scenario.validate_against_plan(&plan)`) were updated
   to match; the two tests that exercised the old method moved from
   `scenario.rs` to `plan.rs` (renamed, same assertions). All 26 Rust
   golden snapshots were regenerated a second time this milestone (the
   vendored `scenario.rs` content changed), reviewed, and are stable
   under a stability re-run. Re-running the Rust-target live proof
   after the fix: `[PASS] v0.17-m5-vertical-slice`, no compile error —
   the regression this milestone found is also the regression this
   milestone closed, disclosed here rather than silently folded into
   "M2 shipped as designed."

   `ciac-sim`'s unit suite grew from 87 (M1's exit count) to 93: the
   namespacing/router/depth-guard/tiebreak coverage above, net of the
   validate_against_plan -> validate_scenario relocation (2 tests
   moved, not added — same coverage, new home). Full `cargo test
   --workspace --no-fail-fast` (`cargo fmt --check` and `cargo clippy
   --workspace --all-targets -- -D warnings` both clean) exits with
   only the same disclosed pre-existing `ruff`-version-drift failure in
   `backfill_cli` (27 M9's own finding, reconfirmed unrelated to this
   milestone). Both live-proof anchors reconfirmed after the fix:
   Python-target `order-system.ciac` (`[PASS]
   27-m9-order-system-flagship`) and Rust-target
   `sim-vertical-slice.ciac` (`[PASS] v0.17-m5-vertical-slice`, the one
   that originally failed).

3. **M3 — Python composition, the pathfinder.** The pyrunner
   system driver: N services loaded under package aliases, one
   world, routes/consumers/jobs registered per service in
   declaration order, call clients guarded to the router, the
   Python driver's bail replaced with topology handling. The
   aliasing sharp edge resolved and recorded (or the sim-only
   shim fallback taken and recorded — either way M5 gets real
   data, which is this milestone's second deliverable beyond
   working code).

   **Shipped (v0.28 M3) — the aliasing sharp edge resolved by
   capture-and-swap, plus one production-code bug the live proof
   found and fixed.** Every generated Python project's top-level
   package is literally named `app`; loading N such projects into one
   process the straightforward way (`sys.path` holding all N project
   directories at once) is unsound — whichever project's `app.db`
   resolves first on `sys.path` wins for *every* service's own `from
   app.db import ...`, silently misrouting every other service's
   database access. This was reproduced live in scratch testing before
   the fix, not just reasoned about. `sim/pyrunner/multi_service.py`'s
   `ServiceModules` is the resolution: each service's fully imported
   `app.*` module tree is captured once (`sys.path` scoped to just that
   project during the import so a not-yet-cached submodule resolves to
   the right files), then swapped into `sys.modules` immediately before
   invoking any of that service's code. Sound specifically because the
   driver's event loop never runs two coroutines' bodies concurrently
   on the same thread — `ScenarioRunner` awaits every step in strict
   sequence — so "which service's `app.*` is live" is a well-defined
   value at every point in execution; a lazy, call-time import (e.g.
   `db.py.j2`'s own `from app.db import ...` inside `get_sessionmaker`,
   reached only when a route actually runs) resolves correctly because
   the target module was already cached during that service's own
   `load()` pass. Confirmed by a standalone smoke test against the real
   generated `multi-service-media.ciac` output before wiring it into
   the driver proper: `billing.app.config` and `upload-api.app.config`
   captured as distinct module objects, module identity round-tripping
   correctly across repeated `activate()` calls, and a lazy call-time
   `from app.state import current` resolving to the *currently active*
   service's own state, not whichever service loaded last.

   `sim/pyrunner/multi_driver.py` is the new N-service counterpart to
   `auto_driver.py` (which is untouched in shape — a single-service run
   still uses it unchanged): it loads every service in `plan.services`
   declaration order into one shared `world.SimWorld`, registers each
   service's workers/jobs/apis wrapped in `_service_scoped` (a
   save-current/activate-target/restore-in-`finally` wrapper, symmetric
   at any nesting depth so a routed call into another service that
   itself routes into a third composes correctly without any call site
   needing to know about `ServiceModules` at all — the same recursive-
   symmetry discipline `CallDepthGuard` uses in Rust), and routes every
   `call <Service>.<Api>` step through `world.call_checked` instead of
   real HTTP via M3b's already-shipped `client.py.j2` guard. `commands.
   rs`'s `sim_drive_python` is now two functions (`_single`, unchanged
   shape, and `_multi`, new): the multi-service path matches each
   `plan.services` entry to its generated project directory by kebab-
   casing the service name (`ciac_codegen::model::Ctx::dir`'s own
   derivation, now also a direct `heck` dependency of the `ciac` crate)
   rather than guessing, `uv sync`s each project's own venv (a service
   may declare dependencies none of the others need — `nats-py` only
   where a queue is used, `aioboto3` only where an object store is
   used), and assembles one `PYTHONPATH` unioning every venv's own
   `site-packages` directory (found by globbing, since the exact
   `pythonX.Y` component varies) alongside the sim scratch dir — third-
   party packages need this global union; `app.*` resolution is what
   `ServiceModules` handles per call.

   **Two bugs the live proof found, neither hypothetical, both fixed
   before this milestone could exit — the API registration gap.**
   Building the live proof against `multi-service-media.ciac` (Billing,
   UploadApi, Transcoder, Notifier, ProgressFeed; `UploadApi.Upload`
   calls `Billing.Charge`, then publishes into a three-hop worker
   chain) surfaced two real defects, not scenario-authoring mistakes:
   (1) the api-discovery logic borrowed from single-service `build_apis`
   only ever registers apis a scenario's own `request` steps name
   directly — correct for single-service (nothing else could call an
   api), wrong for multi-service, where `Billing.Charge` is reached
   *only* via `UploadApi`'s own routed call and is never itself a
   `request` step. Reproduced exactly: `RoutingError: no handler
   registered for Billing.Charge (called from UploadApi)`. Fixed by
   discovering each service's apis from `plan["apis"]` filtered by
   `service_key` — every api the service declares, the same source
   `build_workers_and_jobs` already used for workers/jobs — not from
   scenario steps. (2) Once routing worked, `Video.model_validate`
   failed with three field errors: `call_checked`'s return value turned
   out to carry the *same* `{"status": ..., "data": ...}` envelope
   every route function's own body already builds (`api.py.j2`'s
   `Return` step wraps every pipeline outcome this way, confirmed by
   reading the generated `charge.py`), contradicting M3b's own
   documented assumption ("no HTTP response envelope to unwrap") — a
   real error in that milestone's design note, not just an
   implementation gap. Fixed in `client.py.j2`'s world-guard branch:
   `result["data"]` unwrapped before validation, exactly mirroring the
   real-HTTP branch's `response.json()["data"]`; the doc comment
   corrected to match. Both fixes are additive to the sim-only branch
   only — `client.py.j2`'s production (real-HTTP) branch is untouched,
   confirmed byte-identical by the golden diff review this milestone's
   own exit checklist requires.

   `sim/multi-service-media.ciac-sim.json` (new) is the live-proof
   scenario: one `request` (`UploadApi.Upload`), an `expect.response`
   assertion (proving the routed call round-tripped), two `drain`
   steps, and `expect.quiescence`. Two drains, not one, was itself a
   finding worth recording: `plan.services` (and therefore worker
   registration order, since `ScenarioRunner.workers` is a plain dict
   in registration order) is alphabetical, not source-declaration
   order, so `_drain()`'s single pass reaches `Notifier`'s own worker
   *before* `Transcoder`'s worker has published the `Transcoded`
   message that worker consumes — one drain leaves `Notifier` with an
   undelivered message, exactly as `expect.quiescence` reported before
   the second `drain` step was added. Not a bug: `_drain()`'s single-
   pass-in-registration-order semantics are unchanged from single-
   service and this is the expected shape of a multi-hop cascade
   needing multiple drains, not a defect — but real enough that it
   belongs in this note for whoever authors M4's own three-scenario
   corpus next, rather than being silently absorbed into "the proof
   passed."

   Both single-service anchors reconfirmed unaffected after every fix
   in this milestone (`client.py.j2` is only ever rendered for a
   multi-service system — a single-service program has no `call
   <Service>.<Api>` target to reach): Python-target `order-system.ciac`
   (`[PASS] 27-m9-order-system-flagship`) and Rust-target
   `sim-vertical-slice.ciac` (`[PASS] v0.17-m5-vertical-slice`, unaffected
   since this milestone touched no vendored file). Full `cargo test
   --workspace --no-fail-fast` (`cargo fmt --check` and `cargo clippy
   --workspace --all-targets -- -D warnings` both clean) exits with
   only the same disclosed pre-existing `ruff`-version-drift failure in
   `backfill_cli`. All three multi-service Python golden snapshots
   (`inventory-system`, `multi-service-media`, `traced-checkout`)
   regenerated for the `client.py.j2` change, reviewed, and stable
   under a second, non-updating run; the 26 Rust golden snapshots from
   M2's own fix are untouched by this milestone (no vendored file
   changed). The "sim-only shim fallback" branch this milestone's own
   description names as a possible outcome was not needed — capture-
   and-swap resolved the aliasing edge outright, so M5's checkpoint
   inherits real multi-service data from an actually-working driver,
   not a documented workaround.

4. **M4 — The system corpus, proven on Python.** The three
   scenarios authored (outcomes frozen); `sim-three-service.ciac`
   added and verifying ×5 as an example; the corpus green on
   Python end-to-end — request → cross-service publish →
   foreign-service worker, call round-trip with injected
   call-seam failure, N=3 global ordering. Single-service corpus
   + anchors re-proven untouched. The wall-clock data for
   Python's system runs recorded.

   **Shipped (v0.28 M4) — the three-scenario corpus landed as
   planned, plus two real defects the live proofs found (not
   scenario-authoring mistakes) and a documented, intentional
   coverage trade the second one exposed.** Split into three
   passes rather than one: M4a threaded per-service database
   namespacing end-to-end (the gap M3c's own Shipped note
   disclosed and deferred); M4b authored and proved the new N=3
   example; M4c authored and proved `inventory-system.ciac`'s
   scenario and reviewed `multi-service-media.ciac`'s existing
   M3c scenario against this milestone's own proof-ledger row.

   **M4a — `world.py`'s `namespaced_table_key` (M3a) actually
   wired into the write path, failure-injection subject, and
   transcript, plus the read path.** `_FakeSession._key` composes
   `namespaced_table_key(self._service, table)` once per session
   and every storage/delete/select/commit call site now goes
   through it instead of the bare table name; `ScenarioRunner`
   gained a `multi_service: bool` field so `_expect_row` namespaces
   its own lookup the same way, mirroring `db.py.j2`'s own
   `SERVICE_FOR_SIM` constant — baked in at codegen time (`multi.
   then_some(ctx.service_name.as_str())`, Rust `{:?}`-formatted
   into a Python literal), not derived at runtime, so a
   single-service project's degenerate case (`None` → bare table
   name) is exactly what it was before this milestone touched
   nothing in that path. Disclosed, not fixed: schema-based
   reference/uniqueness validation (Python's `_check_write`,
   Rust's `validate_write`) is keyed by bare table names on both
   targets and silently no-ops against an already-namespaced key —
   traced to M2's own original design, confirmed identical in Rust
   by reading `RelationalSchema::from_tables`/`outgoing`/`incoming`,
   and deliberately unexercised by any of this milestone's three
   scenarios (`sim-three-service.ciac`'s two tables declare no
   `Reference<T>` across the namespaced boundary, by design). A
   real gap, filed for whichever future milestone first needs
   cross-service relations under simulation.

   **M4b — `sim-three-service.ciac`, the N>2 proof.** `Intake`
   calls `Billing.Charge` synchronously, then publishes
   `OrderAccepted` for `Fulfillment`'s worker to consume, each
   downstream service owning its own table (no cross-service
   `Reference<T>`, sidestepping M4a's disclosed gap on purpose).
   The scenario asserts the routed call's effect, the
   cross-service stream delivery, and a `call.request` failure
   injected on `Charge` at occurrence 2 — no charge row, no
   shipment row for the failed order, confirmed genuinely causal
   by a negative control (removing the failure rule, watching the
   previously-failing request succeed instead). All twelve ×5
   golden snapshots (dot/ir/five gen targets/four host-syntax-
   identity/ts-client) generated clean on the first `INSTA_UPDATE`
   pass once `queue NATS` capabilities were added to `Intake` and
   `Fulfillment` (both initially omitted, caught immediately by
   `ciac check`'s CIAC0005).

   **M4c — `inventory-system.ciac`'s call round trip and scoped
   auth, plus two real defects the live proof found.** Extended
   the existing flagship with two scope-gated apis rather than
   inventing a new program: `Gateway.Quote` (the system's own
   ingress) and `Catalog.Restock` (an independent entrypoint on
   the other service), both checked against the one shared,
   system-scoped `FakeAuth`; `Catalog.Price` stays auth-less since
   it's reached only by Gateway's routed `call`, and Pillar 4's own
   "Identity propagation through routed calls" finding — production
   forwards no caller identity through the existing call clients,
   so the router must not invent one — means a call-only callee
   could never satisfy a scope in the first place, sim or real.

   First defect: `multi_driver.py`'s per-service api
   auto-registration loop assumed every `plan["apis"]` entry
   resolves to a single `app.api.<snake>.<snake>` function —
   true for a plain `api`, false for `crud <Name>;`, which lowers
   to a same-shaped `NodeKind::Api` node (`ciac-sema/src/build.rs`'s
   `crud()` expansion) whose module is a five-verb REST resource
   file with no such function. Reproduced live: `AttributeError:
   module 'app.api.item' has no attribute 'item'` the moment
   `Catalog` — which already owned `crud Item` before this
   milestone — was loaded by the multi-service driver for the
   first time. This means the M3c driver could never have loaded
   *any* multi-service system with a `crud` resource, regardless of
   what a scenario actually exercised; not a regression from this
   milestone's own changes, but a latent gap this milestone's
   first `crud`-bearing multi-service proof was always going to
   hit. Fixed by skipping non-single-function modules in that
   loop; a program that genuinely writes `call Catalog.Item` (which
   `ciac check` does not reject — `resolve_call` only checks the
   node exists and the payload type matches, another disclosed,
   pre-existing gap) still fails loudly at simulation time via
   `call_checked`'s own `RoutingError`, not silently.

   Second finding, a correct consequence rather than a bug:
   declaring `auth JWT;` on `Catalog` at all — needed for
   `Restock`'s scope — also flips `crud Item`'s own `has_auth`
   (`docs/language.md`: "`crud` gates every route with that
   capability automatically once it's declared"), even though
   `Item` sets neither `read_scope` nor `write_scope`. This
   silently drops the project's original v0.9 M2 capability
   round-trip system test for `Item` from generation, since
   `ciac-codegen::system_tests::build_capability_checks` already
   and correctly skips any `has_auth` resource ("no credentials to
   present") — confirmed by regenerating the project and finding
   `tests/system/` now emits only `test_calls.py`, no
   `test_capabilities.py`. Documented in the example's own doc
   comment as a disclosed trade rather than avoided by picking a
   different design; `ciac verify --system`'s call-reachability
   test for `Gateway`→`Catalog.Price` is unaffected. Reviewed
   `multi-service-media.ciac`'s existing M3c scenario against this
   milestone's own proof-ledger row ("per-service row assertions")
   and found it genuinely unsatisfiable as written — the program
   declares zero `table`s in any of its five services, a v0.5-era
   topology choice predating this arc — so `sim-three-service.ciac`
   is the proof that actually delivers request→publish→
   cross-service-worker *with* per-service row assertions;
   retrofitting a table onto the v0.5 flagship for this row alone
   was judged higher-risk than worth it (that example's handlers
   are old-style capability-bound stubs, not typed-handler bodies,
   so giving `TranscodeVideo` a real `db.insert` would mean
   changing its fundamental shape, not just adding a field) and is
   left as a named, undone option rather than done silently.

   One clippy regression from M4a's own already-committed code
   (`multi.then(|| ctx.service_name.as_str())`, `clippy::
   unnecessary_lazy_evaluations` under this toolchain) was caught
   and fixed by this milestone's own verification pass
   (`then_some`), not introduced by M4b/M4c.

   Both single-service anchors reconfirmed green:
   `sim-vertical-slice.ciac` at `{"ProcessOrder": 3}`/
   `{"Reconcile": 1}` (`[PASS] v0.17-m5-vertical-slice`) and at
   `{"Reconcile": 7}` via `virtual-week.ciac-sim.json` (`[PASS]
   v0.17-m5-virtual-week`); Python-target `order-system.ciac` via
   the full workspace test suite. Wall-clock data for Python's
   system runs (`ciac sim`, cold, including codegen — not isolated
   scenario-runner time): `multi-service-media` 3.34s,
   `inventory-system` 3.14s, `sim-three-service` 2.77s. Full `cargo
   test --workspace --no-fail-fast` (`cargo fmt --check` and
   `cargo clippy --workspace --all-targets -- -D warnings` both
   clean) exits with only the same disclosed pre-existing `ruff`-
   version-drift failure in `backfill_cli`.

5. **M5 — CHECKPOINT.** The composition go/no-go for compiled
   targets, priced on M3/M4's measured reality: Python's
   composition cost, the observed sharp edges, and a concrete
   re-estimate of the Rust lib+bin reshaping (the single most
   golden-visible item in the arc) against its M1 estimate.
   Outcomes: go (M6–M8 proceed); reshape-first (the Rust lib+bin
   change ships as its own reviewed step before the system
   runner lands on it — the likely choice if churn review wants
   isolation); process-fallback for a named target (a target
   with a structural single-process wall takes the N-process
   fallback — accepted only with the determinism bookkeeping
   design written down first); or no-go (halt, findings, re-plan
   — pre-registered as always, expected never). Checkpoint
   report lands in this file.

   **Shipped (v0.28 M5) — go, with the biggest priced risk
   already retired and a different, unpriced one found in its
   place.** This plan's own text called the Rust lib+bin
   reshaping "the single most golden-visible item in the arc" —
   checked against the actual generator (`crates/ciac-backend-
   rust/src/lib.rs`) rather than against the M1 estimate's
   assumption, **it has already shipped**: every generated Rust
   project has carried both `src/lib.rs` (a full `pub mod` tree —
   `routes`, `state`, `db`, `models`, `services`, `clients`,
   `world`, everything) and a thin `src/main.rs` that only calls
   into `{{ c.module }}::` since v0.17 M11, when `src/bin/
   sim_runner.rs` first needed to import the service's own routes
   and state as a library rather than duplicating them. `Cargo.
   toml.j2` carries no explicit `[lib]`/`[[bin]]` sections at
   all — Cargo's own convention (a package with both `src/lib.rs`
   and `src/main.rs` gets both target kinds for free) already does
   the job. **Net effect: M6 owes zero golden churn for the
   reshaping itself** — every one of the 28 Rust golden snapshots
   the plan expected to touch for this reason alone stays
   untouched by it, because there is nothing left to reshape.
   Reshape-first is consequently not a live outcome: there is no
   separable "reshaping" step left to isolate as its own review.

   That finding does not, on its own, clear M6 — it just means the
   M1 estimate's *named* risk was already retired by earlier work
   for an unrelated reason, not that composition is free. Reading
   the generator further surfaced the risk the M1 estimate never
   named: `crates/ciac-backend-rust/src/lib.rs` vendors `ciac-sim`'s
   `world.rs` (and `clock.rs`/`cron.rs`/`failure.rs`/`scenario.rs`)
   via `include_str!`, pasting the same source text into **every**
   generated project's own `src/world.rs` — a deliberate choice
   (a generated project depends on nothing from this repo's own
   crates, staying self-contained for a user who never sees `ciac`'s
   source) which this plan's own Pillar 3 correctly named as a
   consequence of "Rust: generated projects are binary crates" but
   did not carry one step further: N services built this way get N
   *nominally distinct* `SimWorld` types (same source, but Rust's
   type identity is per-crate, not per-source-text), and a
   system-runner crate depending on all N service crates as
   libraries cannot construct "one world" of a type any of them
   share — the exact "one memory space, one world" property Pillar
   3 opens by naming as the whole reason single-process composition
   was chosen over the rejected N-process design. Python has no
   analogous problem (`sim/pyrunner/world.py` is one file the driver
   imports directly, never duplicated per project), so M3/M4's
   measured experience gives no precedent either way for this one.

   Two paths were weighed, not yet chosen (M6's own first
   pre-registered open question, resolved on contact, in the exact
   tradition M3's aliasing-vs-shim question was): **(a)** extract
   the vendored sim modules into one small path-dependency crate
   that a multi-service system's own N service crates *and* its
   system-runner crate all depend on in place of their private
   `include_str!` copy — single-service projects keep today's
   vendored-and-self-contained shape untouched, since this only
   ever applies when a system-runner crate exists at all; **(b)**
   accept the pre-registered process-fallback for Rust specifically
   (an N-process system runner coordinating N `sim_runner`
   binaries over the existing one-line-stdout protocol, multiplied)
   if (a) proves more invasive than it looks on paper — a
   legitimate, plan-sanctioned outcome for exactly this shape of
   structural wall, not a failure. (a) is the working assumption
   for M6 to open with, since it preserves the single-process
   design Pillar 3 argues for and touches no single-service golden
   file.

   Python's own measured composition cost (M3/M4, cross-checked
   against this checkpoint rather than restated) supports proceeding
   rather than hesitating: `multi_driver.py` (331 lines) + `multi_
   service.py` (153 lines) is the full second driver, and every
   sharp edge the plan predicted in Pillar 3 (import identity in M3,
   database namespacing and API-registration completeness in M4)
   was found live, diagnosed, and fixed with a small, targeted
   change — the aliasing shim fallback was never needed, and no
   scenario ever required a design reversal. The lesson worth
   carrying into M6 specifically: M4c's `crud`-shaped-api
   registration bug (a plan-level api entry that doesn't resolve to
   a single callable, found only once a `crud`-bearing multi-service
   program was actually driven) has no Rust analogue to check for
   yet, since Rust's own call/route registration is direct function
   wiring at codegen time rather than reflective module lookup — but
   the general shape of the lesson ("a system-runner's registration
   pass must be checked against every api-*shaped* IR node the
   corpus actually produces, not just the ones today's examples
   happen to exercise") carries forward regardless of mechanism.

   **Decision: go.** M6–M8 proceed. No reshape-first step (there is
   nothing left to reshape); no process-fallback taken pre-emptively
   (path (a) above is untried, not ruled out); no no-go (nothing
   found here is structural in the sense Pillar 3 reserves that
   outcome for — the vendored-world type-identity question has a
   credible single-process answer that simply wasn't written down
   before this checkpoint went looking for it). M6's own exit
   checklist gains one concrete addition beyond what M1 specified:
   record which of (a)/(b) above was actually taken, and why, the
   same way M3 recorded aliasing-over-shim.

6. **M6 — Rust composition.** The lib+bin project reshaping
   (uniform, behavior-neutral, golden-reviewed under the 26
   invariant discipline — likely pre-shipped per M5), the
   system-runner crate emission (path deps on N service crates,
   one world, registration in declaration order), call-client
   guard, driver topology handling. The system corpus green on
   Rust; outcomes identical to Python's byte-for-byte; build
   timings recorded.

   **Shipped (v0.28 M6) — path (a) taken as planned, a new
   system-runner crate generated and driven, three-scenario corpus
   green with Python-identical outcomes.** M6a (call-client
   world-guard) and M6b (the shared `sim-shared` crate, resolving
   M5's vendored-type-identity finding via path (a) exactly as
   that checkpoint recorded) had already shipped by the time this
   note was written; M6c/M6d close the milestone.

   A gap M5's own checkpoint reading missed surfaced first: Rust's
   vendored `world.rs` had carried M2's namespaced `_for`-suffixed
   methods (`db_insert_checked_for`, etc.) since 28's M2, but
   nothing in `ciac-backend-rust/src/lower.rs`'s typed-handler
   lowering ever called them — every db verb wrote/read the bare
   table name regardless of service, the exact collision risk
   Python's M4a closed for `world.py`. Fixed the same way: `lower.
   rs`'s `RustSyntax` gained a `service_name: Option<String>` field
   (populated from `emit_service`'s own `multi.then_some(ctx.
   service_name.as_str())`, mirroring the Python driver's own
   per-service scoping) and a `world_table_key(&self, table_snake)`
   helper composing `"{service}::{table}"` (`None` degenerates to
   the bare name — the single-service path is untouched, confirmed
   by every single-service Rust golden staying byte-identical
   through this change). Every world-guard branch across `db_
   insert_expr`/`db_update_expr`/`db_delete_expr`/`query_expr`'s
   three arms/`db_get` now composes this key instead of the bare
   `table_snake`, while the real-SQL branch beside it keeps using
   the bare name unconditionally (the physical table itself is
   never namespaced) — the same split M4a drew in Python. A
   `sim_world_tables_multi` counterpart to the existing single-
   service `sim_world_tables` builds the system-runner's own
   `SimWorld::with_schema` table list with the identical namespaced
   keys (and namespaced FK `target_table`s, resolved through a
   `physical name -> owning service` map built from `ir.tables()`),
   so the schema a reference/uniqueness check validates against can
   never drift from what the lowered code actually addresses.

   The system-runner crate itself (`system-runner/`, sibling to
   `sim-shared/` and every service directory) is a plain Cargo
   binary crate with path dependencies on `sim-shared` and every
   service crate by its real package name — since (unlike Python's
   uniformly-`app`-named packages, which needed `multi_driver.py`'s
   `ServiceModules` aliasing shim) every generated Rust service
   crate already has a distinct crate name, there is no aliasing
   problem to solve at all; the driver just names each service
   crate directly. Its `main.rs` (`system_sim_runner.rs.j2`, a new
   template) builds one shared `Arc<sim_shared::world::SimWorld>`,
   constructs each service's own `AppState::simulation(config,
   world.clone())` (sound because `crate::world::SimWorld` is a
   `pub use sim_shared::world` re-export in multi-service mode, so
   every service's "own" world type is the identical nominal type
   M6b's extraction bought), and registers every api in every
   service on the shared world's call router up front, in
   declaration order — mirroring `multi_driver.py`'s own coverage
   rule (an api reachable only via a routed `call` needs an entry
   too, not just the ones a scenario's `request` steps name
   directly). `request`/`advance`/`drain`/`expect` mirror the
   single-service runner's own methods almost verbatim, generalized
   across the `services` list; the one new piece of machinery is
   `block_on_ready`, a same-file helper that polls a future exactly
   once with a no-op waker rather than parking a thread — needed
   because `world.register_api`'s handler type is a *synchronous*
   `Fn(Value) -> anyhow::Result<Value>` (so it stays callable from
   `call_checked`'s own synchronous body), while dispatching a
   routed call still has to drive the same async `axum::Router::
   oneshot` the direct `request` path awaits normally. This is sound
   specifically because full simulation coverage (27's M4: "no
   longer refuses anything") means no world-guarded call site ever
   really suspends on first poll — confirmed live, not just argued:
   every one of the three corpus scenarios (including `sim-three-
   service`'s own cross-service `call` through the Billing/
   Fulfillment seam) ran clean with no `Poll::Pending` panic.
   `commands.rs`'s `sim_drive_rust` gained the same `_single`/
   `_multi` split `sim_drive_python` already has (`find_project_dirs`
   now also excludes `system-runner/`, the same treatment M6b gave
   `sim-shared/`); `_multi` needs no `PYTHONPATH`-style dependency
   assembly at all — `cargo build`/`cargo run` inside `system-
   runner/` already resolves the whole path-dependency graph through
   Cargo itself.

   Live-proofed against all three system scenarios through the real
   `ciac sim` CLI (`--target rust`): `sim-three-service` (N=3,
   cross-service `call` + failure injection), `multi-service-media`
   (upload/charge/transcode/notify fan-out), `inventory-system`
   (call round-trip + scoped auth). All three passed with `error:
   null`, and every field of the JSON outcome matched the Python
   run byte-for-byte with one disclosed, *pre-existing* exception:
   Python's `_drain` unconditionally records `_worker_attempts
   [worker] = 0 + attempts` for every registered worker on every
   `drain` step (so a worker that never fires still appears in the
   dumped map with count `0`, e.g. `multi-service-media`'s own
   `DeadLetterSink`), while the single-service Rust runner's `drain`
   — and this milestone's system-runner, which deliberately mirrors
   it rather than diverging — only touches the map entry inside the
   loop draining that worker's actual messages, so a never-fired
   worker is simply absent from the map rather than present at `0`.
   This predates M6c (the identical structure already existed in
   `sim_runner.rs.j2` since v0.17 M11) and is cosmetic, not
   behavioral: `expect.worker_attempts`/`expect.job_runs` on both
   sides default a missing key to `0` before comparing, so no
   scenario assertion can observe the difference — tracked as an
   open ledger row for a future milestone to close by aligning
   Rust's bookkeeping to Python's, not fixed here since it would
   also require touching the already-shipped single-service
   template, out of this milestone's own scope.

   Golden churn matched the shape M5 predicted for path (a): the
   five multi-service Rust examples (`audited-crud`, `inventory-
   system`, `multi-service-media`, `sim-three-service`, `traced-
   checkout` — every multi-service program in the corpus, no more
   and no fewer) picked up `system-runner/Cargo.toml` + `system-
   runner/src/main.rs`, plus (only in `sim-three-service`, the one
   example with a `table` declaration reachable from a typed
   handler) the two-line bare-to-namespaced `db_insert_checked`
   diff described above (`"charges"` → `"Billing::charges"`,
   `"shipments"` → `"Fulfillment::shipments"`); every single-service
   Rust golden in the corpus stayed byte-identical, confirming the
   `service_name: None` degenerate path is truly a no-op. `cargo
   fmt --check`, `cargo clippy --workspace --all-targets` (zero
   warnings), and `cargo test --workspace --no-fail-fast` all ran
   clean, the latter's only failure being the one standing,
   pre-existing `backfill_cli` ruff-version-drift case this whole
   arc has carried since before it started. Timings: a cold `cargo
   check`/`cargo build` of a fresh `system-runner` crate (first
   resolution of the union of every service's own dependency tree
   plus `tower`/`base64`) took ~35-40s; every subsequent `cargo
   run -- <scenario.json>` was sub-second; the full golden-snapshot
   regeneration across the whole example corpus and all five
   backends (not Rust alone) took ~370-380s, unchanged in shape
   from prior milestones' own recorded runs.

7. **M7 — TypeScript and Go compositions.** TS: system entry
   module importing N app factories; the dependency-skew
   assertion; driver + guard. Go: system-runner module with
   `replace` directives; driver + guard. Corpus green on both,
   identity ×4 now running; timings recorded. Two targets in one
   milestone because both compositions are expected cheap
   (Pillar 3's per-target analysis) — if either surprises, it
   splits out with the deviation recorded, the standing rule.

   **Shipped (v0.28 M7a) — TS half of M7: system-runner npm
   package, real `file:`-dependency resolution verified live, no
   shared Fastify dispatch helper (a deliberate deviation from the
   design sketched at M5/M6, corrected after a live repro), and the
   dependency-skew assertion.** Go's half (M7c/M7d) is separate,
   tracked work; this note covers TS only.

   Before writing `system_sim_runner.ts.j2`, the two open questions
   Pillar 3 flagged for this target — whether `system-runner`
   needs `fastify`/`pg`/etc. as its own direct dependencies, and
   whether a `file:` dependency's own transitive dependencies get
   hoisted into the depending package's `node_modules` — were
   answered empirically, not assumed: a minimal three-package repro
   (`sim-shared` + a mock service with `pg` + a `system-runner`
   depending on both via `file:`) was built in the scratchpad and
   run through real `npm install --package-lock-only`, then real
   `npm ci`/`npm run build`/`node`. Confirmed: npm does **not**
   hoist a `file:` dependency's own transitive dependencies into
   the depender's `node_modules` (only a `node_modules/<name>`
   symlink is created); Node's module resolution, when it follows
   that symlink to the real target directory, searches *that*
   directory's own already-`npm ci`'d `node_modules` for bare
   specifiers reached from within it. This means every service
   still needs its own independent `npm ci && npm run build` before
   `system-runner`'s own build can run (matching the design already
   sketched), but — the one real finding — `system-runner` itself
   needs **no** direct dependency on `fastify`, `pg`, or any other
   provider package: it never imports them directly, and Node
   resolves each service's own transitive imports through that
   service's own installed tree, not through `system-runner`'s.
   `system-runner`'s own `package.json` (built as a small helper
   function in `lib.rs`, mirroring `system_runner_cargo_toml`'s own
   shape, using real `serde_json::json!` construction rather than
   hand-formatted strings) declares exactly three things: `sim-
   shared` and every service by `file:../<dir>`, and `croner`
   (needed directly for the runner's own `dueInstants` helper,
   since `advance` steps compute due job instants without going
   through any service's own code). The `package-lock.json` was
   likewise built from the real lockfile the scratchpad repro
   produced (`node_modules/*` link entries for every local package,
   ordinary registry entries only for `croner`/`typescript`/`@types/
   node`/its `undici-types` transitive) rather than hand-guessed.

   A design point sketched before implementation — a shared,
   structurally-typed `dispatch()` helper so every `(service, api)`
   pair's `app.inject()` call site could funnel through one
   function, mirroring Rust's own `Router`-erasure trick — was
   dropped once actually attempted: unlike `axum::Router::with_
   state`, which erases every service's distinct state type into
   one uniform `Router`, each service's own `buildApp()` return type
   carries its *own* inferred Fastify generic parameters (e.g. from
   `@fastify/otel`'s plugin registration when `tracing` is
   declared), so a shared helper risked exactly the generic-variance
   friction Pillar 3's composition matrix names as this target's
   sharp edge. The template instead inlines the `app.inject()` call
   per `(service, api)` pair — both in `request()`'s big
   `if`/`else if` chain and in the up-front `world.registerApi`
   loop — the same shape the single-service `sim_runner.ts.j2`
   already uses for its one app, just repeated per pair via Jinja
   loops. This sidesteps the friction entirely rather than fighting
   it, at the cost of some duplicated inject-call boilerplate per
   pair — judged the right trade for a target Pillar 3 already
   priced as "expected cheapest," not worth a second design pass to
   avoid.

   One more consequence of the cross-package boundary needed
   fixing: `tsconfig.build.json.j2` had `"declaration": false`
   unconditionally, but `system-runner` needs to import types
   (`AppState`, etc.) from each service's own compiled `dist/*.js`
   output, which requires a co-located `.d.ts`. Gated `"declaration":
   true` on `multi` only (verified live: with it on, every service's
   own `npm run build` in the corpus's three multi-service examples
   emits correct `.d.ts` alongside `.js`, and `system-runner`'s own
   `tsc` resolves `import type { AppState } from "billing/dist/
   state.js"` cleanly, including the transitively-referenced `pg`/
   `drizzle-orm` types inside that `.d.ts`, resolved through
   `billing`'s own already-installed `node_modules` reached via the
   symlink); single-service golden output is untouched by the gate
   (confirmed: no single-service `tsconfig.build.json` snapshot
   changed).

   The dependency-skew assertion Pillar 3 named for this target
   (`assert_no_dependency_skew` in `lib.rs`) checks, for real,
   against the actually-rendered `<dir>/package.json` of every
   service in the system — not just assumed from reading the
   template — that every one of them declares an identical
   dependency/devDependency map (the only per-service variables in
   `package.json.j2` are the `name` field and, uniformly here, the
   `sim-shared` line), returning a `BackendError` naming the first
   divergent service if a future template edit ever broke that
   invariant. Runs unconditionally whenever `system-runner` is
   emitted, before any of its own files are written.

   Live-proofed against all three system scenarios exactly as M6
   was: `sim-three-service`, `multi-service-media`, `inventory-
   system`, each built through the *real* toolchain (`sim-shared`'s
   `npm ci && npm run build`, then every service's own, then `system-
   runner`'s own `npm ci && npm run build`) and run via `node dist/
   sim_runner.js <scenario.json>`. All three passed with `error:
   null`, every field byte-identical to Rust's own M6c system-runner
   output on the same scenarios (worker/job-outcome key ordering
   differs cosmetically — Rust's `BTreeMap` sorts alphabetically,
   TS's plain object keeps insertion order — the same non-finding
   M6 already disclosed, now confirmed to hold at N=3/N=5 service
   scope too, not just single-service). Golden churn: the same five
   multi-service TS examples M6 touched on the Rust side (`audited-
   crud`, `inventory-system`, `multi-service-media`, `sim-three-
   service`, `traced-checkout`) picked up six new `system-runner/*`
   files each (`package.json`, `package-lock.json`, `tsconfig.json`,
   `tsconfig.build.json`, `.gitignore`, `src/sim_runner.ts`) plus the
   per-service `"declaration": true` addition to `tsconfig.build.
   json`; every single-service TS golden in the corpus stayed
   byte-identical. `cargo fmt --check`, `cargo clippy --workspace
   --all-targets` (zero warnings), and `cargo test --workspace
   --no-fail-fast` all ran clean, the latter's only failure being
   the same standing, pre-existing `backfill_cli` ruff-version-drift
   case M6's own note disclosed (confirmed unrelated by reproducing
   it identically against a clean stash of this milestone's changes).

   The live-proof above was run by hand through the raw toolchain
   (`npm ci`/`npm run build`/`node`), not through `ciac sim` itself
   — `commands.rs`'s own `sim_drive_typescript` `_single`/`_multi`
   split (mirroring `sim_drive_rust`/`sim_drive_python`) is the
   remaining TS work this milestone still owes, tracked next.

   **Shipped (v0.28 M7b) — the driver split, and the same three
   scenarios re-proved through the real `ciac sim --target
   typescript` CLI end-to-end.** `sim_drive_typescript` gained the
   identical `_single`/`_multi` split `sim_drive_rust`/`sim_drive_
   python` already carry: `find_project_dirs(out, "package.json")`
   (already excluding both `sim-shared/` and `system-runner/`, per
   M6b/M6c's own additions to that walk) dispatches to
   `sim_drive_typescript_single` for exactly one project (unchanged
   body) or `sim_drive_typescript_multi` for more than one. Unlike
   Rust's own `_multi` (where `cargo build` inside `system-runner/`
   resolves the whole path-dependency graph unaided), M7a's own
   live repro already established that npm does not hoist a `file:`
   dependency's transitive dependencies, so `_multi` builds `sim-
   shared`, then every service project, then `system-runner`
   itself, in that order (`npm ci && npm run build` at each step)
   before driving it -- the same three-phase build order the manual
   scratchpad proof used, now the actual driver's own behavior. Both
   `_single` and `_multi` funnel through one new shared helper,
   `run_node_sim_runner` (`node dist/sim_runner.js <scenario>`,
   one-line-JSON-on-stdout parsing), factored out since the drive
   loop itself was byte-identical between the two paths -- only the
   build steps beforehand differ.

   Live-proofed through the real CLI this time, not by hand: `ciac
   sim --target typescript --out <dir> --scenario <path> <file>`
   against all three system scenarios (`sim-three-service.ciac`,
   `multi-service-media.ciac`, `inventory-system.ciac`), each from a
   clean `--out` directory so every `npm ci` was a genuine cold
   install. All three printed `[PASS]` (and, separately, verified
   with `--json`, produced a well-formed envelope with `sim.
   scenarios[0].passed: true`). Timings (cold, no npm cache reuse
   across services): `sim-three-service` (3 services + sim-shared +
   system-runner, 5 `npm ci`s) ~56s wall; `multi-service-media` (5
   services) ~72s wall; `inventory-system` (2 services) ~31s wall --
   scaling with service count as expected, dominated by `npm ci`
   itself rather than `tsc` or the scenario run (every individual
   `node dist/sim_runner.js` invocation after the builds completed
   was sub-second, matching Rust's own M6d timing note). `cargo fmt
   --check`, `cargo clippy -p ciac --all-targets` (zero warnings),
   and `cargo test -p ciac --no-fail-fast` (skipping the same
   standing, pre-existing `backfill_cli` ruff case) all ran clean.
   This closes the TS half of M7 in full; Go's half (M7c/M7d)
   remains, tracked next.

   **Shipped (v0.28 M7c) — Go's half of M7: the `simbridge` facade
   package (a discovery beyond what Pillar 3 anticipated), the
   `sim-shared` module, `world.go`'s call router, the `client.go`
   world-guard, and `system-runner`'s own `go.mod`/`main.go`.**
   Unlike TS's file-boundary friction (all solved by M7a's own
   npm findings) or Rust's crate-boundary friction (solved cleanly
   by M6b's vendored `sim-shared` crate alone), Go's own module
   boundary turned out to need a *second*, independent fix once
   actually attempted: Go's `internal/` package-visibility rule is
   directory-based and scoped to the import-path prefix rooted at
   `internal`'s own parent, and this scoping holds **at the module
   level too** — a live `go build -mod=mod` against a hand-built
   `system-runner` module reached only through a `go.mod` `replace`
   directive produced `"use of internal package billing/internal/
   config not allowed"` the moment it tried to import anything under
   a service's own `internal/` tree, not just `internal/world` as
   the plan's own composition matrix had assumed. `world.World`
   itself was already solved the same way `sim-shared` solves it for
   Rust — moving `world.go` out of any single service's `internal/`
   tree into its own `sim-shared` module, imported identically by
   every service and by `system-runner` — but every *other* internal
   symbol `system-runner` needs (`Config`, `AppState`, `FromEnv`,
   `New`, `NewSimulation`, `Router`, worker/job handler funcs and
   their subject/queue-group/retry constants, typed payload structs)
   still needed a per-service answer, since those types must stay
   each service's own distinct types, not one shared type the way
   `world.World` could be.

   The fix: a new `simbridge.go.j2` template emitting a non-
   `internal` `simbridge` package per service (`simbridge/
   simbridge.go`, gated to multi-service emission only, unconditional
   whenever `system-runner` exists since `Config`/`AppState`/`New`
   always exist regardless of which capabilities a service declares)
   whose entire content is bare re-exports of already-`internal`
   symbols — Go type aliases (`type Config = config.Config`) and
   function-value vars (`var FromEnv = config.FromEnv`) — mirroring
   how a Rust crate's `pub use` or a TypeScript package's named
   export already cross an equivalent boundary for free; Go's own
   directory-based visibility needed an explicit facade package to
   do the same job. `system_sim_runner.go.j2` imports one aliased
   `simbridge` package per service (`{{ suffix }} "{{ svc.package }}/
   simbridge"`) rather than four separate `internal/*` imports per
   service, and every previously-`{{ suffix }}Workers.`/`{{ suffix
   }}Schemas.`/`{{ suffix }}Config.`/`{{ suffix }}State.`/`{{ suffix
   }}Routes.`-prefixed reference collapses to one `{{ suffix }}.`
   prefix. One live bug the corpus caught before this shipped: two
   workers in the `notifier` service (`Notify` and
   `DeadLetterSink`) both consume `Video`-typed payloads, and the
   per-worker Jinja loop originally emitted `type Video =
   schemas.Video` twice — a `go vet` "Video redeclared in this
   block" failure. Fixed with a `{%- set ns =
   namespace(seen_payloads=[]) %}` loop tracking already-emitted
   `worker.payload.class_name` values, only emitting the alias once
   per distinct payload type.

   `world.go.j2` gained the call router M2's design already named:
   `type ApiHandler func(req any) (any, error)`, an `apiKey{service,
   api}` map key, `const maxCallDepth = 64`, `apis
   map[apiKey]ApiHandler` and `callDepth int` fields on `World`, and
   `RegisterAPI`/`CallChecked` methods. Go's handler-invocation chain
   is naturally synchronous — no async bridge was needed at all here,
   unlike Rust's `block_on_ready` seam or even TypeScript's own
   (trivial) `async`/`await` — `CallChecked` just unlocks `w.mu`
   before invoking the handler, since the handler may itself call
   back into `World` and the mutex is not reentrant. A
   `NamespacedTableKey` free function (`if service == "" { return
   table }; return service + "::" + table`) mirrors Rust's
   `SimWorld::namespaced_table_key`/TS's `namespacedTableKey`,
   used only by `system-runner`'s own `given.db` seeding at scenario-
   load time (runtime service/table values); codegen-time call
   sites still bake the equivalent key in as a literal via the
   already-existing `GoSyntax::world_table_key`. `client.go.j2`
   gained the same world-guard shape as Rust's/TS's own clients: a
   `world *world.World` field plus, per api method, an `if
   c.world != nil { ...c.world.CallChecked(...)... }` branch that
   unwraps `result.(map[string]any)["data"]` from the real `{
   "status":"accepted","data":<result>}` envelope `httpx.Accepted`
   already builds, falling through to the real HTTP path when
   `world` is nil (single-service, unchanged).

   `system-runner`'s own `go.mod` needed one more empirically-found
   correction: a first attempt hand-wrote a minimal `require` list
   (just `sim-shared` plus per-service `replace` targets and a
   conditional `robfig/cron/v3`), which failed under Go's default
   `-mod=readonly` build mode with `"go: updates to go.mod needed"`
   — `system-runner` transitively needs every third-party package
   any service's own `simbridge` package pulls in (aws-sdk-s3,
   redis, nats, jwt/validator deps, …), and `-mod=readonly` refuses
   to resolve anything not already declared. `go build -mod=mod`
   confirmed the auto-populated `// indirect` block was exactly the
   same fixed, capability-independent set every service's own `go.
   mod.j2` already declares unconditionally — so
   `system_runner_go_mod()` was rewritten to actually **render**
   `go.mod.j2` itself (with `c.package = "system-runner"`, `multi =
   true`) rather than hand-duplicating a dependency list, appending
   only the genuinely per-system `require <pkg> v0.0.0` / `replace
   <pkg> => ../<dir>` lines as plain text afterward. `sim-shared`'s
   own `go.mod` needed no such treatment — `world.go.j2` has zero
   per-service template variables and imports only Go stdlib, so its
   `SIM_SHARED_GO_MOD` constant is a fixed three-line module
   declaration.

   `system_sim_runner.go.j2` (the new, ~800-line multi-service
   scenario-driving runner, mirroring TS's own `system_sim_runner.
   ts.j2` but simplified) took one genuine, disclosed shortcut TS
   couldn't: Go's `*http.ServeMux` is a plain non-generic concrete
   type, unlike TS's per-app Fastify generic types (the friction
   that forced M7a to inline every `app.inject()` call site rather
   than share one helper), so Go's runner drives every `(service,
   api)` pair through one shared `doInject(mux *http.ServeMux,
   method, path string, body []byte, headers map[string]string)
   (int, any, error)` function — no per-pair inlining needed. One
   compile-time lesson the corpus caught: `drain`/`advance` were
   first written as top-level functions taking `*world.World` and a
   counts map, but `go build` reported `undefined: stTranscoder`
   (etc.) — Go has no closures over top-level functions, so a
   separate `func drain(...)` can't see a `st{{suffix}}` variable
   declared inside `main()`. Fixed by converting both into local
   closures declared inside `main()` right after the per-service
   state-building loop, dropping their `world`/state parameters
   entirely since closures capture the enclosing scope directly —
   the same reason TS's own runner nests its equivalent two
   functions inside its own `main()`-equivalent. A second, unrelated
   compile error (`declared and not used: stProgressFeed`, for a
   service with zero apis/workers/jobs of its own) was fixed with an
   unconditional `_ = st{{ suffix }}` right after each service's
   state construction.

   Build-verified (not yet run through `ciac sim` end-to-end — that
   CLI wiring is M7d's own remaining work, matching the M7a/M7b
   split TS just went through) across the full corpus: `gofmt -l`
   and `go vet ./...` came back clean under Go's *default*
   `-mod=readonly` mode for every one of the five multi-service
   corpus examples (`audited-crud`, `inventory-system`, `multi-
   service-media`, `sim-three-service`, `traced-checkout`) — every
   service directory, `sim-shared`, and `system-runner` itself, each
   regenerated fresh into a clean scratch directory rather than
   reusing a stale build cache. Single-service Go golden output was
   spot-checked unaffected beyond the new call-router additions to
   `world.go.j2` (present in every target, single- or multi-service,
   since the router costs nothing when `RegisterAPI` is never
   called). `cargo fmt -p ciac-backend-go -- --check` initially
   flagged two spots (the `sim_needs_context` closure and the `client.
   go.j2` render call site) needing rustfmt's own line-wrapping;
   `cargo fmt -p ciac-backend-go` fixed both. `cargo clippy -p ciac-
   backend-go --all-targets -- -D warnings` was clean on the first
   run; `cargo test -p ciac-backend-go` passed 5/5 with no
   regressions. Golden-snapshot regeneration (`INSTA_UPDATE=always
   cargo test -p ciac-integration-tests --test golden
   example_generated_project_snapshots`, 464.75s) updated 27 `golden__
   gen__go__*.snap` files with zero cross-target contamination
   (verified: no non-Go golden file touched) — 20 single-service Go
   examples picked up only the call-router section, and the five
   multi-service examples picked up the full new `simbridge/
   simbridge.go`, `sim-shared/go.mod`, `sim-shared/world/world.go`,
   `system-runner/go.mod`, `system-runner/go.sum`, `system-runner/
   main.go` file set. Full `cargo test --workspace -q --no-fail-fast`
   (skipping the one standing, pre-existing, already-disclosed
   `backfill_cli` ruff-drift case every prior milestone's own note
   also excludes) ran clean end to end, `EXIT_CODE:0`, zero `FAILED`
   lines anywhere in the log. Go's half of M7 now has its
   implementation and build-verification half done; the driver
   wiring, `ciac sim`-CLI-driven live proof against all three system
   scenarios, and M7 close-out remain, tracked next as M7d.

   **Shipped (v0.28 M7d) — the driver split, a live-proof-caught
   `_steps.go.j2` correctness bug, and M7 close-out.** `sim_drive_go`
   gained the identical `_single`/`_multi` split `sim_drive_rust`/
   `sim_drive_typescript` already carry:
   `find_project_dirs(out, "go.mod")` dispatches to
   `sim_drive_go_single` for exactly one project (unchanged body,
   `cmd/sim_runner` sub-package) or `sim_drive_go_multi` for more
   than one (`system-runner/main.go`, a plain package-root `main` —
   no sub-package split needed there since a module with only one
   `main` package never collides with anything). Unlike TypeScript's
   `_multi` (three-phase build required, since npm doesn't hoist a
   `file:` dependency's own transitive dependencies — M7a's own
   finding), Go's `_multi` needs no build-ordering logic at all:
   `go build`/`go run` inside `system-runner/` resolves the whole
   `replace`-directive dependency graph unaided, the same as Rust's
   own Cargo path-dependency resolution in `sim_drive_rust_multi`.
   Both paths funnel through one new shared helper,
   `run_go_sim_runner(project_dir, args, scenarios, wall_timeout)`,
   parameterized only on the caller's own `go run` package selector
   (`["run", "./cmd/sim_runner"]` vs `["run", "."]`) — the drive loop
   itself (one-line-JSON-on-stdout parsing) is otherwise byte-
   identical between single- and multi-service, mirroring exactly why
   TS's own `run_node_sim_runner` was factored out the same way.

   Live-proofed through the real CLI against all three system
   scenarios (`sim-three-service.ciac`, `multi-service-media.ciac`,
   `inventory-system.ciac`), each from a clean `--out` directory:
   `multi-service-media` failed immediately, not with a driver bug
   but a genuine template-correctness bug this milestone's own proof
   was the first thing ever to reach. Root cause, found via the
   panic trace: `_steps.go.j2`'s `match` step (`pipeline Transcode:
   TranscodeVideo -> match status { Ready -> publish Transcoded;
   Failed -> publish DeadLetters; }`) had always type-asserted
   `result` to `map[string]any` and read the matched field out of it
   as a generic JSON value — a speculative shape the template's own
   doc comment already flagged as untested ("`call`/`match` steps
   render plausible code ... but are unreachable by every
   24UpdatePlan.md M3 example"), written before Go's `call`
   infrastructure existed and never revisited once it did. But
   `result` at that point in a worker pipeline is never
   `map[string]any` — `worker.go.j2`'s drain loop always JSON-decodes
   into the worker's own known typed payload struct
   (`schemas.Video`), and the preceding untyped `handler` step's
   `services.NewTranscodeVideo(st).Handle(ctx, result)` call passes
   that typed value straight through unchanged when (as here) the
   seed stub is left as its default `return payload, nil`. The
   type assertion panicked: `interface conversion: interface {} is
   schemas.Video, not map[string]interface {}`. Fixed by switching on
   `result.(payload_type).<Field>` instead — `payload_type` was
   already threaded through `emit_step` for typed-handler steps, so
   `worker.go.j2` and `route_api.go.j2` (both of which already
   compute a real `payload_type`) needed no changes; only
   `_steps.go.j2`'s own `match` branch changed, gated on whether
   `payload_type` is non-empty, falling back to the old
   `map[string]any` shape only for `job.go.j2`'s still-`payload_type
   = ""` context (no example reaches a job-pipeline `match`, so that
   branch stays exactly as speculative as before). This is the same
   typed-field-access shape Rust's own `match result.status { ... }`
   already uses — Rust never had this bug because its pipeline
   threads the real `Video` type throughout rather than `any`.

   Golden churn from the fix: exactly two snapshots, one line each
   (`golden__gen__go__multi-service-media.snap`,
   `golden__gen__go__routed-media.snap` — the only two corpus
   examples whose `match` step is ever actually reached) —
   `switch v, _ := result.(map[string]any)["status"].(string); v {`
   became `switch v := result.(schemas.Video); v.Status {`; every
   other Go golden file, single- or multi-service, stayed byte-
   identical.

   With the fix in, all three system scenarios passed clean through
   the real CLI: `sim-three-service` 1.5s, `multi-service-media`
   1.3s, `inventory-system` 1.7s (each `[PASS]`, `--json` confirming
   `error: null`) — an order of magnitude faster than TS's own
   cold-`npm-ci` timings (M7b: 31–72s), since `go build`/`go run`
   never needs a package-manager round-trip once the module cache is
   warm, the same shape Rust's own M6d timings already showed. Ran a
   four-way identity comparison (Python/Rust/TypeScript/Go) against
   the same three scenarios: all twelve runs printed `passed: true,
   error: null`, with matching `worker_attempts`/`job_runs` — Python's
   own `multi-service-media` outcome carries one extra
   `DeadLetterSink: 0` zero-attempt key the other three targets'
   maps omit, the same cosmetic non-finding M6d/M7b's own notes
   already disclosed (an unreached worker some targets omit from the
   attempts map entirely, Python includes at zero), confirmed to
   still hold at this scope. `cargo fmt --check`, `cargo clippy
   --workspace --all-targets` (zero warnings), and a full `cargo test
   --workspace --no-fail-fast` (skipping the same standing,
   pre-existing, already-disclosed `backfill_cli` ruff-drift case
   every prior milestone's own note excludes) all ran clean —
   `EXIT_CODE:0`, zero `FAILED` lines. This closes M7 in full: both
   TS's half (M7a/M7b) and Go's half (M7c/M7d) now have implementation,
   build verification, driver wiring, and CLI-driven live proof done,
   with the one real correctness finding (the `match`-step bug) fixed
   rather than merely disclosed, since it was caught before this
   milestone's own exit rather than after.

8. **M8 — Java composition.** N isolated
   `AnnotationConfigApplicationContext`s in one JVM sharing the
   world bean; the classpath-assembly decision (aggregator POM
   vs generalized exec arrangement) made against 25's packaging
   precedents and recorded; driver + guard; corpus green,
   identity ×5 complete; timings recorded.

9. **M9 — Close-out: ×5 identity, docs, ledger, version,
   retrospective.** The full system corpus asserted identical
   ×5 in the harness (now a standing CI surface via the
   `generated-sim` rows, with the job's measured cost recorded
   and scoped); the compose-backed fidelity rows for the three
   scenarios (Docker-delegated) recorded; docs complete
   (simulation.md system section + fidelity note; five
   "single-service only" sentences retired; scenario reference);
   backends.md Open row closed with proof; version **0.25.0 →
   0.26.0** (workspace + pins, vscode manifest, language.md
   compiler parenthetical — language still 1.0.0, third
   consecutive arc proving the two-version discipline);
   retrospective appended after a rule — composition cost per
   target vs M1/M5 estimates, the sharp edges that materialized
   vs the ones that didn't, and the handoff to 29UpdatePlan.md.

### Per-milestone exit checklists

- **M1 exits when:** validation + SIM0011/0012 implemented and
  fixture-tested; namespacing/ordering/outcome rules recorded in
  docs draft; composition matrix + process-shape rule committed
  in this file; schema additions validated; `multi_service`
  consulted.
- **M2 exits when:** namespacing (with degenerate case proven by
  27's corpus running green unchanged), router semantics, and
  scheduler tiebreak unit-tested in-crate; cycle refusal tested.
- **M3 exits when:** the Python driver accepts multi-service
  programs end-to-end; aliasing resolution recorded; call-client
  guard's production branch byte-identical (golden review).
- **M4 exits when:** three scenarios frozen and green on Python;
  the new example verifies ×5; single-service corpus + anchors
  untouched; timings recorded.
- **M5 exits when:** the checkpoint report with measured data and
  one of the four named outcomes is committed in this file.
- **M6 exits when:** Rust corpus green with Python-identical
  outcomes; lib+bin reshaping reviewed (or pre-shipped) under
  the invariant discipline; timings recorded.
- **M7 exits when:** TS + Go corpus green, identity ×4;
  dependency-skew assertion in place; timings recorded.
- **M8 exits when:** Java corpus green, identity ×5; packaging
  decision recorded; timings recorded.
- **M9 exits when:** CI rows live with measured cost; fidelity
  rows recorded; docs complete with the five retirements; ledger
  row closed; version bumped; retrospective appended.

## Open questions resolved at implementation (pre-registered)

1. **Python module aliasing vs sim-only shim** — resolved in M3
   on contact with the real import machinery; recorded with the
   losing approach's failure mode.
2. **Rust lib+bin: this arc or pre-shipped at M5's direction** —
   the reshaping is behavior-neutral either way; the question is
   review isolation, decided on M5's churn data.
3. **Java classpath assembly** — aggregator POM vs generalized
   exec-plugin arrangement; decided in M8 against 25's packaging
   precedents and the one-line-stdout contract.
4. **`publish` step service disambiguation** — expected
   unnecessary (streams are globally named); confirmed or added
   additively in M1, recorded.
5. **Outcome-key qualification for single-service runs** — bare
   names preserved (bias: yes, zero churn) vs uniformly
   qualified; decided in M1 with the harness's canonicalization
   rules updated to match.
6. **CI scoping for system rows** — all three scenarios ×5 every
   push vs a representative subset with the rest scheduled;
   decided at M9 on measured job cost, recorded — the
   pre-agreed-narrowing discipline, not silent deletion.

## Verification strategy

The standing discipline plus this arc's spine: the system corpus
×5 identity harness (extending 27's), the
single-service-untouched proof at every milestone (27's corpus +
the two anchors — the degenerate case is a regression surface,
not a freebie), the byte-identical production branch review on
every call-client guard and on the Rust reshaping, and measured
build/run timings per target per milestone (the arc's cost
honesty).

The proof ledger by layer:

| Claim | Oracle |
| --- | --- |
| systems simulate | three system scenarios green per target as its composition lands |
| outcomes are target-independent | ×5 identity on the system corpus |
| N>2 works | the three-service scenario, not an assertion |
| single-service regression-free | 27 corpus + canonical anchors byte-exact every milestone |
| call seam faithful | router round-trip incl. error envelope; injected call failure scenario; production branch byte-identical goldens |
| global ordering deterministic | cross-service delivery order unit tests + the N=3 scenario's frozen outcomes |
| composition adds no deps | per-milestone dependency assertion ×5 |
| sim vs reality at system scope | compose-backed fidelity rows for the three scenarios (Docker-delegated) |
| refusals specific | SIM0011/0012 fixture tests; scoped record/replay message |
| cost visible | recorded build/run timings per target; CI cost measured before scoped |

## Milestone dependencies and parallelism

M1→M2→M3→M4→M5 strictly sequential (the pathfinder spine).
M6/M7/M8 after M5, independent of each other, listed order
default (Rust first because its reshaping is the arc's biggest
review; Java last mirroring every arc's precedent). M9 last.
The only intra-arc parallelism worth taking: the three-service
example + scenario authoring (M4 content) can draft during
M2–M3; docs drafts ride their milestones as always.

## Explicit cuts

No network-failure modeling between services (no partitions, no
latency injection — the shared world is reliable transport by
design; the fidelity note says so). No per-service clocks or
clock skew. No multi-service record/replay (scoped refusal;
ledger row widened honestly). No parallel scenario execution.
No N-process composition except as a checkpoint-authorized
fallback with its determinism design written first. No service
subsetting (`ciac sim` runs the whole system or nothing — a
partial-system mode is a plausible future convenience, not this
arc). No cross-service transaction semantics (the language has
none; the simulator invents none — a call inside a transaction
is a call, exactly as in production, where it is also not
transactional). No new failure actions. No changes to `verify
--system`/compose beyond the fidelity rows.

## Risks

- **The composition problem is harder than its per-target
  analysis.** The whole arc structure is the mitigation: Python
  pathfinds with the least machinery, M5 re-prices everything
  with real data before three more targets commit, the fallback
  ladder is pre-registered, and the worst honest outcome — some
  target ships composition an arc late with the ledger saying
  so — degrades the schedule, not the truth.
- **The Rust lib+bin reshaping churns every Rust golden at
  once.** Behavior-neutral by construction, reviewable
  mechanically (bin delegates to lib), and M5 can order it
  pre-shipped as an isolated change precisely so the system
  runner's review isn't buried in it.
- **Import/classpath collisions surface late and ugly.** Each
  target's sharp edge is named in Pillar 3 *before* its
  milestone, with an intended shape and a fallback — the
  milestone starts from a design, not a discovery; deviations
  are findings, recorded.
- **The degenerate case quietly changes single-service
  behavior.** The 27 corpus + anchors run at every milestone as
  a hard gate — the cheapest possible tripwire for the most
  embarrassing possible regression.
- **Global-order rules underspecify some interleaving and ×5
  identity fails on a legitimate ambiguity.** 27's experience
  says the harness finds these fast and the fix is a one-line
  rule addition to the delivery spec — the spec is the living
  document for exactly this; each addition is recorded and both
  references + three restatements re-checked against it.
- **CI cost creep.** Timings measured per milestone, the scoping
  decision pre-agreed as data-driven (Open question 6), and the
  narrowing — if needed — deliberate and visible, never quiet.

## Confidence and handoff

High on the world and contract (bounded extensions of
just-built, just-tested machinery, designed once in the shared
crate). Medium on composition, held with the same honesty 27
held its restatements: the novel problem is named, its
per-target sharp edges are pre-analyzed with intended shapes and
fallbacks, the pathfinder-then-checkpoint structure buys real
data before the expensive commitments, and the fallback ladder
terminates in outcomes that are worse in schedule but never in
truthfulness. When this arc closes, the simulation surface has
no structural refusals: five targets, full depth, any topology,
one command, zero infrastructure — the sentence 29UpdatePlan.md
gets to put on the front door.

Handoff: 29UpdatePlan.md (The Front Door) — the README narrative,
the guide series, the positioning doc, the editor polish, and
dogfooding readiness, all describing the system as these three
arcs leave it: gaps closed or decided, simulation total, releases
real. The plan after that one is written by whatever the
dogfooding session teaches.
