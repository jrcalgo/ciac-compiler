# CIaC v0.14 — Expressiveness: Logic in the Language (roadmap forecast)

> Forecast document. Assumes v0.13 (dev loop, target parity, agent
> front door) has landed. Direction-setting; the v0.14 planning pass
> finalizes the query-predicate grammar and the blueprint-hygiene
> rules for pipeline-bearing bodies. This is the version with real
> language-design risk, which is why it shares its release with
> nothing else.

## The gap this version closes

A real backend is roughly 20% topology and 80% behavior. CIaC models
the 20% completely — and since v0.7 it models a *sliver* of the 80%:
typed handler bodies with `let`/`if`/`match`, `db.*` verbs against
declared tables, `publish`, `fail`. Everything else — a filtered
query, a cache lookup, a call to a third-party HTTP API, sending an
email, writing to the object store — must be written by hand in a
seeded extern stub, in the target language, per target.

Every line in a stub is a line the compiler cannot verify, `ciac
verify` cannot regenerate, `--target` cannot port, and an agent
cannot get compile-time feedback on (its only signal is a runtime
failure inside a container). **The extern-stub surface is the single
largest remaining tax on time-to-completion**, for humans and agents
alike.

The second half of the same gap is reuse: blueprint bodies (v0.8) may
contain only `use`/`crud`/`stream`/`handler`. The std-library
patterns that would eliminate whole classes of hand-writing — a
webhook receiver, an outbox publisher, a rate-limited API — all need
`api`/`worker`/`pipeline` inside a blueprint, which has now been
explicitly deferred three times (v0.8 M3, v0.12 planning, and the
blueprints.md roadmap note). v0.14 is the version that finally needs
it, so v0.14 pays for it.

**v0.14 theme: shrink the extern surface. Not by becoming a
general-purpose language — by extending the closed, analyzable verb
vocabulary until the common 80% of behavior type-checks in `.ciac`.**

## Design stance (the guardrail against language creep)

The verb set stays **closed and effect-shaped**: every new construct
is a typed operation against a declared capability instance, checked
against the graph (using a verb without its capability is an error
with a code, exactly like `Queue` without `queue` is `CIAC0005`
today). No user-defined functions, no loops, no recursion, no
arbitrary expressions over collections in v0.14 — a handler body
remains a straight-line, analyzable effect script with branching.
When a body outgrows that, `extern` remains the honest escape hatch;
the goal is to need it for the exotic 20%, not the boring 80%.

## Pillar 1 — Query predicates and the full `db` verb set

```ciac
extern handler ListRecent(f: Filter) -> VideoPage;
handler ListRecent(f: Filter) -> VideoPage {
    let vids = db.query Videos where status == Ready
               order_by created_at desc limit f.page_size;
    return VideoPage { items: vids, total: db.count Videos };
}
```

- `db.query <Table> [where <predicate>] [order_by <field> [asc|desc]]
  [limit <expr>] [offset <expr>]` — predicates are conjunctions of
  `field <op> expr` comparisons (`==`, `!=`, `<`, `<=`, `>`, `>=`,
  and `contains` for `String`), where `field` must exist on the
  table's record and the expression's type must match the field's
  (new error codes: unknown field, type mismatch in predicate,
  ordering by a non-field). Disjunction (`or`) is a planning-pass
  decision; joins are explicitly out of scope for v0.14.
- `db.count <Table> [where ...]`, `db.delete_where <Table> where ...`
  (bulk), plus a list-typed value (`[Video]`) so query results are
  first-class in bodies and return types — the minimal collection
  type, no map/filter over it in v0.14.
- Lowering: parameterized SQL on both backends (SQLAlchemy Core /
  sqlx query builders) — predicates compile to bound parameters,
  never string interpolation, stated and tested explicitly
  (injection resistance is a generated-code property the behavioral
  tests assert).

## Pillar 2 — The capability verb set

Each verb requires its capability on the handler's service, checked
at sema with a dedicated error code; each is typed end-to-end:

- **cache**: `cache.get <expr> as <Record>?` (optional-typed),
  `cache.set <expr> = <expr> [ttl <n>]`, `cache.del <expr>` — keys
  are `String`-typed expressions.
- **http** (against declared `external_http` instances):
  `http.call billing POST "/charge" with <expr> as <Record>` —
  request body serialized from a record expression, response
  validated into the named record (or `Json` for untyped endpoints).
  Non-2xx behaves like `fail` with a builtin error carrying status.
- **email**: `email.send { to: <expr>; subject: <expr>; body: <expr>; }`.
- **object_store**: `store.put <key> = <expr>`, `store.get <key> as
  Json?` — binary payloads stay out of scope (a `Bytes` field type is
  a v0.15+ question).
- **search**: `search.index <Table-record expr>`,
  `search.query <expr> as [<Record>]`.
- Optional-typed results introduce a narrow `<expr>?` optional with
  `if let`-style unwrapping (`if cache.get k as Video is v { ... }`)
  — exact surface finalized in planning; the type checker treats
  unhandled optionals as errors, not nulls.
- Every verb lowers on **both** backends in the same milestone that
  introduces it — the v0.11 lesson (split support) is a cost worth
  paying only for whole providers, never for core language semantics.

## Pillar 3 — Blueprint bodies grow up (api/worker/pipeline/record/table)

