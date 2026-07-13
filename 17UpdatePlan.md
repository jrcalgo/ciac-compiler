# CIaC v0.17 — The Simulation Version: The Infra-Free Whole-System Loop (roadmap forecast)

> Forecast document. Assumes v0.16 (relations, constraints,
> transactions) has landed — the simulator's database fake must honor
> relational semantics to be worth trusting, which is why this version
> follows it. Direction-setting; the v0.17 planning pass finalizes the
> fake-capability contract and the virtual-clock mechanism per target.
> **Confidence labels**: per-capability fakes are *structural*; the
> whole-system simulator with virtual time is a *high-conviction bet*
> that must earn its keep through the M2 acceptance gate below.

## The gap this version closes

Ciac's only correctness signal for capability-touching behavior is a
live `docker compose up`. That is the right *outer* truth and stays
so. But as the every-edit inner loop it fails two audiences at once:

1. **Coding agents increasingly run where Docker doesn't.** The
   entire v0.15 arc was, itself, the case study: every
   tracing/Keycloak milestone was developed in an environment with no
   Docker daemon, proven via standalone spike programs and disclosed
   CI delegation. Agentic environments are routinely sandboxed,
   unprivileged, and ephemeral by construction. For that audience the
   compose-backed loop isn't slow — it's *absent*. (This is a
   conditional bet, stated honestly: it matters in proportion to how
   much of ciac's usage is agent-driven. The bet is that proportion
   grows.)
2. **Even with Docker, the loop is 30–90+ seconds** per edit: image
   rebuild, container boot, health-probe backoff (`compose_up`'s
   180s `--wait` budget, `verify_live`'s 60s probe budget in
   `crates/ciac/src/commands.rs`). And whole classes of declared
   behavior are effectively untestable at any speed because they live
   on the wall clock: a `job { schedule: "0 3 * * *"; }` fires at 3am;
   a worker's `max_retries: 2` needs real failures spaced by real
   backoff; a `cache_ttl: 300` needs five real minutes. Nobody writes
   those tests today because nobody can.
3. **User-authored logic gets zero scaffolding.** The generated test
   suites prove *wiring* (auth, CRUD round-trips, broker delivery,
   trace continuity, live-IdP scopes) — never the business logic a
   human or agent just wrote into a seeded handler stub. The moment
   real logic exists, the developer is back to hand-rolling fixtures
   for capability clients ciac itself injected.

The out-of-the-box observation that turns this from "add mocks" into
a version-defining pillar: **ciac owns the whole system graph.** A
framework can fake one service's database; only a whole-system
compiler can put *every* service, the broker between them, and the
clock they all share into one deterministic process. This is
FoundationDB/Antithesis-style simulation testing offered as a
compiler artifact — a thing none of ciac's substitutes can do,
because none of them see the topology.

**v0.17 theme: full-system behavioral feedback in under a second,
with zero infrastructure, deterministic from a seed — and the real
compose stack retained as the outer truth it already is.**

## Pillar 1 — The capability fake contract

Every capability gains an in-process fake conforming to a written
contract, swapped in at the same construction seam the generated apps
already have (`AppState::new` / `get_settings()` — the exact seam the
v0.14 M6 scope tests already exploit for the `jwt` scheme).

- **Decision: sim is a generated artifact, not a runtime mode.** The
  shipped binary/app never contains fake code paths. Python: a
  generated `tests/sim/` harness wires fakes via FastAPI
  `dependency_overrides` plus fixture-injected fake engines/clients.
  Rust: a `sim` cargo feature in the generated `Cargo.toml.j2` gates
  `#[cfg(feature = "sim")]` constructors — compiled only under
  `cargo test --features sim`, never in the release artifact.
- Per-capability fake semantics (the contract, checked in as
  `docs/simulation.md` tables):
  - `db` — in-memory relational store honoring v0.16 semantics:
    typed columns, unique violations → the same 409 mapping, FK
    `restrict`/`cascade`/`set_null`, transactions with rollback. Not
    a SQL engine: it implements the *closed verb set* + generated
    store operations, which is exactly why the closed registry
    keeps paying rent. (SQLite in-memory is the fallback
    implementation detail per target where cheaper than hand-rolling
    — e.g. `sqlite::memory:` behind the same store API — as long as
    the contract tests pass either way.)
  - `cache` — in-memory map with TTL semantics driven by the virtual
    clock (Pillar 3), so `cache_ttl` is finally testable.
  - `queue` — in-process broker preserving per-subject ordering and
    at-least-once redelivery; queue-group load-balancing semantics
    for workers; **no** Kafka partition/rebalance fidelity (disclosed
    cut — the contract documents exactly which broker behaviors are
    modeled).
  - `auth` — the existing local-JWT path for `jwt`; for `oauth2` a
    fake JWKS (generated keypair, in-process endpoint) so OAuth2
    services finally get infra-free auth tests — closing the
    long-standing "OAuth2 excluded from the no-infra suite" gap noted
    since v0.14 M6.
  - `object_store`/`email`/`search`/`external_http` — recording
    fakes: store puts/gets in memory, capture sent emails for
    assertion, substring search over indexed docs, scripted HTTP
    responses per route.
  - `scheduler`/`realtime` — driven by the virtual clock / in-process
    channel delivery.
