# CIaC v0.18 — The Evolution Version: Semantic Diff, Safe Change, Mechanical Refactoring (roadmap forecast)

> Forecast document. Assumes v0.16 (domain) and v0.17 (simulation)
> have landed. Direction-setting; the v0.18 planning pass finalizes
> the system-snapshot manifest format and the breaking-change
> classification table. **Confidence label**: the semantic diff is a
> *confirmed pillar* (agreed in the v0.15→v0.16 planning
> conversation); the extensions around it (gates, backfills, rename)
> are low-risk generalizations of machinery that already exists —
> `evolution.rs` (v0.8 M5), the migration differ (v0.7 M5), the
> regeneration manifest (v0.6), and the structured-fix edit engine
> (v0.15 M7).

## The gap this version closes

Ciac is excellent on day one and nearly silent on day fourteen.
Everything from `ciac new` through `verify --sim` serves *creating* a
system; almost nothing serves *changing* one that already exists —
and mature systems spend essentially all of their life being changed.
Concretely:

1. **`ciac diff` is textual.** It answers "which generated files
   change" (status per path, unified patches under `--patch`) — never
   "what does this change *mean*". An agent asked "does this edit
   break any consumer?" must diff strings and infer. Yet the compiler
   holds two `NormalizedIr`s and could simply *say*: route removed,
   field retyped, scope tightened, stream payload changed. The v0.15
   house style — the compiler emits typed facts (`openapi.json`,
   `fixes`, the protocol schema) instead of prose — has not yet been
   applied to *change itself*.
2. **Breaking changes are discovered by consumers, not by the
   compiler.** `evolution.rs` already proves the concept in
   miniature: since v0.8 M5, removing or retyping a record field that
   a consumer service reads is CIAC0051 *within one program*. But a
   deployed system's real consumers — the TS client shipped to a
   frontend team, the OpenAPI doc other teams generated against, the
   previous version of the system itself during a rolling deploy —
   have no compatibility check at all.
3. **Schema evolution stops at shape.** The migration differ handles
   column add/drop/retype, and v0.16 adds FK/index ops plus the
   `--allow-destructive` gate — but "add a required column to a table
   with existing rows" still has no story beyond "declare a
   default". Real evolution needs backfills: compute the new column
   from existing data, then tighten.
4. **Renames don't exist.** LSP rename has been "explicitly
   deferred" since v0.12. Renaming a record or field today means:
   hand-edit the `.ciac`, regenerate, watch the migration differ emit
   a *drop + add* (data loss, caught only by the destructive gate),
   and grep seeded files by hand. The single most common refactor in
   any codebase is the least supported operation in this one.

**v0.18 theme: change becomes a typed, classified, gated compiler
artifact — the week-two loop gets the same treatment the week-one
loop already has.**

## Pillar 1 — The system snapshot and semantic diff

### The snapshot

The regeneration manifest (v0.6, extended v0.8 M5 with
`Manifest.records`) grows into a full, schema-versioned
`SystemSnapshot` embedded in the generated output: every route
(method, path, request/response record shapes, scope), every record
(fields, types, refs, constraints), every stream (subject, payload),
every capability instance (kind, provider, engine), every policy-
relevant attribute. It is derived by one serializer from the same
`SystemModel`/`NormalizedIr` everything else renders from —
`openapi.json` discipline, applied to the whole system — and carries
its own `snapshot_version` (append-only evolution, staleness-tested
exactly like `docs/protocol-schema.json`).

### The diff

`ciac diff --semantic` compares the current compile against a
baseline and emits a **typed changelist**, not text:

```json
{ "changes": [
  {"kind": "route_removed",   "route": "DELETE /orders/{id}", "class": "breaking"},
  {"kind": "field_retyped",   "record": "Order", "field": "total",
   "from": "Int", "to": "Float", "class": "breaking",
   "consumers": ["tests/system", "clients/ts", "Billing.Charge"]},
  {"kind": "route_added",     "route": "GET /orders/{id}/items", "class": "additive"},
  {"kind": "scope_tightened", "route": "GET /orders", "from": null,
   "to": "orders:read", "class": "breaking"},
  {"kind": "index_added",     "record": "Order", "fields": ["customer"],
   "class": "internal"}
]}
```

