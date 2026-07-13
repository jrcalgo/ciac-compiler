# CIaC v0.21 — The Reach Version: The Brownfield Bridge and the Deliberate Breadth Token (roadmap forecast)

> Forecast document. Assumes v0.16–v0.20 have landed. This is the one
> version in the arc that spends a breadth token, and it does so
> under the discipline the project imposed on itself in v0.12 and
> v0.15: deferred audience decisions get **re-asked against real
> usage, not pre-committed**. Accordingly, this plan is written
> differently from the others — Pillar 1 is committed (it deepens an
> existing surface), Pillar 2 is committed-if-gated, and Pillar 3 is
> a decision framework rather than a work plan. **Confidence labels**:
> Pillar 1 *high-conviction* (it converts an existing untyped surface
> into a typed one); Pillars 2–3 *hypotheses with explicit decision
> criteria*.

## The gap this version closes

Five consecutive depth versions have optimized time-to-completion
for someone already inside ciac. Two audiences remain outside, and
for them completion time is dominated by *getting in*:

1. **Brownfield teams and agents.** Ciac is greenfield-only in both
   directions. Inbound: a team with forty existing services cannot
   adopt incrementally — there is no path from "here is our OpenAPI
   spec" to "here is a `.ciac` skeleton". Outbound: integrating a
   generated system with *anything external* runs through
   `external_http`, which since v0.4 is an untyped `base_url` and a
   raw client — the single surface in the language where the
   compiler's type discipline simply stops. Every Stripe/GitHub/
   internal-legacy-API call is stringly-typed, unvalidated, and
   hand-tested, inside a system where everything else is checked.
   For agents this is doubly costly: the highest-error-rate code an
   agent writes is precisely third-party API glue, and it's the one
   place ciac offers no help.
2. **End users of the systems ciac generates.** The stated goal is
   end-to-end systems *used* by users — and a generated backend has
   no surface a non-developer can touch. Every crud resource, policy,
   and auth flow exists; rendering them usable requires a frontend
   team. The Django-admin lesson is twenty years old: a generated
   admin surface over declared resources is often the single
   highest-leverage artifact a backend framework ships, because it
   converts "API exists" into "humans can operate the product" on
   day one.
3. **The TypeScript-backend question, still open.** Pre-slotted as
   the v0.16 headline by the v0.15 plan, deliberately re-deferred in
   the v0.16 re-planning as breadth-vs-depth, de-risked meanwhile by
   the v0.15 TS client's proven record→TS mapping. It cannot stay
   open forever: an unmade decision is itself a cost (contributors
   and users plan around it). v0.21 closes it — one way or the other
   — with named criteria.

**v0.21 theme: lower the walls — typed integration with the world
that already exists, a usable surface for the people the system is
for, and an honest verdict on the third backend.**

## Pillar 1 — The brownfield bridge (committed)

### Typed `external_http` from OpenAPI specs

```ciac
use {
    external_http stripe {
        spec: "vendor/stripe-subset.json";
        operations: [CreateCharge, GetCharge, CreateRefund];
    }
}
```

- **Spec loading is compile-time, offline, deterministic**: `spec:`
  is a local file (URL fetch goes through the v0.12 registry-cache
  discipline — pinned, hashed, cached; never a network dependency of
  a clean build). The file's hash lands in the manifest/snapshot, so
  a vendor bumping their spec is a *visible regeneration diff and a
  v0.18 semantic-diff entry*, not silent drift.
- **`operations:` allowlist is mandatory.** Real vendor specs are
  thousands of operations; generating all of them produces an
  unauditable client and a determinism-hostile surface. The
  allowlist keeps generated code reviewable and makes "which parts of
  Stripe do we depend on" a declared, diffable fact — which is the
  actual operational question.
- **Mapping**: the operation's request/response schemas map onto the
  existing `FieldTypeKind` vocabulary (the exact mapping already
  proven twice — v0.10 M1 for external backends, v0.15 M1 in
  reverse for OpenAPI emission). Unsupported constructs
  (`oneOf`-heavy polymorphism, dynamic keys) degrade *explicitly*:
  the field lands as `Json` with a warning naming the schema path —
  never a silent lie, never a hard wall on the whole spec.