- **The fidelity ratchet (the load-bearing test)**: a
  contract-parity suite in ciac's own CI runs *the same generated
  assertions* against the fakes and against the real compose stack
  (`generated-system` job, v0.9 M5). Any divergence is a bug in the
  fake, not an accepted drift. This is the mitigation for the
  classic mock rot failure mode and it is the acceptance bar for the
  pillar.

## Pillar 2 — The whole-system simulator

`ciac verify --sim` (and bare `ciac sim` for an interactive REPL-ish
run) executes the *entire multi-service system* in one process:

- **Python target**: every service's FastAPI app is imported into one
  process; the generated typed call clients (v0.5 M5,
  `app/clients/*.py`) are pointed at in-process
  `httpx.ASGITransport` instances instead of real sockets — real
  request/response semantics, zero network. The fake broker delivers
  published messages to consuming workers as direct awaited tasks in
  deterministic order. Generated as a `sim/` package at the system
  root (sibling of `tests/system/`, which stays compose-backed).
- **Rust target**: under the `sim` feature, each service's axum
  `Router` is exercised via `tower::ServiceExt::oneshot` (the exact
  in-process pattern `scope_tests.rs.j2` already uses), call clients
  swap `reqwest` for an in-process tower client, and the fake broker
  is a `tokio::sync` channel fabric.
- **Determinism**: single-threaded executor, seeded RNG for every
  generated id (`--seed N`), stable delivery order. Acceptance test:
  two runs with the same seed produce byte-identical event
  transcripts; a failing run's seed reproduces the failure exactly.
  This is what makes sim failures *reportable* by an agent: "seed
  42, step 17" is a complete bug report.
- **Scope honesty**: the simulator proves *logic and topology* —
  handler behavior, pipeline composition, delivery, policy/auth
  outcomes, time-driven behavior. It deliberately does not prove
  driver/wire fidelity (SQL dialects, rdkafka behavior, JWKS
  fetching); that remains `verify --system`'s job, and the docs say
  so in exactly those words.

## Pillar 3 — Virtual time

The single highest-leverage detail, because it makes currently
*untestable* declarations testable:

- **Rust**: `tokio::time::pause()`/`advance()` — built into the
  runtime; the generated sim harness exposes `sim.advance(Duration)`.
  Worker retry backoff, job cron schedules, and cache TTLs all run on
  `tokio::time`, so they come along for free.
- **Python**: the generated runtime's time touchpoints
  (`croniter`-driven job loop, retry sleeps in `worker.py`, TTL
  checks) already flow through a small number of generated call
  sites; those sites are generated to consult a clock object that is
  real by default and virtual under sim (a seam ciac controls
  *because it generates the code* — no monkeypatching of the event
  loop, no `freezegun` dependency).
- What this unlocks, as generated example assertions:
  `sim.advance(hours=24)` → the 3am cleanup job ran exactly once;
  force two handler failures → third attempt succeeds after
  `max_retries` backoff, all in microseconds; `advance(seconds=301)`
  → the cached read misses. Each of these becomes a *generated* test
  for the constructs that declare them (a job gets a fires-on-
  schedule test; a `max_retries` worker gets a retry test; a
  `cache_ttl` crud gets an expiry test) — declared behavior finally
  has declared proof.

## Pillar 4 — Handler-logic test scaffolding

The user-authored-code gap, closed on top of the fakes:

- For every seeded handler stub, generation seeds (once,
  `FileRole::Seeded` — user-owned after first write, same as the stub
  itself) a matching test file: `tests/logic/test_<handler>.py` /
  `tests/logic/<handler>_test.rs`, pre-wired with the handler's bound
  capability fakes and a sample payload from the `sample_json`
  machinery (`system_tests.rs` already synthesizes type-correct
  payloads; this exposes it per-handler).
- The scaffold contains one passing smoke assertion and a commented
  skeleton for the behavioral cases — enough that an agent's next
  action is "fill in the assertion", not "figure out how to construct
  a fake S3 client".
- `ciac verify` (plain, no flags) runs these alongside the existing
  per-project suites — they need no infra by construction.