- **Baseline resolution**, in order: `--against <path|git-ref>`
  (compile that source and compare IR-to-IR — the most precise
  mode), else the `--out` directory's stored snapshot (compare
  IR-to-snapshot — the everyday mode, no second compile needed).
- **Classification** is a closed, documented table: *breaking*
  (consumer-visible contract narrowed: route/field removed, type
  changed, scope added/tightened, stream payload narrowed, unique
  constraint added to existing field), *additive* (contract widened:
  new optional field, new route, scope loosened), *internal*
  (indexes, handler bodies, capability provider swaps that keep the
  contract, docs). Every classification rule is unit-tested with a
  fixture pair, and the table lives in `docs/evolution.md` so the
  vocabulary is stable for tooling.
- **Consumer attribution** reuses and extends `evolution.rs`'s
  consumer lookup: a breaking record change names the in-program
  consumers (call targets, workers, channels) *and* the generated
  artifacts that embody the old contract (TS client, OpenAPI).
- Output: human table on stderr, `--json` array in the envelope
  (JSON_VERSION bump), and a **new MCP tool `diff_semantic`** — the
  agent's "what would this change do" primitive, completing the
  check → fix → verify_sim → diff_semantic loop vocabulary.

## Pillar 2 — Compatibility gates

Classification without enforcement is a report nobody reads:

- `ciac diff --semantic --deny breaking` exits non-zero when any
  change classifies at or above the named class — the local gate.
- The generated CI workflow (`ci.rs`, v0.15 M5) gains a `compat` job
  when a snapshot baseline exists in the repo: regenerate, semantic-
  diff against the checked-in snapshot, fail on `breaking` unless the
  commit message carries an explicit `ciac-breaking:` trailer — the
  same "you may, but you must say so out loud" discipline as
  `--allow-destructive` and the sidecar system.
- Intentional breaks are then *recorded*: accepting one updates the
  snapshot, and the changelist entry (with the trailer's
  justification) is appended to a generated `CHANGELOG.ciac.md` —
  the system's contract history becomes an artifact, for humans and
  for agents doing archaeology.

## Pillar 3 — Backfills and the destructive-change ladder

v0.16 left a deliberate seam: new required columns demand a
`default`. v0.18 completes the ladder for the changes a default can't
cover:

- **Expand/backfill/contract as a first-class shape.** When the
  differ detects a change that needs data motion (required column
  without default, retype with conversion, `set_null`→`restrict`
  tightening), it refuses the one-shot migration and instead emits a
  three-step plan: (1) additive migration (nullable column), (2) a
  **seeded backfill script** (`migrations/backfill_<n>.py` /
  `.rs` — user-owned after first write, like every seeded file, with
  the row-iteration skeleton and the obvious conversion pre-filled),
  (3) the tightening migration, gated on the backfill having run
  (tracked in the existing migration-ledger table).
- The sim (v0.17) can execute the whole ladder against the in-memory
  store with generated fixture rows — expand/backfill/contract gets a
  fast rehearsal before it touches Postgres.
- `--allow-destructive` remains the final gate for true drops; the
  ladder exists so that reaching for it becomes rare.

## Pillar 4 — Mechanical rename

