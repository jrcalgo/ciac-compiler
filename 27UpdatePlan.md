# CIaC v0.27-file — Simulation Depth: Five Full Worlds (implementation plan)

> Implementation plan. Document number ≠ release number (standing
> precedent; version assigned at execution — expected to ship as
> **0.25.0**). Assumes 26UpdatePlan.md shipped: in particular, Rust
> `transaction {}` is genuinely atomic in production before this
> arc builds a deepened transaction fake on top of it (the
> sequencing 26's Pillar 1 argued for explicitly), the divergence
> ledger's Open table exists for this arc to empty, and CI scans
> what this arc generates. This is the arc that resolves the
> largest single trust deficit the punch-list review named: four of
> five targets fake exactly two effects (`db.insert` + broker
> publish) while Python fakes nine capability families, and the
> user-facing consequence is that `ciac sim` — the project's
> signature capability — refuses most real programs on most
> targets. The decision, made explicitly during the planning
> discussion, was the ambitious branch: **deepen the four narrow
> targets to match Python's coverage**, not enshrine narrowness —
> and further, extend Python's own fake where its disclosed
> residual gaps (`db.update`, the query subset) block the flagship
> demonstration.
>
> **Parity contract:** every capability family Python's world fakes
> today — relational store with schema-aware inserts, deletes,
> cascades, and atomic `commit_batch`; broker with independent
> per-(subject, group) cursors; virtual clock; TTL cache; object
> store; email; search; fixture-driven external HTTP; claims-lookup
> auth — faked on Rust, TypeScript, Go, and Java, plus `db.update`
> and `db.count`-class read verbs added to **all five** including
> Python; the `unsupported_sim_capabilities` gates driven to empty
> and all four targets flipped from `SimSupport::Narrow` to
> `SimSupport::Full` (with record/replay carved out as its own
> explicit support flag — this plan discovers and fixes a latent
> gating conflation there); the failure vocabulary held at
> `error`-only *by parity* (that is what Python implements; the
> wider vocabulary stays parse-but-refuse, disclosed); and the
> flagship acceptance: **`examples/order-system.ciac` — today's
> canonical refusal case — simulates on all five targets with
> identical exact business outcomes**, alongside the two standing
> canonical scenarios whose outcomes must never move
> (`{"ProcessOrder":3}/{"Reconcile":1}`,
> `{"ProcessOrder":100}/{"Reconcile":7}`).
>
> **Confidence:** high on the Rust half of the arc — the Rust
> backend vendors `ciac-sim`'s source verbatim via `include_str!`,
> so deepening the shared crate's world *is* deepening Rust's
> world, and the shared crate is ordinary Rust with ordinary unit
> tests. Medium on the three hand-restated worlds (TS/Go/Java),
> not because any single fake is hard — none is; they are maps,
> lists, and counters — but because restatement drift is this
> arc's defining risk at 3× the surface it had in the narrow era,
> and the mitigation (a shared scenario corpus that all five
> targets must pass with identical outcomes — behavior conformance,
> not code review) has to carry the whole load. M5 is a hard
> CHECKPOINT between the two halves: Rust reaches full parity
> first, and the measured cost of Rust's guard/runner work
> calibrates the go decision for the three restatements, exactly
> as the factory arcs' checkpoints did.

## The gap this version closes

`ciac sim` is the project's most distinctive claim: deterministic,
Docker-free, virtual-time testing of real business outcomes against
generated production code. On Python, the claim is broad — nine
capability families faked, worker retry, lost-ack redelivery, TTL
expiry across virtual weeks. On the other four targets the claim
is two verbs wide. The status table in docs/simulation.md says
"same narrow slice as Rust" three times in one column, and the
refusal machinery — honest, specific, well-engineered — tells a
Rust or Java user, for most real programs, that the fastest test
loop in the toolchain is not for them. `examples/order-system.ciac`,
the language's own flagship example, is the canonical *refusal*
fixture: it exists in the sim test suite to prove the gates work,
which is the disclosure discipline at its best and the capability
story at its worst.

The narrowness was never a design position — 23/24/25 each shipped
their slice as "the gated final milestone" of a backend arc whose
budget was spent on production parity, with depth explicitly
deferred. The punch-list review forced the question the deferrals
had stacked up ("either deepen or commit to narrow forever"), and
the answer was deepen. This arc is that answer executed: at its
end, the sentence "narrow slice" leaves the documentation, the
gates gate nothing on any checked-in example, the flagship
simulates everywhere, and the simulation story stops being a
Python feature with four asterisks.

The arc also pays down two debts inside the machinery itself,
found while scoping: Python's `Full` support hides residual gaps
of its own (`_FakeSession` discloses no attribute-mutation update
and no arbitrary query — sim/pyrunner/world.py:708-718 — which is
precisely why order-system, which calls `db.update`, cannot
simulate even on Python today); and the `SimSupport` enum
conflates simulation depth with record/replay support (`sim_inner`
refuses `--record/--replay` on `Narrow` — flipping four targets to
`Full` without decoupling that check would silently promise replay
machinery none of their runners implement). Both are fixed here,
not papered.

## Pillar 1 — The parity target, enumerated

"Match Python's coverage" becomes a checkable contract only if the
coverage is enumerated. From sim/pyrunner/world.py (787 lines), the
reference fake set, with the extension rows marked:

| Family | Reference behavior (Python today) | Notes for the four ports |
| --- | --- | --- |
| Relational store | `insert`/`get`(pk)/`delete`/`count`/`snapshot`; schema-aware: reference existence, `unique`, cascade/restrict on delete (`_check_insert` :234, `_plan_delete` :267); atomic `commit_batch(inserts, deletes)` validating against a scratch overlay (:200) | the shared `ciac-sim` world (Pillar 2) becomes the reference restatement; schema arrives via the SimPlan's table topology |
| **`db.update` (extension)** | **not implemented — added this arc, all five targets** | attribute-level update by pk + filtered update, matching the production verb's semantics |
| **read verbs (extension)** | `count` exists; the `query`/filtered-read subset the production verbs actually lower to is completed where handler-reachable | scope fixed at M1 by scanning the corpus's actual verb usage, not speculative SQL |
| Broker | ordered per-subject log; **independent per-(subject, group) cursors** (`take_next`/`drain`/`pending_count` :340) — real fan-out | the narrow worlds' single-consumer queue is replaced, not extended; this is the structural upgrade |
| Virtual clock | `now_ms`/`advance_by` (:400); TTLs and schedules measured against it | Rust's `clock.rs` already exists in ciac-sim; the three restatements gain a clock struct |
| Cache | redis-shaped `get`/`set(ex=)`/`delete`; TTL vs virtual clock (:418) | |
| Object store | `put`/`get`/`delete`/`list` (:453) | |
| Email | `send` records messages (:474) | observable via new expect kind (Pillar 5) |
| Search | `index`/`search` (case-insensitive substring over JSON)/`delete` (:485) | substring semantics ported exactly — fidelity to the fake, not to a search engine |
| External HTTP | fixture-driven client consuming `GivenHttpResponse` (:511-545) | fixtures already in the scenario schema's `given.external_http` |
| Auth | `issue`/`verify` by claims lookup, virtual-clock expiry, **no crypto** (:555-564, disclosed) | retires the auth-refusal branch in all four gates (Pillar 6) |
| Delivery/retry | `deliver`/`deliver_counting_attempts` (:646/:669): worker retry, lost-ack redelivery | narrow targets have retry (vertical-slice's ProcessOrder:3 proves it); redelivery + fan-out generalize it |
| Failure engine | `error` action only, occurrence-counted, first-match-wins, unmatched-rule reporting (SIM0007) | **unchanged** — parity, not expansion; Delay/Timeout/Lose/Duplicate/Disconnect stay parse-but-refuse |

Everything in the left column is contract; the two bold rows are
the deliberate superset (without them the flagship stays refused —
order-system's named refusal reasons are `auth` plus unguarded
`cache.delete`/`cache.set`/`db.count`/`db.update`, and closing
three of those four without `db.update` would leave the flagship
exactly where it started). What is *not* contract: Python's
internal architecture. The ports restate behavior, not classes;
the conformance oracle is Pillar 7's shared scenario corpus, which
is indifferent to how a world is shaped and merciless about what
it answers.

### The reference semantics, family by family

The behaviors the corpus will assert, written down now so five
implementations answer one specification rather than five readings
of Python source. Where world.py's behavior is subtle, the subtle
part is stated:

- **Relational insert**: validates against the *committed* state
  plus schema — every `Reference<T>` field must name an existing
  referent row; every `unique` field must not collide; violation
  raises the family's checked error (which a `failures` rule can
  also inject via `db.commit`). Row storage is by table name,
  schema-agnostic beyond the checks (extra keys pass through —
  the fake stores documents, the *checks* are relational).
- **Relational delete**: plans before it acts — `_plan_delete`
  semantics: restrict-on-delete referents abort the whole delete;
  cascade referents are collected transitively and deleted in the
  same commit. The plan-then-apply shape is contract because it
  is observable (a restricted delete leaves *everything*,
  including cascade candidates already visited).
- **`db.update` (new, specified here)**: by-pk attribute update —
  missing pk is the checked error; updated fields re-validate
  `unique` (excluding self) and reference existence; unspecified
  fields persist. Filtered update (where-clause form) updates
  every matching row under the same per-row validation, count
  observable via `expect.row`. No upsert semantics — the
  production verb has none.
- **`commit_batch`**: the transaction fake — inserts and deletes
  validated together against a scratch overlay of committed
  state (inserts see earlier inserts in the same batch; deletes
  see the overlay too); any violation, or an injected `db.commit`
  failure rule, discards the entire overlay. After 26, this is
  also exactly what production Rust does, which is the point.
- **Broker cursors**: one ordered log per subject; each
  (subject, group) pair holds an independent cursor;
  `take_next(subject, group)` returns the next unconsumed message
  for that group without disturbing others; a message is
  redelivered to a group iff its delivery was not acked (the
  lost-ack case `deliver_counting_attempts` models); `drain` is
  per-cursor, `pending_count` per-cursor. Two workers in one
  group share a cursor (queue-group semantics); two groups on one
  subject each see every message (fan-out).
- **Worker retry**: a handler error re-enqueues for the same
  group up to the plan's retry budget, attempts counted per
  worker for `expect.worker_attempts` — semantics already proven
  by the vertical slice's ProcessOrder:3, now generalized to
  coexist with fan-out (attempts count per group, not globally).
- **Cache**: `set` with optional TTL stamps expiry against the
  virtual clock; `get` past expiry is a miss *and removes the
  entry* (matching world.py's lazy expiry); `delete` is
  unconditional. No eviction beyond TTL, no size limits.
- **Object store / email / search**: store is a keyed byte/JSON
  map with `list` returning keys in insertion order; email
  appends `{to, subject, body}` records in send order; search
  `index` upserts a document by id, `search` returns documents
  whose serialized JSON contains the query case-insensitively,
  in indexing order. All three are deliberately naive — the
  fidelity-boundary notes say so in each world.
- **External HTTP**: `given.external_http` fixtures are consumed
  in declaration order per (fixture-key) match; an unfixtured
  call is the checked error (a simulation must declare its
  outside world — silence would be nondeterminism smuggled in);
  consumption counts observable via `expect.http_calls`.
- **Auth**: principals are scenario data; `verify` succeeds iff
  the token maps to a seeded/issued principal and the virtual
  clock is before its expiry; scopes flow to the same
  enforcement code production uses above the validator. No
  signatures anywhere, stated in the disclosure comment ×5.
- **Failure engine**: unchanged contract restated for
  completeness — `error` action, `(effect, subject?, occurrence?,
  phase)` selectors, first-match-wins in declaration order,
  occurrence counters per (effect, subject), unmatched rules
  reported (SIM0007). Every new `*_checked` fake routes through
  it with its family's effect name, which *widens the failure
  vocabulary's reach* (a scenario can now fail `cache.set` on the
  third occurrence) without widening its action set.

### What "identical outcomes" means, precisely

The ×5 identity harness compares the runners' one-line outcome
JSON — business counts, expect-failure reports, unmatched-rule
lists — after canonical ordering (keys sorted, lists in the
schema's declared order). It does not compare logs, timings, or
world-internal state dumps: targets remain free in everything a
scenario cannot observe, which is the same claim boundary
docs/simulation.md already draws for the narrow slice, held
deliberately as the surface grows.

## Pillar 2 — The vendored-crate lever: deepen once, Rust inherits

The Rust backend does not restate the simulation world — it vendors
it. `crates/ciac-backend-rust/src/lib.rs:49-52` embeds
`ciac-sim`'s `cron.rs`/`failure.rs`/`scenario.rs`/`world.rs`
verbatim via `include_str!` and mounts them as sibling modules of
the generated `sim_runner` binary (:362-366). The consequence
shapes the whole arc: **M2–M3 deepen
`crates/ciac-sim/src/world.rs` itself** — ordinary, unit-testable,
clippy-checked workspace Rust — and the Rust target inherits every
new fake at its next generation, with no template surface beyond
the world-guard leaves and runner wiring (M4).

This assigns `ciac-sim`'s world a second job it has quietly had
since v0.17 M11 and this arc makes official: it is the **reference
restatement** — the executable specification the three
hand-restated worlds are checked against (via the corpus, Pillar
7), and the place where behavioral questions get settled first.
The deepening order inside the crate:

- **M2 — the stateful core**: relational store to the reference
  contract (schema-aware inserts/deletes/cascades from the
  SimPlan's topology, `get`/`update`/`delete`/`count`, atomic
  `commit_batch` with scratch-overlay validation — the fake whose
  semantics 26's atomicity fix makes honest to mirror), broker
  replaced with the per-(subject, group) cursor log, and the
  existing `clock.rs`/`schedule.rs` wired through the world rather
  than beside it. `FakeDatabase`'s current `find_where` stays as
  the runner-facing row-assertion query it already is.
- **M3 — the peripheral fakes**: cache (TTL against the clock),
  object store, email, search (substring semantics ported exactly),
  external HTTP (fixture consumption moved down into the world so
  all runners share one behavior), auth (claims-lookup, clock
  expiry, no crypto — with the disclosure comment ported verbatim
  from world.py, same honesty, same words).

Each fake lands with unit tests in the crate (the narrow world has
them; the deep world keeps the discipline) plus its corpus
scenario (Pillar 7). The crate's public surface grows accordingly
and `dump_plan`/`scenario_fixtures` stay green throughout —
`SimWorld`'s existing method signatures (`db_insert_checked`,
`publish_checked`, `unmatched_failure_rules`) do not break; new
capability methods arrive beside them following the same
`*_checked` failure-gate naming.

### The world's method surface, as drafted

The `SimWorld` methods the guards and runners will call — drafted
here so the restatements translate a signature list, with the
effect name each routes through the failure engine in parentheses:

```text
// relational (M2)
db_insert_checked(table, row)                  (db.commit — existing)
db_update_checked(table, pk, changes)          (db.commit)
db_update_where_checked(table, filter, changes)(db.commit)
db_delete_checked(table, pk)                   (db.commit)
db_get(table, pk) / db_count(table, filter)    (db.read)
commit_batch_checked(inserts, deletes)         (db.commit)
seed_db(table, row) / find_where(table, filter)   [runner-only, unchecked]

// broker (M2)
publish_checked(subject, payload)              (broker.publish — existing)
take_next(subject, group) / ack / nack         (broker.consume on take)
pending_count(subject, group) / queues_empty()    [runner-only]

// clock (M2)
now_ms() / advance_by(ms)                         [unchecked]

// peripherals (M3)
cache_get/​cache_set(ttl?)/​cache_delete          (cache.get/.set/.delete)
store_put/store_get/store_delete/store_list    (store.put/.get/.delete/.list)
email_send_checked(msg)                        (email.send)
search_index/search_query/search_delete        (search.index/.query/.delete)
http_request_checked(fixture_key, req)         (http.request)
auth_issue(principal, exp) / auth_verify(token)(auth.verify)

// bookkeeping (existing, unchanged)
unmatched_failure_rules()
```

Runner-only methods (seeding, row assertion, pending counts,
quiescence peeks) take no failure check by design — they are the
scenario's instrumentation, not simulated effects, the same
distinction the narrow worlds already draw between
`db_insert_checked` and `seed_db`/`find_where`. Effect-name
strings are contract (scenarios reference them in failure
selectors) and land in docs/simulation.md's scenario reference as
a table in M1.

### Two references, one answer sheet

An honesty note the arc's structure requires: Python does *not*
consume `ciac-sim` — pyrunner's world.py is its own
implementation, embedded in the CLI via `include_str!` and written
in Python. After this arc there are therefore two
reference-quality implementations (world.py for Python, world.rs
for the vendoring Rust target and as the restatement spec) plus
three restatements — and the two references can drift from *each
other* exactly the way restatements drift from a reference. The
plan does not pretend otherwise and does not unify them (a
Python-executes-Rust bridge would be machinery invented to avoid
writing a fake twice — the wrong trade at this surface). The
mitigation is the same corpus that disciplines the restatements:
every scenario runs on Python too, and the ×5 identity check binds
world.py and world.rs to one answer sheet with no privileged
member. Where M2–M3 discover that world.py's behavior is
underspecified or accidental (the likeliest place: edge ordering
in cascade deletes, TTL boundary inclusivity), the resolution is
decided once, recorded in the family's reference-semantics entry
in this file, and applied to **both** references — the contract
outranks both implementations.

### The delivery loop, specified

Deterministic outcomes across five independently written runners
require the delivery *order* to be contract, not accident. The
rules, fixed here (and matching what the existing runners already
do implicitly for the narrow slice, now stated because fan-out
and peripherals widen the choice space):

1. A `drain` step processes subjects in the SimPlan's declaration
   order; within a subject, groups in declaration order; within a
   (subject, group) cursor, messages in log order.
2. Delivery of one message runs its consumer handler to
   completion (including that handler's own publishes, which
   append to logs but are not delivered until the loop reaches
   them — breadth over depth, matching the existing runners) with
   retry attempts consumed immediately and consecutively up to
   the budget.
3. `advance` fires due schedule entries in (due-time, declaration
   order) — the existing `schedule.rs` semantics — and does not
   implicitly drain; scenarios say `drain` when they mean it
   (the existing convention, unchanged).
4. Quiescence means: every cursor at log head, no due schedule
   entries at current virtual time, no retry budget mid-spend.

The ×5 identity harness additionally canonicalizes outcome JSON
before comparison — sorted keys, and **integer-valued numbers
emitted as integers** on every target (the cross-language trap:
a counter that one runtime serializes as `3` and another as
`3.0` would fail identity on formatting, not behavior; each
runner's outcome serialization is checked against this rule at
its milestone, which costs a line per runner and buys the
harness its bluntness).

## Pillar 3 — The world-guard economy: how per-verb guards scale

Every faked verb needs a production-code seam: the generated
handler branches to the world when simulating and to real
infrastructure otherwise. Today each narrow backend guards two
leaves (`db_insert` tail, publish). This arc multiplies the guard
count by roughly six per target, and the economics matter more
than any single guard:

- **The scan is the ledger.** `ciac_codegen::lower::scan`'s
  `Needs::unguarded_verbs` is already the machine-readable list of
  which verbs a target has *not* guarded — it is what the
  `unsupported_sim_capabilities` gates print. The arc's progress
  metric is that list shrinking to empty per target, and the gate
  functions themselves are the always-current scoreboard: no
  parallel tracking document, no drift between claim and code.
- **Guard shape is idiom-fixed per target, decided once.** Rust:
  `if let Some(world) = &self.world { ... } else { ... }` (exists);
  TypeScript: `if (world) { ... }`; Go: `if w := s.World; w != nil`;
  Java: `if (world != null)`. Each new guard copies its target's
  existing two guards — the pattern was frozen in the M9 slices
  precisely so this arc could be mechanical about it.
- **The transaction leaf is special on every target.** Under
  simulation the deepened world finally gives `transaction {}` an
  *atomic* fake (`commit_batch` semantics: violations roll the
  whole batch back) — retiring the "degrades to non-atomic /
  guarded no-op under simulation" disclosures that
  backends.md/simulation.md carry for TS/Go/Java, and giving Rust's
  newly atomic production path (26 M1–M2) a fake with matching
  semantics. The sim-side atomicity scenario (a failure-injected
  `commit_batch` asserting zero partial rows *in the fake*) joins
  the corpus and must agree with 26's live rollback proof — fake
  and real asserting the same property is exactly what the
  fidelity ratchet exists to check.
- **Guards are golden-visible and equivalence-checked.** Every
  guard lands in generated code, so every guard is reviewed as a
  golden diff against the standing rule that the `else`/production
  branch is byte-identical to the pre-guard emission — the same
  invariant discipline 26's Pillar 1 used, applied ~24 more times
  (six-ish verbs × four targets).

### The guard inventory, per target

The verbs whose lowering arms gain guards, common to all four
targets (each target's exact arm names differ by lower.rs; the
list is the shared scan's verb vocabulary, which is what
`unsupported_sim_capabilities` prints today when it refuses):

| Verb family | Guarded arms | World call |
| --- | --- | --- |
| db.insert | already guarded (M9 slices) | `db_insert_checked` |
| db.update / filtered update | update arm(s) | `db_update_checked` / `db_update_where_checked` |
| db.delete | delete arm | `db_delete_checked` |
| db.get / db.query subset / db.count | read arms | `db_get` / `find_where`-backed reads / `db_count` |
| transaction | tx leaf (special: atomic fake) | `commit_batch_checked` |
| publish | already guarded | `publish_checked` |
| cache.get/set/delete | cache arms | `cache_*` |
| store.put/get/delete/list | store arms | `store_*` |
| email.send | email arm | `email_send_checked` |
| search.index/query/delete | search arms | `search_*` |
| http.post/request | external-http arm | `http_request_checked` |
| auth (validator seam) | middleware/validator, not a verb arm | `auth_verify` |

Two structural notes: the transaction leaf composes with the
per-verb guards (inside a guarded world-branch transaction, inner
db verbs accumulate into the batch rather than hitting their own
world calls individually — the runner-side batch assembly mirrors
how Python's `_FakeSession` defers to `commit_batch`; each
target's mechanism is decided at its milestone against its
existing tx-guard shape and recorded), and the read verbs are the
one family where guard-vs-fake fidelity is subtle (production
reads return typed rows; the world returns JSON documents — the
guard branch must decode through the same schema types the
production branch uses, so a type-level mismatch is a compile
error, not a silent divergence; this decode-through-schema rule
is contract for all four targets).

## Pillar 4 — The three restatements: TS, Go, Java

The hand-restated worlds (`world.ts.j2`, `world.go.j2`,
`World.java.j2`) grow from ~150-line narrow fakes to full worlds.
The lessons the narrow era already taught get promoted to rules:

- **Self-containment is a requirement, not a style.** Java's world
  learned this live in 25 M9 (its `Schemas.MAPPER` dependency broke
  on `db`+`queue` programs with no records — fixed by giving the
  world its own identically-configured mapper). Rule: a world
  template may depend on its language's standard library and the
  target's already-always-present runtime deps, and nothing else;
  each world's serialization config is restated locally with a
  comment naming its twin.
- **Behavioral comments port verbatim.** Where world.py discloses a
  boundary (auth's no-crypto note, search's substring semantics),
  the restatements carry the same sentence — five copies of one
  disclosure is the acceptable cost of five self-contained worlds;
  five *different* disclosures would be drift in its most
  camouflaged form.
- **Structure may diverge; answers may not.** Go will want the
  cursor log as a struct with mutex where Python has a class and
  Rust has the reference restatement; fine. The corpus (Pillar 7)
  is the only arbiter that counts, and every restatement decision
  that produces an observable difference is by definition a bug in
  the restatement, not a fact about the language.
- **Runners grow with their worlds.** Each target's sim runner
  (`sim_runner.ts.j2`, `sim_runner.go.j2`, `SimRunner.java.j2`,
  Rust's `sim_runner.rs.j2`, Python's `scenario_runner.py`) gains
  the delivery loop generalizations (fan-out via per-group cursors,
  redelivery) and the new expect handlers (Pillar 5). The
  child-protocol contract — one JSON outcome line on stdout —
  does not change; the CLI drivers in commands.rs do not change
  this arc beyond passing through the new expect kinds' failures
  (28UpdatePlan.md is the arc that touches driver topology).

### Restatement starting points, per target

What each restatement inherits and what it replaces — the delta
each of M6–M8 actually executes:

- **TypeScript** (`world.ts.j2` + `sim_runner.ts.j2`): narrow
  `SimWorld` class with `FakeDatabase`/`FakeQueue`/`FailureEngine`
  and `dbInsertChecked`/`publishChecked`. The queue is replaced by
  the cursor log; the database generalizes in place (its row maps
  are already `Record<string, unknown>[]` per table — the checks
  are what's new); peripherals are new classes on the same file's
  pattern. The runner already owns request dispatch and drain;
  it gains group-aware delivery and the new given/expect handlers.
  TS's structural-typing looseness is the drift risk to watch —
  the decode-through-schema rule leans on the generated zod/type
  layer the production branch already uses.
- **Go** (`world.go.j2` + `sim_runner.go.j2`, `cmd/sim_runner`):
  narrow `World` struct with `DBInsertChecked`/`PublishChecked`/
  `SeedDB`/`FindWhere`/`DrainQueue`, mutex-guarded. Same
  replacement shape; Go's `map[string]any` rows meet the
  decode-through-schema rule via the generated structs'
  `json.Unmarshal` — a type-checked path that exists since 24 M2.
  The cursor log wants a small struct with per-key cursor map;
  redelivery integrates with the runner's existing attempt
  counting.
- **Java** (`World.java.j2` + `SimRunner.java.j2`): narrow `World`
  with its own self-contained `MAPPER` (the 25 M9 lesson,
  pre-paid), `dbInsertChecked`/`publishChecked`/`seedDb`/
  `findWhere`, nested `FailureEngine`. Peripherals as nested or
  sibling classes in the same generated file (one file per world
  stays the rule — restatements must not sprawl into packages);
  the runner's `AnnotationConfigApplicationContext` +
  standalone-MockMvc arrangement is untouched except for new
  given/expect records mirroring the schema additions. Jackson's
  `convertValue` is the decode-through-schema mechanism, already
  in place for inserts.

In all three, the *world file replaces its predecessor wholesale*
— no compatibility shim, no dual paths: nothing outside the
generated project depends on a world's internals, and the guards
+ runner regenerate together with it. The narrow worlds' verbatim
disclosure comments (scope notes, single-consumer caveats) are
superseded by the deep worlds' new comment set, drafted once in
the shared crate and ported per Pillar 4's verbatim rule.

## Pillar 5 — Scenario schema: observing what the worlds now fake

A faked capability that no scenario can observe is dead weight.
The scenario schema (crates/ciac-sim/src/scenario.rs, versioned:
`SCENARIO_VERSION = 1`, structural `validate()` at :190) currently
speaks five step kinds (`request`/`publish`/`advance`/`drain`/
`expect`) and five expect kinds (`response`/`row`/
`worker_attempts`/`job_runs`/`quiescence`). The deepened worlds
need, additively:

- `given.cache` (seed entries with optional TTL), `given.store`
  (seed objects), `given.search` (seed index docs) — parallel to
  the existing `given.db`/`given.external_http`.
- `expect.email { to?, subject_contains?, count }` — the email
  fake's observation surface.
- `expect.cache { instance, key, present, value? }`.
- `expect.object { store, key, present }`.
- `expect.search_hits { index, query, count }`.
- `expect.http_calls { fixture, count }` — asserting fixture
  consumption, closing the loop on `given.external_http`.

All additive; no existing scenario changes meaning. The versioning
decision is pre-registered (Open question 1): bias is to keep
`SCENARIO_VERSION = 1` and treat unknown-field rejection as the
compatibility boundary (scenarios are consumed by the same-version
CLI that generated the runner — there is no cross-version scenario
ecosystem to protect yet), with a bump to 2 only if a
non-additive change proves necessary. Whichever way it lands, the
schema, the five runners, and docs/simulation.md's scenario
reference move in the same milestone (M1 for the schema and
reference; runners adopt as their targets deepen).

### The new kinds, as drafted

Representative JSON for the additions (field names final at M1;
shapes are the plan's):

```text
"given": {
  "db": [ ... existing ... ],
  "cache": [ { "instance": "sessions", "key": "u1", "value": {...},
               "ttl": "30m" } ],
  "store": [ { "instance": "uploads", "key": "a.png",
               "value_base64": "..." } ],
  "search": [ { "instance": "catalog", "id": "p1",
                "doc": {...} } ],
  "external_http": [ ... existing ... ],
  "failures": [ ... existing ... ]
}

{ "expect": { "email": { "to": "ops@example.com",
                          "subject_contains": "reconciled",
                          "count": 1 } } }
{ "expect": { "cache": { "instance": "sessions", "key": "u1",
                          "present": false } } }
{ "expect": { "object": { "store": "uploads", "key": "a.png",
                           "present": true } } }
{ "expect": { "search_hits": { "instance": "catalog",
                                "query": "widget", "count": 2 } } }
{ "expect": { "http_calls": { "fixture": "payments-ok",
                               "count": 3 } } }
```

Design rules the drafts encode: every new given/expect names its
capability *instance* (the language has named instances since
v0.5; scenarios address them the way programs do); every expect is
a point assertion with an exact count or presence bool (the
exact-outcome discipline — no matchers, no ranges); and
`value_base64` on store seeds keeps the scenario file
JSON-clean for binary objects (the one place the schema
acknowledges non-JSON payloads, matching the object store's
byte-map semantics).

## Pillar 6 — FakeAuth and the retirement of the auth refusal

Every narrow gate's first branch refuses any program declaring
`auth`, with the same recorded reason: validating a real signed
token needs real cryptography. Python's answer (world.py:555-564)
is a `FakeAuth` that skips crypto entirely — principals are
scenario data (`request.as { sub, scopes }`), tokens are looked up,
expiry is virtual-clock — and the honest disclosure that this
tests *authorization logic* (scopes, principals, per-endpoint
enforcement) and not *authentication cryptography* (which 26's
OAuth2 rig and the generated scope tests now cover no-infra on
every target, making this division of labor cleaner than it was
when the refusal was written: sim fakes authorization; the scope
suites prove authentication).

The port: the shared world (M3) and the three restatements
(M6–M8) implement claims-lookup auth; the runners map `request.as`
principals through it; the guard seam on each target is wherever
its generated auth middleware/validator sits — under simulation
the validator consults the world instead of verifying signatures,
with the production branch byte-identical (the standing guard
invariant). Then the refusal branch is deleted from all four
`unsupported_sim_capabilities` functions — the single largest
category of refused programs (every `auth`-bearing example)
becomes simulatable in one step, which is why it gets its own
corpus scenarios: scope-denied (403 outcome asserted in sim),
scope-granted, and expiry-across-`advance`.

The seam's per-target placement (final per Open question 5, the
candidates named now): Python's validator dependency is already a
constructor-injected seam (v0.17 M3 built the dependency seams
explicitly — the fake validator slots where the real one does);
Rust's validator is a function the middleware layer calls — the
guard branches inside it on the world's presence, same shape as
the db guards; TypeScript's middleware chain takes the validator
at app construction, where the runner already builds the app;
Go's middleware closure captures the validator at router
construction (`sim_runner`'s router build passes the world-backed
one); Java's Spring Security filter arrangement is the delicate
one — the candidate is the same `ObjectProvider<World>` null-check
pattern every other Java guard uses, inside the token-validation
component rather than the filter chain itself, keeping Spring's
wiring untouched. In every case the scope-*enforcement* code above
the validator runs unmodified — simulation exercises the real
authorization logic against fake authentication, which is the
whole design.

One interaction pre-named: `request.as` already exists in the
scenario schema (`Principal { sub, scopes }`) and the narrow-era
runners accept it only for programs without `auth` (it decorates
requests no validator inspects). After this pillar, the same field
drives the real enforcement path — existing scenarios keep their
meaning (no `auth` declared, nothing changes), and `auth`-bearing
programs get the principal honored end-to-end. No schema change,
one behavior promotion, called out in the scenario reference.

## Pillar 7 — Conformance by corpus, gate retirement, and the Full flip

**The corpus is the specification.** The shared scenario corpus —
today two files (`sim/vertical-slice.ciac-sim.json`,
`sim/virtual-week.ciac-sim.json`) — grows to cover every faked
family: per-family scenarios plus the flagship, every one running
on **all five targets** with identical outcome JSON required. This
cross-target identity is the conformance mechanism for the
restatements, held in the equivalence machinery alongside the
existing typed-handler cases — deliberately the same trick the
factory arcs used (behavior oracles over code review) pointed at a
new surface. The planned corpus, authored across M2–M9 as its
families land:

| Scenario file (sim/) | Program (examples/) | Families exercised | Authored in |
| --- | --- | --- | --- |
| vertical-slice.ciac-sim.json | sim-vertical-slice.ciac | insert, publish, worker retry — **existing; outcomes frozen** | v0.17 |
| virtual-week.ciac-sim.json | sim-broker-slice.ciac | cron, virtual time at scale — **existing; outcomes frozen** | v0.17 |
| relational-depth.ciac-sim.json | new sim-relational.ciac (or domain-orders) | update/delete/count, uniques, references, cascade + restrict | M2 |
| atomic-batch.ciac-sim.json | same program | commit_batch rollback under injected db.commit failure | M2 |
| fanout.ciac-sim.json | new sim-fanout.ciac (two groups, one subject) | cursor independence, redelivery, per-group attempts | M2 |
| cache-ttl.ciac-sim.json | order-system or a minimal cache program | set/get/delete, TTL expiry across `advance` | M3 |
| peripherals.ciac-sim.json | a program declaring store/email/search | store put/list, email counts, search hits | M3 |
| http-fixtures.ciac-sim.json | an external_http program | fixture order, consumption counts, unfixtured-call error | M3 |
| auth-scopes.ciac-sim.json | oauth-echo or order-system | scope-denied/granted, expiry-across-advance | M3 |
| order-system.ciac-sim.json | order-system.ciac | **the flagship: everything at once** | M4 (outcomes frozen there) |

Program choices marked "or" are decided at authoring against the
standing criterion (fewest new moving parts, recorded); new
`sim-*.ciac` example programs are added only where no existing
example exercises a family cleanly — each is a real example that
must verify on all five targets like any other, so the corpus
grows the example matrix deliberately, not incidentally.

**Gate retirement is observable, not declared.** As each target's
guards land, its `unsupported_sim_capabilities` output shrinks; a
target is *done* when the function provably returns empty for
every checked-in example (a unit test iterates the corpus and
asserts emptiness — the test that today asserts order-system's
refusal reasons inverts into the test that asserts there are
none). Then the flip: `SimSupport::Narrow { .. }` →
`SimSupport::Full`, per target, in the same milestone its
emptiness test lands (truth over ceremony — no waiting for a
group photo at M9; targets.json and the docs status table update
per milestone, and the checked-in-JSON test keeps them honest).

**The record/replay conflation, fixed.** `sim_inner` currently
refuses `--record/--replay` for `Narrow` targets (commands.rs
:1158) — meaning a naive flip to `Full` would route replay flags
into four runners that implement no replay. The fix (M1, before
any flip exists to get it wrong): record/replay support becomes
its own explicit field on the sim surface (`TargetInfo`-level,
e.g. `sim_replay: bool` — exact shape decided against `TargetInfo`'s
existing style), `sim_inner` consults it instead of the depth
enum, Python sets it true, the four others false, and the ledger's
record/replay row stays honestly open (Explicit cuts) instead of
becoming accidentally, falsely closed. The docs state the final
shape plainly: five targets simulate fully; one target records and
replays.

## Pillar 8 — The flagship, and the fidelity ratchet at depth

**order-system on five targets** is the arc's acceptance test
because it is the project's own worst counterexample today, and
because it exercises the deepened surface end-to-end in one
program: auth with scopes, cache set/delete, `db.update`,
`db.count`, transactions, workers, publish. The scenario asserts
business outcomes (orders placed, cache invalidated, worker
completions, scope-denied requests refused) with exact counts; the
same file runs ×5; the outcomes match or the arc is not done. It
also becomes CI's deepened `generated-sim` row alongside
sim-vertical-slice, keeping the whole surface exercised on every
push — and the two standing canonical outcomes remain byte-exact
throughout the arc as the no-regression floor.

The flagship scenario's outline, drafted now (exact counts frozen
at M4 authoring against the program's real topology): given — a
seeded catalog and a principal with `orders:write`; steps — N
order-placing requests as the principal (one denied via a
scope-less principal, asserted 403), an injected `db.commit`
failure on one order (asserted: error envelope, zero partial rows
via `expect.row`, cache untouched), `advance` past the
reconciliation job's schedule, `drain`; expects — placed-order row
counts, `expect.cache` present-false for the invalidated key,
`expect.worker_attempts` for the retry the failure caused,
`expect.job_runs` for reconciliation, `expect.quiescence`. One
scenario, every family the program touches, five identical answer
sheets — the sentence the arc exists to make true, in one file.

**The ratchet extends to the new fakes where reality is cheap.**
The v0.17 fidelity ratchet compared fake vs real for the vertical
slice; this arc adds ratchet rows where a real counterpart can be
stood up without ceremony: relational semantics vs SQLite
(zero-Docker — cascades, uniques, update/delete/count, batch
rollback), cache TTL vs a real redis under compose (delegated,
same honesty as the existing Docker-delegated rows), broker
fan-out vs real NATS queue groups (delegated). Families with no
cheap real counterpart (email, search-as-substring, claims-lookup
auth) get the opposite treatment — an explicit *fidelity boundary*
note in docs/simulation.md stating what the fake deliberately is
not, ported verbatim ×5 per Pillar 4's comment rule. The ratchet's
job at depth is unchanged: catch the fake drifting from the real
where a real exists, and say so where one does not.

## Implementation map

| Area | Changes |
| --- | --- |
| `crates/ciac-sim/src/world.rs` | the deepening: schema-aware relational store + `update`/read verbs, cursor-log broker, TTL cache, object store, email, search, http fixtures, auth; unit tests per fake |
| `crates/ciac-sim/src/scenario.rs` | additive given/expect kinds (Pillar 5); version decision per Open question 1 |
| `crates/ciac-sim/src/plan.rs` | table topology carried for the schema-aware store (extend `SimPlan` if the current shape lacks constraint data) |
| `crates/ciac-codegen` (`TargetInfo`) | `sim_replay` (or equivalent) field; `SimSupport` flip sites ×4 as targets complete |
| `crates/ciac-backend-rust` | world-guard leaves for all newly faked verbs in lower.rs; runner wiring for new expects/delivery; auth seam |
| `crates/ciac-backend-ts`, `-go`, `-java` | `world.*.j2` full restatements; guard leaves in each `lower.rs`; runner template growth; auth seam |
| `crates/ciac-backend-python` | `db.update` + read-verb fakes in pyrunner's world/session; auth already Full — corpus adoption only |
| `sim/pyrunner/world.py` | `update`/query-subset closure of the `_FakeSession` disclosure |
| `crates/ciac/src/commands.rs` | `sim_inner` consults the replay flag instead of the depth enum; drivers pass through new expect failures |
| `sim/*.ciac-sim.json` | per-family corpus scenarios + `order-system.ciac-sim.json` |
| `tests/` | corpus-runs-×5-identical harness; gate-emptiness tests ×4; sim-side atomicity case |
| CI | `generated-sim` row gains order-system ×5 |
| docs | simulation.md rewrite (status table collapses), backends.md ledger rows closed, scenario reference for new kinds |

## Capability parity checklist

The arc-end matrix — every row must read identically across the
five target columns, which is why the table has one column:

| Surface | All five targets at M9 |
| --- | --- |
| Relational: insert/get/update/delete/count, uniques, references, cascade/restrict | faked, corpus-asserted |
| Atomic `commit_batch` (transaction fake) | faked; agrees with 26's live rollback semantics |
| Broker: per-(subject, group) cursors, fan-out, redelivery | faked, fanout scenario green |
| Virtual clock; TTL cache; object store; email; search; external-HTTP fixtures | faked, per-family scenarios green |
| Auth: claims-lookup principals, scope enforcement, clock expiry | faked; auth refusal branch deleted |
| Worker retry + attempt counting | generalized to coexist with fan-out |
| Failure injection | `error`, occurrence-counted, every `*_checked` effect reachable |
| `unsupported_sim_capabilities` | provably empty on the corpus (×4; Python has no gate) |
| `SimSupport` | `Full` ×5 |
| Record/replay | Python only — its own flag, its own ledger row, stated everywhere the support matrix appears |
| Corpus (incl. flagship order-system) | identical outcomes ×5 |
| Canonical anchors | byte-exact, untouched |

## Determinism and supply chain

The fakes are deliberately dependency-free: maps, vectors, string
matching, and the existing virtual clock — no new runtime or
test-scoped dependency lands in any generated project on any
target for any fake (a property the narrow worlds already hold and
the deep worlds keep; asserted at review, recorded if any target's
idiom forces an exception). The shared crate gains no new
dependencies. CI's `generated-audit` job (26 M6) therefore sees no
new surface from this arc — the cheapest possible supply-chain
story for the largest behavioral arc since v0.17. Scenario JSON
remains hand-authored, deterministic fixtures; runner output
remains the one-line outcome contract; virtual time remains the
only clock (no wall-clock reads in any fake — the review grep that
guards this on the narrow worlds extends to the deep ones).

## Diagnostics, gating, and docs impact

Refusal messages get *better* before they get rare: the gate
output format is unchanged, but every reason string it can emit is
re-derived from the shrinking unguarded set, so mid-arc a
partially-deepened target names exactly what still blocks a
program — the gates' precision is what makes incremental shipping
non-confusing. No new CIAC codes. SIM codes: the existing registry
(SIM0001–SIM0009) covers the deepened worlds' outcomes
(SIM0007's unmatched-rule discipline extends to every new
`*_checked` fake unchanged); one addition is expected for
replay-not-supported when `--record/--replay` hits a
`sim_replay: false` target (today that refusal rides the Narrow
check this arc removes) — reserved as SIM0010, wording fixed at
M1. Docs: simulation.md is substantially rewritten (status table,
scenario reference, fidelity boundaries); backends.md's Open
table loses its largest row and its per-target sim paragraphs
compress; the "degrades to non-atomic under simulation"
disclosures for TS/Go/Java retire at their milestones.

## Relationship to the forecast documents

This arc is the direct execution of the punch-list's first Tier 1
item via the discussion's explicit "deepen to match Python"
decision, against the surface v0.17 built and 23/24/25 M9
narrowly extended. It consumes 26's outputs (atomic Rust
transactions to mirror, the Open ledger table to empty, scanning
CI to run under) and produces the precondition for 28UpdatePlan.md
(multi-service simulation orchestrates *worlds*; orchestrating
full worlds once beats orchestrating narrow ones and re-plumbing
for depth after — the sequencing argument recorded when the four
plans were laid out). 29UpdatePlan.md's onboarding narrative then
gets to describe simulation without an asterisk footprint, which
is half the reason it goes last.

## What this arc is predicted to cost

Predictions to be reconciled at M5 (for the Rust half, calibrating
the restatements) and in M9's retrospective (for the whole):

| Workstream | Predicted size |
| --- | --- |
| Shared-crate deepening (M2–M3) | world.rs grows ~4–5× (narrow ~200 lines → a full world in the several-hundred-to-thousand range, matching world.py's 787-line reference); unit tests roughly double the crate's test surface |
| Rust guards + runner (M4) | ~10 verb-arm guard edits + the validator seam + runner expect/delivery growth; golden churn on every db/cache/store/email/search/http-bearing Rust example |
| Each restatement (M6–M8) | the world template rewrite (~5× growth each) + the same ~10 guard edits + runner growth; per-target cost expected *below* Rust's M4 because the shared world settles every behavioral question first — the delta M5 measures |
| Python closure (M9) | smallest item: two verb families in an existing, tested fake |
| Corpus | ~8 new scenario files + up to 3 new small example programs, each verifying ×5 |

### Predicted golden churn

| Milestone | Expected churn |
| --- | --- |
| M1 | none (schema/crate/CLI internals; scenario fixtures only) |
| M2–M3 | none in generated projects (shared crate only — Rust inherits at M4's regeneration, not before) |
| M4 | Rust goldens: world/runner files wholesale + guard diffs on verb-bearing files; production branches byte-identical under review |
| M6/M7/M8 | same shape, one target each |
| M9 | Python pyrunner files (CLI-embedded, not golden-snapshotted) + docs + version churn |

### The config/env surface

None. No new environment variables, no new config file rows, on
any target — simulation remains a build-and-run-runner concern
wired through the world's presence, and the new fakes read
scenario data, not env. (Asserted at each milestone's review; any
exception is a recorded deviation.)

## Milestones

Nine milestones: contract and mechanics first, the shared crate's
two-stage deepening, Rust to full parity, a hard checkpoint, three
restatements in the factory's cheapening order, and the flagship
close. Every milestone ends with full workspace verification, the
standing golden review, the two canonical anchors byte-exact, and
a commit + push; Shipped notes append in place per convention.

1. **M1 — Contract, schema, and the replay decoupling.** The
   parity contract's final verb list fixed by scanning the example
   corpus's actual reachable verbs (the two extension rows scoped
   to what handlers really lower to — recorded as the contract
   table's final form in this file); scenario schema's additive
   given/expect kinds landed with structural validation + fixture
   tests; the version decision executed (Open question 1);
   `SimPlan` extended if the schema-aware store needs constraint
   topology it doesn't carry; the record/replay support flag
   introduced and `sim_inner` rewired to consult it (SIM0010
   reserved, wording fixed) — landing *before* any Full flip
   exists to get wrong. No world code yet: this milestone is the
   arc's contract, and it is deliberately small enough to review
   as one.

2. **M2 — The shared world, stage one: the stateful core.**
   `ciac-sim`'s relational store to the reference contract
   (schema-aware insert/delete with references, uniques, cascades;
   `get`/`update`/`delete`/`count`; atomic `commit_batch` with
   scratch-overlay validation), the cursor-log broker (fan-out,
   `take_next`/`drain`/`pending_count` semantics), clock/schedule
   wired through the world per the delivery-loop specification.
   Where the reference-semantics section proves underspecified
   against world.py's actual behavior, the resolution is decided,
   recorded in this file's Pillar 1 entries, and applied to both
   references (the Pillar 2 rule) — expected sites: cascade visit
   order, TTL boundary inclusivity, redelivery-vs-retry
   interleaving. Unit tests per behavior including batch rollback
   and two-group fan-out; the relational/atomic-batch/fanout
   corpus scenarios authored (runnable ×1–2 until targets deepen —
   the harness's per-scenario coverage ledger starts here);
   `scenario_fixtures` and `dump_plan` green; existing public
   surface unbroken; world.py adjusted only where a both-references
   resolution demands it, with pyrunner's fixtures extended to
   cover the adjustment.

3. **M3 — The shared world, stage two: the peripheral fakes.**
   Cache (TTL vs clock), object store, email, search (substring
   semantics ported exactly), external HTTP fixture consumption
   moved into the world, auth (claims-lookup, clock expiry,
   disclosure comment ported verbatim). Unit tests + corpus
   scenarios per family (TTL-across-advance, fixture-count,
   scope-denied/granted/expiry among them). The shared crate is
   now the complete reference restatement; the ratchet's SQLite
   relational rows land here (zero-Docker, runnable in-crate).

4. **M4 — Rust to full parity.** World-guard leaves for every
   newly faked verb in Rust's lower.rs per the guard inventory
   (pattern copied from the two existing guards; production
   branches byte-identical — golden-reviewed under 26's invariant
   discipline; the transaction leaf's world branch upgraded from
   the degraded per-verb shape to batch assembly against
   `commit_batch_checked`, retiring Rust's
   non-atomic-under-simulation disclosure); the auth seam under
   the generated validator; runner growth (new given/expect
   handlers, group-aware delivery, redelivery, outcome
   canonicalization rule adopted); the decode-through-schema rule
   enforced on every read-verb guard (a type error, not a test,
   is the first line of defense). Rust's gate-emptiness test
   lands and passes for the whole corpus; Rust flips `Narrow` →
   `Full` (`sim_replay: false`); targets.json + status table
   update in this milestone. Live: the full corpus × Rust with
   exact outcomes — including order-system's first-ever
   successful simulation on any compiled target (single-target
   acceptance; the ×5 sentence is M9's) — and both canonical
   anchors byte-exact.

5. **M5 — CHECKPOINT.** The go/no-go for the three restatements,
   in the factory tradition: measure Rust's actual M4 cost (guard
   count, runner delta, corpus failures found and fixed, wall
   time), reconcile against this plan's estimates, and decide —
   go (default: the restatements proceed on the measured-cheaper
   path), narrow-go (a family proves disproportionately expensive
   per target: descope it uniformly across all four with the
   ledger row split accordingly — uniform scope is non-negotiable
   even under descoping), or no-go (structural surprise:
   restatements halt, findings recorded, the arc re-plans — the
   outcome no one expects, pre-registered anyway). The checkpoint
   report lands in this file; the corpus-runs-×5 harness lands
   now (executing ×2: Python pending its M9 verb closure on some
   scenarios, recorded per-scenario) so the remaining milestones
   plug into a running scoreboard.

6. **M6 — TypeScript restatement.** `world.ts.j2` to the full
   contract (self-contained per Pillar 4's rules); guard leaves
   across TS lower.rs; runner growth; auth seam; gate-emptiness
   test; `Full` flip; the corpus × TS identical to Rust's
   outcomes. The first restatement is deliberately the
   structurally closest language to the corpus's JSON world —
   restatement drift, if the discipline has a hole, shows here
   cheapest.

7. **M7 — Go restatement.** Same shape as M6 (`world.go.j2`,
   mutex-guarded structs where idiom wants them, behavior
   identical); the corpus × Go. Go's existing `FindWhere`/
   `SeedDB`/`DrainQueue` surface generalizes rather than
   duplicates — the restatement replaces the narrow world, no
   compatibility shim (nothing external depends on a generated
   world's internals; the runner and guards regenerate with it).

8. **M8 — Java restatement.** Same shape (`World.java.j2` +
   `SimRunner.java.j2` growth, self-containment rule already
   proven there by the v0.25 mapper lesson); the corpus × Java.
   Java closes the restatement set; all four gates are provably
   empty; all five targets report `Full`; the status table's
   "narrow" column is gone from docs at this milestone, not M9 —
   truth lands when it becomes true.

9. **M9 — Python's closure, the flagship ×5, version, and the
   retrospective.** Python's residual verbs (`db.update`, the read
   subset) land in pyrunner's world/session, closing the
   `_FakeSession` disclosure; every corpus scenario now runs ×5
   and the harness asserts five identical outcome sets on every
   scenario including `order-system.ciac-sim.json` — the arc's
   acceptance sentence, executed. Remaining ratchet rows
   (compose-delegated redis/NATS) recorded; CI's `generated-sim`
   row extended; docs/simulation.md rewrite completed (fidelity
   boundaries section, scenario reference, the one-word status);
   backends.md Open-table row closed with proof; version
   **0.24.0 → 0.25.0** (workspace + pins, vscode manifest,
   language.md compiler parenthetical — language stays 1.0.0).
   Retrospective appended after a rule: measured restatement costs
   ×3 vs M5's calibration, corpus-found bugs by target (the
   restatement-drift scorecard — the arc's honesty metric), and
   the handoff to 28UpdatePlan.md.

### Per-milestone exit checklists

- **M1 exits when:** contract table finalized in this file; new
  given/expect kinds validated + fixture-tested; version decision
  recorded; SimPlan carries what the store needs; replay flag
  wired with Python true / others false and the Narrow-based
  refusal gone; SIM0010 reserved and worded.
- **M2 exits when:** relational + broker + clock behaviors
  unit-tested in-crate including batch rollback and two-group
  fan-out; corpus scenarios authored; existing crate surface
  unbroken; both canonical anchors untouched.
- **M3 exits when:** all peripheral fakes unit-tested with
  disclosure comments in place; per-family corpus scenarios
  authored; SQLite ratchet rows green in-crate; the shared world
  is contract-complete.
- **M4 exits when:** Rust gate-emptiness test green across the
  corpus; production branches byte-identical (golden review);
  Full flip + replay false recorded in targets.json; full corpus
  green on Rust; order-system simulates on Rust with fixed exact
  outcomes; canonical anchors byte-exact.
- **M5 exits when:** the checkpoint report (measured vs estimated,
  decision sentence) is committed in this file; the ×5 harness
  runs with per-scenario target coverage recorded; go/narrow-go/
  no-go decision recorded.
- **M6/M7/M8 exit when (each):** the target's world is
  self-contained and contract-complete; gate-emptiness green;
  Full flip recorded; the full corpus matches Rust's outcomes
  exactly; production branches byte-identical; status
  table/targets.json updated in the same milestone.
- **M9 exits when:** Python's disclosure is closed and its runner
  passes the full corpus; ×5 identity green on every scenario
  including the flagship; CI row extended; simulation.md rewrite
  complete; ledger row closed with proof; version bumped;
  retrospective appended with the drift scorecard.

## Open questions resolved at implementation (pre-registered)

1. **Scenario schema versioning** — hold `SCENARIO_VERSION = 1`
   with additive kinds (bias: yes — no cross-version scenario
   ecosystem exists to protect) vs bump to 2; decided in M1,
   recorded with the compatibility reasoning.
2. **`SimPlan` constraint topology** — whether the plan already
   carries references/uniques/cascade facts the schema-aware store
   needs or must be extended; discovered in M1 against the real
   struct, extension kept additive.
3. **Replay flag shape** — a bare `sim_replay: bool` on the sim
   surface vs folding into a small enum alongside `SimSupport`;
   decided in M1 against `TargetInfo`'s existing style (the
   registry consumers and targets.json emitter are the tiebreak).
4. **Filtered-update/read-verb exact scope** — fixed in M1 by
   corpus scan of reachable lowered verbs, not by speculative SQL
   coverage; the contract table's bold rows get their final,
   narrower-is-fine wording there.
5. **Auth seam placement per target** — under the validator
   middleware vs inside the scope-check helper; per-target
   decision at each restatement milestone, constrained by the
   byte-identical-production-branch invariant, recorded per
   target.
6. **Java runner packaging under depth** — whether the test-scoped
   `SimRunner` main-class shape (25's decision) survives the
   larger world or wants the exec-plugin arrangement adjusted;
   revisited in M8 against the one-line-stdout contract, recorded.

## Verification strategy

Standard per-milestone discipline (fmt/clippy/test, golden review,
live proofs, Docker-delegation honesty), plus this arc's specific
spine: the corpus-×-targets identity harness as the running
conformance oracle from M5 onward; gate-emptiness unit tests as
the per-target completion criterion; the two canonical anchors as
the never-moves floor at every single milestone; and the
byte-identical-production-branch golden invariant on every guard
landed.

The proof ledger by layer:

| Claim | Oracle |
| --- | --- |
| the shared world implements the contract | per-fake unit tests in ciac-sim, incl. batch rollback + fan-out |
| restatements behave identically | the corpus ×5 identical-outcome harness — every scenario, every target |
| guards don't disturb production | byte-identical `else`-branch golden review per guard; equivalence suite green |
| depth doesn't break the narrow floor | canonical anchors byte-exact at every milestone |
| gates tell the truth mid-arc | gate output derived from the live unguarded scan; emptiness tests at completion |
| the flagship claim | order-system.ciac-sim.json green ×5 with fixed exact outcomes |
| sim-atomicity matches real atomicity | fake batch-rollback scenario agrees with 26's live rollback proof |
| fake≠real drift caught where real is cheap | SQLite relational ratchet in-crate; redis/NATS rows compose-delegated |
| replay honesty | `--record/--replay` refused with SIM0010 on the four; accepted on Python |
| docs match machine truth | targets.json checked-in test; status table updated per-milestone under review |

## Milestone dependencies and parallelism

M1 strictly first (contract + schema + replay decoupling gate
everything). M2→M3 sequential inside the shared crate; M4 after
M3; M5 after M4 — the arc's spine is serial through the
checkpoint by design. After M5, M6/M7/M8 are independent of each
other and may proceed in any order or in parallel (each touches
only its own backend crate + templates + goldens), though the
plan's listed order is the deliberate default. M9 last, needing
all three restatements plus Python's closure. Python's M9 verb
work could technically start any time after M1; it is held to M9
so the ×5 harness's pending-coverage ledger stays simple —
an execution convenience, revisable at M5 if the checkpoint wants
Python earlier.

## Explicit cuts

No failure-vocabulary expansion (`error` only — parity; Delay/
Timeout/Lose/Duplicate/Disconnect stay parse-but-refuse,
disclosed). No record/replay on the four newly-Full targets (the
decoupled flag makes this a visible, honest 1/5 — the ledger row
stays open). No multi-service simulation (28UpdatePlan.md — the
per-target drivers' single-service bails are untouched here). No
crypto in FakeAuth, ever (authentication is the scope suites' job;
the fake's job is authorization logic). No arbitrary-SQL fidelity
in the relational fake (the contract is the lowered verb set, not
a database). No cross-target log/serialization-format unification
smuggled in via the worlds (26's cut, still cut). No performance
work on the fakes beyond not-being-stupid (maps and vectors; the
virtual clock makes wall time irrelevant to outcomes). No new
CLI surface beyond what the replay flag's refusal message needs.

## Risks

- **Restatement drift at 3× surface.** The defining risk, and the
  reason the corpus-identity harness is the arc's spine rather
  than an afterthought: five worlds, one answer sheet, every
  scenario, every push. The M9 drift scorecard (corpus-found bugs
  per restatement) measures whether the mitigation worked and
  feeds the retrospective honestly either way.
- **The schema-aware store underestimates relational semantics.**
  The contract is deliberately the *fake's* semantics (Python's
  scratch-overlay model), not a database's; the SQLite ratchet
  rows exist precisely to catch where that model lies about
  reality, and M5 can narrow-go a family if the gap is structural.
- **Guard proliferation churns goldens beyond meaningful review.**
  The byte-identical-production-branch invariant makes each guard
  diff mechanically checkable (the reviewer verifies the `else`
  side is unchanged, the world side is new); milestones land one
  target at a time to keep any single review bounded.
- **The auth seam tempts a production-code change.** The
  invariant forbids it; if some target's validator genuinely
  cannot branch without restructuring, that finding goes to M5/the
  restatement milestone as a recorded design decision, not a
  quiet rewrite.
- **Python's verb closure destabilizes the one Full target.**
  Smallest change in the arc, held to M9 deliberately, behind the
  full corpus as its regression net — and pyrunner's session fake
  has unit-style scenario fixtures of its own to extend first.
- **Checkpoint theater.** M5's decision space (go/narrow-go/no-go)
  is written down before M4 executes, with narrow-go's uniformity
  rule fixed in advance — the checkpoint can only be honest if
  its options were priced before the sunk cost existed. Same
  device, same reason as the factory arcs.
- **Delivery-order or serialization nondeterminism defeats the
  identity harness with noise.** Pre-empted structurally: the
  delivery loop's ordering rules are contract (Pillar 2's
  specification), outcome JSON is canonicalized (sorted keys,
  integer-valued numbers as integers), and the harness reports
  *which* scenario and *which* field diverged so a formatting slip
  is a five-minute fix, not an afternoon of diffing runner logs.
- **The two references (world.py, world.rs) drift from each
  other.** Named openly (Pillar 2); bound by the same corpus with
  no privileged member; underspecified behaviors resolved once in
  this file's reference-semantics section and applied to both.
- **Corpus example programs bloat the five-target example
  matrix.** Each new `sim-*.ciac` program multiplies CI's example
  loops ×5; the corpus table therefore reuses existing examples
  wherever a family is cleanly reachable and caps new programs at
  three — a budget, revisable only at M5 with the cost stated.

## Confidence and handoff

High on the shared-crate half: it is workspace Rust with unit
tests, inheriting into the Rust target through vendoring machinery
that has worked since v0.17 M11, and its contract is an
enumeration of an existing, running fake. Medium on the
restatement half at 3× surface — with the corpus harness as a
mitigation that is structural rather than aspirational, a
checkpoint that prices descoping before it is tempting, and the
honest fallback that a narrow-go descopes uniformly rather than
raggedly. The arc ends the "narrow" era either way: worst honest
case, four targets are *deep-but-not-total* with one uniformly
descoped family named in the ledger; expected case, the word
disappears.

Handoff: 28UpdatePlan.md (Multi-Service Simulation) inherits five
full worlds, a corpus harness that already runs everything
everywhere, a replay flag that says what it means, and per-target
drivers whose only remaining refusal is the single-service bail it
exists to remove. After it, 29UpdatePlan.md gets to put a
simulation story in front of strangers that needs no asterisks —
which was the point of doing these two arcs before the front door.