## Pillar 5 — Loop integration (CLI, MCP, dev, CI)

- `ciac verify --sim` flag; `ciac dev --sim` makes the v0.13 watch
  loop run the sim suite on save instead of restarting compose —
  sub-second red/green in the editor.
- **MCP `verify_sim` tool** — the tool the v0.15 arc's own experience
  demanded: full behavioral verification exposed to agents without
  the Docker-sandboxing concerns that rightly keep `--system`/`--live`
  off MCP. Result carries the standard envelope plus the seed and
  failure transcript on red.
- Generated CI (`--deploy ci`, v0.15 M5) gains a `sim` job *before*
  the compose-smoke job — fast failure first, containers only for
  green candidates.
- `AGENTS.md` (generated, v0.13 M5) rewrites its verify guidance:
  sim is the inner loop, `--system` is the merge bar.

## Secondary items

- `--json` envelope: sim results reuse the envelope with a
  `sim: { seed, steps, transcript_digest }` block (JSON_VERSION bump).
- `docs/simulation.md`: the fake contract tables, the fidelity
  ratchet, determinism guarantees, and an honest "what sim does not
  prove" section.
- The OAuth2 fake-JWKS work retires the `jwt`-only gate on
  `scope_tests.rs.j2` / `test_smoke.py.j2`'s scope blocks.

## Milestones

1. **M1 — fake contract + Python fakes**: contract doc, db/cache/
   queue/auth fakes, dependency-override wiring, contract-parity
   suite green against compose in CI. Acceptance: `dev-identity` and
   `commerce` examples' behavioral suites pass infra-free.
2. **M2 — whole-system harness, Python**: ASGI-transport call
   clients, fake broker, multi-service `sim/` package; the
   `traced-checkout` example's call→publish→worker chain proven
   in-process. **Gate**: if in-process fidelity for the call-client
   path can't be made honest here, the whole-system pillar is cut
   back to per-service sim and the plan is revised — this is the
   high-conviction bet's checkpoint.
3. **M3 — virtual time**: tokio pause/advance (Rust), generated
   clock seam (Python); the generated job/retry/TTL tests; the
   determinism (same-seed) acceptance test.
4. **M4 — Rust sim parity**: `sim` feature, oneshot harness, tower
   call clients, fake broker; contract-parity suite extended to the
   Rust corpus.
5. **M5 — handler-logic scaffolding**: seeded test files both
   targets, per-handler fake wiring, sample payloads; regen
   discipline proven (scaffolds are seeded, never clobbered).
6. **M6 — loop integration**: `verify --sim`, `dev --sim`, MCP
   `verify_sim`, CI sim job, AGENTS.md rewrite, envelope extension.
7. **M7 — docs, hardening, 0.17.0**: `docs/simulation.md`, fidelity-
   ratchet CI wiring as a permanent job, full verification, version
   bump, arc notes.

## Risks

- **Fake/real divergence (mock rot)** — the defining risk.
  Mitigation is structural, not aspirational: the contract-parity
  suite runs both ways in ciac's own CI forever; a fake without a
  parity test doesn't merge.
- **Whole-system fidelity too weak to trust** (e.g. subtle
  serialization differences on the in-process call path). Mitigation:
  the M2 gate above — cut to per-service sim rather than ship a liar.
- **Python virtual-clock seam fragility.** Mitigation: the seam is
  generated code consulting a generated clock object — ciac controls
  every call site; no event-loop patching; the seam is golden-tested.
- **Two test stacks confuse users.** Mitigation: naming and docs are
  blunt — *sim proves your logic fast; system proves the wiring for
  real* — and `AGENTS.md` encodes when to run which.
- **Maintenance surface**: every new capability now needs a fake +
  parity test. Accepted deliberately — at ~17 capabilities the
  retrofit is affordable; at 30 it wouldn't have been. This cost is
  the argument for doing it *now*.

## Cut lines

- Fault injection (random latency/crash/partition schedules) — the
  full Antithesis dream. The deterministic substrate ships now; the
  chaos schedule on top is future work once real usage trusts the
  substrate.
- Cross-target sim (running a rust-target system under the Python
  harness or vice versa): each target gets its own native sim.
- Any sim of `--deploy` artifacts (k8s/terraform behavior).
- Broker fidelity beyond ordering + at-least-once (partitions,
  rebalances, exactly-once).

## After v0.17

Systems are now easy to express (v0.16) and instant to verify. The
next place time leaks is *change*: week-two edits to a system that
already exists, where an agent gets the least help today. v0.18 (the
Evolution version) takes the confirmed semantic-diff pillar and
builds the safe-change story on machinery this version leaves green.