- `BlueprintItem` extends to `api`, `worker`, `pipeline`, `record`,
  `table` (and `job`/`channel` if hygiene falls out cleanly — a
  planning decision, not a promise). Hygiene rules: every name a
  blueprint body declares is suffixed per expansion (existing
  mechanism); a body-local `pipeline X:` binds to the body-local
  `api X`/`worker X` *post-suffix*, so the pair stays attached; steps
  may reference body-local handlers, the type parameter's record, and
  `params` values. Referencing enclosing-scope names from a body
  stays an error (blueprints are closed terms).
- **The std library this unlocks**, shipped in-version as both proof
  and product:
  - `std/webhook.ciac` — `expand Webhook<Payload> { path: "/hooks/x"; }`:
    an api + validation + publish-to-stream receiver;
  - `std/outbox.ciac` — transactional-outbox shape: table + api-side
    `db.insert` + a worker draining the table to a stream (the
    pattern needs Pillar-1 verbs, which is why blueprints and verbs
    share a version);
  - `std/rate-limited-api.ciac` — cache-backed fixed-window limiter
    in front of a declared handler (needs Pillar-2 cache verbs).
- Each std blueprint gets the same byte-equivalence-or-behavioral
  test discipline `std/crud.ciac` established (v0.8 M3).

## Pillar 4 — Route authorization from the model

- The `scope: "videos:write"` attribute apis already parse becomes
  *enforced*: JWT/OAuth2 middleware checks the token's
  `scope`/`scp` claim against the route's declared scope on both
  backends; `crud` grows per-verb scope attrs (`read_scope`/
  `write_scope`). A `scope:` attribute on a service with no `auth`
  capability is a new sema error (the "declared but silently
  unenforced" failure mode is the one this pillar exists to kill).
- Generated tests: per-scoped-route assertions that a token missing
  the scope gets 403 and a token carrying it passes — in the
  generated project's own test suite (both backends), not just the
  system suite.
- Explicitly out of scope: roles/RBAC models, multi-tenancy, row-level
  policies — scopes are the v0.14 cut line because they're already in
  the surface syntax and map 1:1 onto both providers' token claims.

## Secondary items

- LSP + `ciac describe` pick up the new verbs/attrs automatically
  (shared static tables from v0.13 M5 — extending the table is the
  implementation).
- `docs/expressions.md` rewritten around the full verb set with a
  per-verb capability/typing/lowering table; `docs/blueprints.md`
  gains the new item kinds and the three new std entries.
- New flagship example threading it together (e.g.
  `order-system.ciac`: outbox blueprint + query predicates + cache +
  scoped routes), golden + CI `generated-system` row.

## Milestones

1. **M1 — front end**: grammar/AST/parser for predicates, list type,
   optionals, all Pillar-1/2 verb forms; sema + type checker + the
   new error-code block; negative fixtures per error code. (No
   codegen yet — the front end lands whole so both lowerings target a
   frozen surface.)
2. **M2 — db verbs, both backends**: query/count/delete_where
   lowering (SQLAlchemy Core / sqlx, bound parameters), behavioral
   tests incl. injection-shaped inputs, live proof on Postgres +
   MySQL + SQLite.
3. **M3 — cache + http verbs, both backends**: live proof: cache
   against apt Redis; http against a local test server (the JWKS/
   registry technique).
4. **M4 — email/store/search verbs, both backends**: live proof where
   local infra allows; CI-delegated where it doesn't (disclosed).
5. **M5 — blueprint body extension + std library**: BlueprintItem
   growth, hygiene rules + tests, `std/webhook` + `std/outbox` +
   `std/rate-limited-api` with equivalence/behavioral proofs.
6. **M6 — authorization scopes**: enforcement both backends +
   generated 403/200 tests, `crud` per-verb scopes, sema rule, docs.
7. **M7 — flagship example, docs rewrite, version 0.14.0**, full
   verification, whole-version analysis.

## Risks

- **Language creep** is the existential one. Mitigation is the design
  stance section above, enforced structurally: the verb registry is a
  closed table (like the capability registry), every addition needs a
  capability, a type rule, two lowerings, and tests — a cost that
  keeps the vocabulary honest. Anything smelling like general-purpose
  computation is deferred by default.
- **Rust lowering complexity** (sqlx's compile-time checking vs
  dynamically-shaped queries). Mitigation: generate runtime-checked
  query building (`sqlx::query` not `query!`) — determinism and
  correctness come from *our* type checker having already validated
  shapes; sqlx macros checking them again at target-compile time is
  redundant belt-and-suspenders we can drop.
- **Blueprint hygiene for pipeline binding** has sharp edges
  (name-pair attachment across suffixing). Mitigation: it lands in
  its own milestone with property-style tests (expand twice, assert
  no cross-talk), and the std blueprints are real consumers that
  would break loudly.
- **Optional types ripple** through the existing checker. Mitigation:
  optionals exist only as verb results with mandatory unwrapping —
  no optional fields on records in v0.14, which contains the blast
  radius.

## After v0.14

The boring 80% of behavior type-checks in the model, reusable
patterns are real library entries rather than documentation prose,
and declared security is enforced security. What remains is making
the *generated system* worthy of a team's production traffic — and
legible to the software around it — which is v0.15.
