# CIaC v0.15 — Operations & Reach (roadmap forecast)

> Forecast document. Assumes v0.13 (friction) and v0.14
> (expressiveness) have landed. Direction-setting; the v0.15 planning
> pass finalizes the tracing capability surface and the structured-
> fix schema. The deliberate cut across all three versions — a full
> TypeScript backend — is sequenced *after* this version (v0.16's
> headline), with the OpenAPI-generated TypeScript client below as
> the low-risk beachhead for that audience.

## The gap this version closes

By v0.14 a small team (or one agent) can express, generate, run, and
verify a real multi-service system in minutes. Three things still
separate that system from one a team confidently *operates*, and one
thing separates ciac from the software ecosystem around it:

1. **Debuggability across services.** ciac generates systems whose
   whole point is cross-service edges (`call`, streams, channels) —
   and gives the operator no way to follow one request across them.
   Logs and metrics exist as capabilities; distributed tracing does
   not. The first production incident in a `call` chain is currently
   solved with grep and vibes.
2. **The API boundary is a dead end.** The IR knows every method,
   path, request record, and response shape — and never emits an
   OpenAPI document. That single artifact is the adapter to the
   entire external tooling world: client generators, gateways,
   contract tests, other teams' agents.
3. **"Works on my compose" is not a delivery story.** Generated
   projects have Dockerfiles and tests but no CI definition; the
   deploy artifacts (k8s/Terraform) assume an image registry pipeline
   the user must invent per project.
4. **Auth stops at token validation.** Both auth providers validate
   tokens someone else must issue. For the dev loop and for tests
   that need "a user with scope X", there is no story but hand-rolled
   scripts — the most common real-world capability gap left.

Plus the agent thread running through the whole arc: diagnostics
carry prose `help`; making the mechanical ones carry **applyable
edits** turns an agent's check→edit iteration (and an editor's
quick-fix) from guesswork into a patch.

**v0.15 theme: a generated system a team can run in production and
point other software at — and a compiler whose error messages fix
themselves.**

## Pillar 1 — Distributed tracing (`tracing OpenTelemetry`)

- A new capability, opt-in like every other (`use { tracing
  OpenTelemetry; }`), closed-registry consistent. When present:
  - **Python**: OTel SDK + FastAPI/HTTPX auto-instrumentation; broker
    publishes carry `traceparent` in NATS headers / Kafka record
    headers; workers extract and continue the trace.
  - **Rust**: `tracing` + `opentelemetry-otlp` layers; the generated
    call clients inject `traceparent`; queue consumers extract it.
  - **Compose**: an `otel-collector` container plus Jaeger UI wired
    in dev (the same "real dev container" convention every capability
    already follows); `OTEL_EXPORTER_OTLP_ENDPOINT` in generated
    config with the k8s ConfigMap/Terraform outputs following suit.
- The load-bearing property, and its test: **one trace id spans an
  api → `call` → downstream handler → `publish` → worker chain.** The
  system-test generator (v0.8/v0.9 machinery) gains a trace-continuity
  check: hit an edge-bearing route, query the collector's API for the
  trace, assert the span tree crosses every hop. This is the v0.15
  equivalent of v0.9's capability round-trips — proving the wiring,
  not trusting it.
- Cut line: auto-instrumented spans only (HTTP server/client, broker
  produce/consume, db calls where the instrumentation library gives
  them for free). No custom span verbs in handler bodies in v0.15.

## Pillar 2 — OpenAPI generation (+ the TypeScript client beachhead)

- `ciac build` emits `openapi.json` per service (and a system-level
  index for multi-service programs): every `api` and every
  crud-expanded route, request records as component schemas (enums
  included — `FieldTypeKind` from v0.10 M1 already carries exactly
  what a JSON Schema needs), scoped routes carrying their v0.14
  `securitySchemes`/scope requirements, `/health` included.