- **Codegen**: per-operation typed methods on the existing generated
  client instance (`app/clients/` / `src/clients/` — the same shape
  v0.5 M5 call-clients established): typed request struct in,
  validated response envelope out, non-2xx → typed error. Handler
  verbs gain `stripe.CreateCharge(payload)` through the closed-verb
  machinery (v0.14), so typed handlers reach vendor APIs without
  leaving the DSL.
- **Sim (v0.17) integration is where this pillar compounds**: the
  recording/scripted `external_http` fake becomes *spec-aware* —
  scripted responses are validated against the vendor's own response
  schema, so a test can't stub Stripe with a shape Stripe would never
  return. Third-party integration tests, infra-free, honest.

### `ciac import openapi.json` (inbound)

- Generates a **seeded skeleton**: `record` declarations from
  component schemas, `api` declarations (method/path/request) from
  paths, `TODO` pipelines, and a report of everything that didn't
  map (auth schemes → suggested `use` entries; unmappable schemas →
  listed with reasons). One-way, lossy, and labeled as scaffolding —
  the tool's job is to convert "start from nothing" into "start from
  80%", not to round-trip.
- Mirrors `ciac new`'s ethos (v0.12): the output compiles under
  `ciac check` or the importer has a bug — that's the acceptance
  test, run against a corpus of real public specs.

## Pillar 2 — The generated admin surface (committed behind a gate)

- `ciac build --client admin` emits `clients/admin/` — a static,
  dependency-free admin SPA over the declared surface, in the exact
  discipline the TS client (v0.15 M2) established: generated from
  the IR, deterministic, golden-covered, no external generator, no
  npm-install-to-build (the TS client's `tsc`-only bar holds; the
  admin builds on that client rather than duplicating fetch logic).
- **Scope is a hard wall, and the wall is the feature**: list /
  detail / create / edit / delete per `crud` resource — columns from
  `RecordCtx`, form controls from field types and v0.16 validation
  attributes, FK fields as pickers over the target resource's list
  endpoint, enum fields as selects. Auth: bearer-token paste, plus
  the `users Keycloak` password flow when declared (dev logins with
  `dev-admin`/`dev-user` work out of the box — the v0.15 M6
  machinery becomes visible to non-developers). Ownership policies
  (v0.19) are enforced server-side already; the admin *displays*
  what the token can see, by construction.
- **Not** a UI framework: no custom pages, no theming beyond CSS
  variables, no dashboard widgets, no plugin points. The moment a
  team outgrows it, they have the TS client and the OpenAPI doc —
  the admin is the on-ramp, not the destination.
- **The gate**: this pillar ships **only if** the v0.20 hypothesis
  gate's usage review shows real deployments with human operators
  (the audience an admin serves). A ciac used purely as
  agent-to-agent infrastructure doesn't need it, and the budget
  returns to Pillar 1 depth (more spec-mapping coverage) instead.
  The gate criteria are written down *now* so the future decision is
  a lookup, not a debate.

## Pillar 3 — The TypeScript backend: a decision, not a milestone

This plan deliberately does **not** schedule the TS backend. It
schedules the *decision*, with the criteria named in advance:

- **Ship it** (as v0.22's headline) if the usage evidence shows:
  meaningful demand from TS-native teams blocked on target language
  (not merely curious); the external-backend protocol (v0.10) proving
  insufficient for the community to build it *outside* the core
  (the Go worked-example path exists precisely to test this); and
  maintenance headroom demonstrated by two consecutive versions of
  both bundled backends holding parity without heroics.
- **Decline it** (and say so publicly in the docs) if the TS
  audience's actual blocker turns out to be consumption (solved:
  OpenAPI + TS client + admin) rather than authorship, or if
  external-backend authorship is viable — in which case the
  investment goes to first-classing the external-backend developer
  experience (protocol conveniences, a conformance suite, template
  scaffolding for backend authors) instead: breadth delegated to the
  ecosystem rather than absorbed by the core.
- Either way, the decision document ships in this version — the open
  question closes.

## Secondary items