- `ciac rename <Record> <NewName>` and `ciac rename
  <Record>.<field> <new_field>` — a CLI verb (and LSP
  `textDocument/rename`, paying the v0.12 deferral) that:
  1. rewrites every reference in the `.ciac` source set (the
     compiler owns every span — this is the `fixes` edit engine from
     v0.15 M7, `Fix::apply` and friends, driven by a whole-program
     reference walk instead of a single diagnostic);
  2. regenerates owned files through the normal build path;
  3. **reports** — never rewrites — references in seeded files
     (they're user-owned by contract): a file:line list on stderr /
     in the envelope, so the human or agent finishes the job with
     full information;
  4. records a rename identity in the snapshot so the migration
     differ emits `ALTER TABLE ... RENAME` instead of drop+add, and
     the semantic diff classifies it as `renamed` (breaking for
     external consumers, but distinctly labeled) rather than
     remove+add.
- The identity record is the load-bearing design decision: **the
  differ never guesses renames** from shape similarity — a rename is
  a rename only because the user said so through the verb. No
  heuristics, no silent misclassification.
- LSP rename wires the same engine into the editor;
  MCP gains `rename` for agents.

## Secondary items

- `ciac diff --semantic --format markdown` for PR-comment-ready
  output (pairs with the generated CI compat job).
- `openapi.json` diffing falls out for free: the semantic changelist
  is strictly richer, but a `--surface http` filter emits only the
  HTTP-contract subset for teams that think in OpenAPI terms.
- Structured fixes: `field_retyped`-class CIAC0051 diagnostics gain
  a "revert to baseline type" fix where mechanical.
- `docs/evolution.md`: the classification table, the snapshot format,
  the expand/backfill/contract ladder, the rename discipline.

## Milestones

1. **M1 — SystemSnapshot**: serializer from `SystemModel` +
   `NormalizedIr`, embedded in generated output, schema-versioned,
   staleness-tested; manifest migration for existing outputs (old
   manifests upgrade in place on next build, disclosed).
2. **M2 — semantic diff core**: IR-vs-snapshot and IR-vs-IR
   comparison, the typed changelist, consumer attribution, human +
   `--json` output; fixture-pair unit suite covering every changelist
   kind.
3. **M3 — classification + gates**: the closed class table,
   `--deny`, the generated CI `compat` job with the
   `ciac-breaking:` trailer discipline, `CHANGELOG.ciac.md` emission;
   MCP `diff_semantic`.
4. **M4 — rename**: reference walk + edit engine, seeded-file
   reporting, snapshot rename identity, differ RENAME emission, LSP
   `textDocument/rename`, MCP `rename`; live proof: rename a field in
   `commerce.ciac`, run the emitted migration against real Postgres,
   assert data survived.
5. **M5 — backfill ladder**: differ detection of data-motion
   changes, three-step plan emission, seeded backfill scripts,
   ledger gating, sim rehearsal; live proof: expand/backfill/contract
   a populated table end-to-end under `verify --system`.
6. **M6 — docs, hardening, 0.18.0**: `docs/evolution.md`, README
   pitch update, full workspace verification, version bump, arc
   notes. Per-milestone discipline throughout (fmt/clippy/tests/
   insta, live proof or disclosed delegation, commit + push).

## Risks

- **Snapshot format churn** — it becomes a compatibility surface of
  its own. Mitigation: schema-versioned from day one, append-only
  evolution, the protocol-schema staleness-test pattern applied
  verbatim.
- **Classification disputes** ("is scope-loosening really
  additive?"). Mitigation: closed documented table, every rule with a
  fixture; disagreements become table PRs, not silent behavior
  drift.
- **Rename touching seeded code tempts auto-rewrite.** Mitigation:
  the ownership contract wins — report, never rewrite; the reported
  list is precise enough (file:line:col) that an agent finishes it in
  one pass anyway.
- **Baseline ambiguity** (which snapshot is "the" baseline in a
  multi-branch repo). Mitigation: the tool takes what it's given
  (`--against`/out-dir) and the CI job pins the convention (the
  checked-in snapshot on the target branch); ciac doesn't invent a
  branching model.
- **Backfill scripts are user-owned and can be wrong.** Mitigation:
  sim rehearsal is generated alongside them, and the ledger gate
  means the tightening step can't run before the backfill did —
  wrong is recoverable, skipped is impossible.

## Cut lines

- Rename detection by similarity heuristics: never.
- Cross-*system* compatibility (two independent ciac programs
  consuming each other's APIs): the snapshot is per-system; federated
  contracts are future work.
- Automatic data conversion in retype migrations beyond the seeded
  script skeleton (no "we guessed your `Int`→`Float` semantics").
- API versioning surface in the language (`/v1`, `/v2` route
  namespaces): the diff tells you *when* you broke something;
  choosing a versioning scheme remains the team's call. Revisit with
  usage evidence.

## After v0.18

Express (v0.16), verify instantly (v0.17), change safely (v0.18).
What remains before "a team bets production on this" is the class of
bugs that only appear under partial failure and multi-tenancy — the
dual-write ciac still generates when a pipeline writes then
publishes, and the per-row authorization every real SaaS needs.
That's v0.19, and it deliberately builds on this version: the outbox
rides v0.16's transactions, and policy changes are exactly the kind
of thing the semantic diff must classify as breaking.