- Discipline copied from the protocol schema (v0.10 M2): the document
  is derived from the IR by one serializer, snapshot-covered, and a
  staleness test keeps the checked-in examples' specs honest. For the
  Python target, FastAPI's own auto-generated spec is *replaced* by
  the ciac-emitted one (mounted as the app's openapi source) so both
  backends serve one truth rather than two near-duplicates.
- **`--client ts`**: a generated, dependency-free typed fetch client
  package (`clients/ts/`) — interfaces from records, one function per
  route, scope-bearing routes typed to require a token. Generated
  from the IR directly (not by shelling to an external generator), so
  it inherits determinism and golden coverage. This is deliberately
  the *smallest* TypeScript artifact worth shipping: it serves the
  TS-consumer audience now and de-risks the full v0.16 TS backend by
  proving the record→TS type mapping in isolation.

## Pillar 3 — Generated CI (`--deploy ci`)

- `--deploy ci` (repeatable alongside `k8s`/`terraform`) emits
  `.github/workflows/ci.yml` into the generated project: test job
  (uv/cargo, mirroring what `ciac verify` runs), image build job per
  Dockerfile, push-on-tag with registry/credentials as documented
  placeholders (`secrets.REGISTRY_*` — ciac emits the shape, never
  credentials), optional compose smoke job.
- Owned-file discipline applies (sidecars on conflict) — teams that
  rewrite their workflow keep their edits, same as any owned file.
- GitHub Actions only in v0.15, stated plainly (it's where the
  audience is; GitLab CI is a template away once the shape settles).

## Pillar 4 — `users`: a dev identity provider

- `use { users Keycloak; }` — the capability that makes OAuth2
  systems *runnable* without an external IdP:
  - compose gains a Keycloak container with a **generated seeded
    realm** (realm JSON import): a client, the scopes the program's
    routes declare (v0.14), and two dev users (one per scope posture);
  - `auth OAuth2`'s `issuer` defaults to the Keycloak container URL
    when `users` is present (explicit issuer still wins — prod points
    at the real IdP and the `users` container simply isn't deployed:
    k8s/Terraform emit nothing for it, disclosed as dev-only);
  - a generated `scripts/token.sh` (password-grant against the dev
    realm) so humans and generated tests can mint real tokens — the
    scoped-route 403/200 tests from v0.14 upgrade from crafted JWTs
    to tokens issued by a real IdP in the system suite.
- Explicitly out of scope, stated in docs: user CRUD in the model,
  registration/login UI generation, session management. `users` is a
  dev-and-test identity provider, not an identity *product* — the
  resource-server stance from v0.11 stands.

## Pillar 5 — Structured fixes: diagnostics that apply themselves

- A `fixes` field on diagnostics — in the `Diagnostic` type itself,
  serialized into the `--json` envelope (schema-versioned) and served
  as LSP code actions (the v0.12 deferral now paid): each fix is
  `{title, edits: [{file, line, column, end_line, end_column,
  replacement}]}` — the same resolved-position shape `JsonLabel`
  already uses.
- Seeded with the mechanical, unambiguous cases (the ones whose
  `help` text is already effectively an edit): missing capability for
  a step/verb (insert `queue NATS;` into the existing `use` block or
  synthesize one), unknown provider/capability with a close-match
  suggestion (rename edit), missing required attr (`issuer` on
  OAuth2 — insert with placeholder), unknown record/stream/field with
  nearest-name rename. Fixes are *offered*, never auto-applied by
  `check`; correctness bar: applying a fix must yield a program where
  that diagnostic is gone (property-tested exactly that way across
  the negative-fixture corpus).
- Agent loop payoff: `ciac check --json` → apply `edits`
  mechanically → re-check. The LSP quick-fix and the agent patch are
  the same data.

## Secondary items

- `ciac mcp` (v0.13) grows a `fix` tool (apply a named fix from the
  last check) once the schema lands.
- `docs/operations.md` (tracing, generated CI, the users capability,
  prod checklists); OpenAPI + client docs in `docs/deployment.md`'s
  orbit; README's pitch updated to include the spec/client artifacts.
- Provider table rows for `tracing`/`users`; `ciac describe` picks
  both up via the shared tables.

## Milestones

1. **M1 — OpenAPI**: IR→OpenAPI serializer, per-service emission,
   FastAPI spec unification, snapshots + staleness test over the
   example corpus.
2. **M2 — TypeScript client** (`--client ts`): record→TS types, per-
   route functions, golden coverage; live proof = generated client
   exercised against a running generated service (node available in
   CI).
3. **M3 — tracing, Python**: capability + OTel wiring + compose
   collector; trace-continuity system test on a call-edge example.
4. **M4 — tracing, Rust + cross-target proof**: Rust layers +
   propagation; the continuity test passes on a rust-built system
   (CI-delegated where local infra can't run it — disclosed).
5. **M5 — `--deploy ci`**: workflow generation, owned-file/sidecar
   coverage, docs. (Small by design — breathing room in the version
   that carries two runtime pillars.)
6. **M6 — `users Keycloak`**: seeded realm generation, issuer
   defaulting, token script, system-suite upgrade of the scoped-route
   tests; live proof against a local Keycloak where the sandbox
   allows, CI-delegated otherwise.
7. **M7 — structured fixes**: `fixes` on Diagnostic, JSON envelope +
   schema bump, LSP code actions, the fix-must-clear-its-diagnostic
   property test over the negative corpus, MCP `fix` tool.
8. **M8 — docs, reconciliation, version 0.15.0**, full verification,
   and the arc analysis for v0.13→v0.15.

## Risks

- **Two heavyweight runtime deps in one version** (OTel SDKs,
  Keycloak). Mitigation: both are opt-in capabilities — programs that
  don't declare them generate byte-identically to v0.14 (golden-
  proven), so the blast radius of a bad integration is bounded to the
  programs that asked for it.
- **Keycloak is heavy for a dev container** (memory, startup time).
  Mitigation: it only exists behind `users`; startup is absorbed by
  the existing health-probe/bounded-backoff machinery; if it proves
  too heavy in practice the capability's provider registry leaves
  room for a lighter OIDC dev issuer without surface change.
- **Trace-propagation correctness across brokers** is exactly the
  kind of claim that's easy to assert and wrong in practice.
  Mitigation: the trace-continuity system test *is* the milestone's
  acceptance bar — no green continuity test, no milestone.
- **Fix suggestions that don't fix.** Mitigation: the property test
  (apply fix → diagnostic gone) runs over the whole negative corpus;
  any fix that can't meet that bar ships as prose `help`, not a fix.
- **OpenAPI drift vs FastAPI's own opinions** (route naming, implicit
  422s). Mitigation: ciac's document is the single source served by
  both backends; divergence is a snapshot failure, not a runtime
  surprise.

## After v0.15

The system a team generates is observable, continuously integrated,
authenticated end-to-end in dev, and speaks OpenAPI to the rest of
the world; the compiler hands both editors and agents patches instead
of prose. The natural v0.16 headline is the full TypeScript backend
(de-risked by M2's type mapping), with GCP Terraform parity and
custom span/metric verbs as the supporting cast — but as v0.12
closed: that decision deserves to be re-asked against real usage of
v0.13→v0.15, not pre-committed here.