- `docs/integration.md`: the spec-loading discipline, the allowlist
  rationale, degradation rules, the import workflow, admin scope.
- Semantic diff (v0.18): vendor-spec hash changes and admin-visible
  surface changes enter the classification table.
- `vocab.rs`/`describe`/LSP: `spec:`/`operations:` attributes,
  hover for imported operation names.
- MCP: `import` tool (spec in, skeleton + unmapped-report out) —
  the agent onboarding primitive.

## Milestones

1. **M1 — spec loader + mapping core**: OpenAPI 3.0/3.1 subset
   parser, `FieldTypeKind` mapping with explicit degradation,
   allowlist enforcement, manifest hashing; corpus tests over real
   public specs (Stripe/GitHub/petstore subsets checked in as
   fixtures).
2. **M2 — typed clients, Python**: generated operation methods,
   error typing, handler-verb wiring; live proof against a scripted
   local server; sim fake made spec-aware.
3. **M3 — typed clients, Rust**: parity through the same model;
   the per-backend golden matrix as always.
4. **M4 — `ciac import`**: skeleton generation, unmapped-report,
   the compiles-under-check acceptance corpus; MCP `import`.
5. **M5 — admin surface (gated)**: list/detail/forms over crud
   resources, FK pickers, validation-aware controls, token +
   dev-Keycloak auth; golden-covered; live proof driving the
   `commerce` example end-to-end through a browser (CI-delegated
   where the environment can't, per standing disclosure).
6. **M6 — the TS-backend decision + docs + 0.21.0**: the decision
   document against the named criteria, `docs/integration.md`,
   README reach rewrite, full verification, version bump, and the
   v0.16→v0.21 arc retrospective (what the depth-first bet paid,
   what it cost, what usage said).

## Risks

- **OpenAPI in the wild is a swamp** — the defining risk of Pillar
  1. Mitigation is triple: subset + allowlist keeps the input
  bounded; explicit `Json` degradation keeps unmappable corners from
  blocking whole specs; the real-spec fixture corpus keeps the claim
  "works on actual vendor specs" continuously tested rather than
  asserted.
- **Admin scope creep** — every generated admin in history grew a
  widget system. Mitigation: the hard wall is written into the
  pillar, the cut lines, and the acceptance criteria; feature
  requests route to "use the TS client".
- **Spec-file licensing/redistribution** (checked-in vendor spec
  fixtures). Mitigation: fixtures are minimal hand-written subsets in
  the vendor's shape, not vendored originals.
- **The import tool over-promises.** Mitigation: "seeded skeleton,
  one-way, lossy" is in the command's own help text; the
  unmapped-report makes the loss explicit rather than discovered.
- **Deciding the TS backend wrong.** Mitigation: the criteria are
  falsifiable and written before the evidence is in; either outcome
  ships as a documented decision, reversible by a future plan with
  new evidence.

## Cut lines

- OpenAPI *round-tripping* (import → edit → re-export fidelity):
  the import is scaffolding; ciac's own emitted OpenAPI is the
  contract going forward.
- Spec formats beyond OpenAPI 3.x (gRPC/protobuf, GraphQL, SOAP):
  each is its own bridge; none ships here.
- Admin customization surface of any kind (see Pillar 2).
- Multi-cloud Terraform, Helm, and the rest of the walked-back
  deployment-maturity list: still walked back; still waiting for
  someone to actually be blocked.
- Starting the TS backend implementation inside this version, even
  if the decision is "ship it" — it gets its own version and plan.

## After v0.21

Six versions: the domain became expressible (v0.16), verification
became instant and infrastructure-free (v0.17), change became a
typed, gated artifact (v0.18), partial-failure and multi-tenancy
bugs became inexpressible (v0.19), every runtime symptom became
traceable to a source line (v0.20), and the walls to the existing
world came down (v0.21). The arc's through-line was stated at its
start and held throughout: ciac's moat is owning the whole graph, so
every version converted a class of runtime pain into a compile-time
or sub-second-feedback fact. What comes after v0.21 should be
decided the way this arc was: from the usage evidence the arc
produced, with depth as the default and every breadth token spent
out loud.
