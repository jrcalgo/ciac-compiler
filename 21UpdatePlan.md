# CIaC v0.21 — Reach: One Deliberate Breadth Token, Re-asked from Usage (roadmap forecast)

> Forecast document. Assumes v0.16–v0.20 have landed and begins by
> reconciling those assumptions against the live code/docs that actually
> ship. Direction-setting; this is a selection plan, not a cumulative
> feature list.
>
> There are three candidates and one breadth slot:
>
> 1. a brownfield OpenAPI bridge;
> 2. an opt-in generated admin UI;
> 3. a full TypeScript backend.
>
> Exactly one delivery track is activated after an evidence/feasibility
> checkpoint. A deliberately bounded OpenAPI-plus-admin adoption track is
> eligible only when both minimum slices fit the same technical and
> permanent-maintenance budget as one ordinary track. TypeScript remains
> exclusive. Track AB is not a fourth breadth slot; it consumes the single
> token only when both bounded minima fit one delivery and maintenance
> budget. “No breadth feature met the bar” is a valid outcome.
>
> No track is deployment maturity. There is no hosted control plane,
> infrastructure application, admin hosting, CDN, gateway provisioning,
> identity product, secret distribution, or rollout orchestration.
>
> Confidence follows the selected track, not its position in this file:
> the bounded OpenAPI bridge is currently high-conviction; admin and a
> third backend remain hypotheses with explicit evidence gates.

## The gap this version closes

By v0.15, CIaC already has:

- Python and Rust bundled runtimes;
- a documented external backend protocol and narrow Go reference;
- generated OpenAPI;
- a generated TypeScript client;
- typed `external_http` capability plumbing but an untyped POST-like
  handler verb;
- typed CRUD resources;
- safe regeneration and verification.

The assumed v0.16–v0.20 arc deepens domain semantics, simulation,
evolution, failure correctness/policies, and provenance.

Those five depth versions optimize time-to-completion for people already
inside a CIaC system. Reach is about the walls faced by people and systems
outside it:

- **inbound:** no OpenAPI → checked `.ciac` skeleton path;
- **outbound:** `external_http`, present since the early capability model,
  still abandons CIaC's type discipline at a third-party boundary;
- **agent:** third-party request/response glue is some of the
  highest-error code an agent writes by hand;
- **end user:** a generated backend has no usable non-developer surface,
  despite the long-standing Django-admin lesson that records plus policy
  can create enormous reach;
- **runtime audience:** the TypeScript client de-risked type mapping, but
  the third-backend decision cannot remain permanently unasked.

The next adoption barrier could plausibly be any of three:

- Existing teams cannot start greenfield; they need typed use of services
  described by OpenAPI.
- Existing CIaC systems work but still need recurring internal operations
  screens over CRUD/policy data.
- TypeScript teams like the model/client but will not adopt a Python or
  Rust runtime.

All are credible. None is established by a roadmap sentence.

`15UpdatePlan.md` called a TypeScript backend a natural possible v0.16
headline and explicitly said to re-ask the decision from real usage.
That second clause governs. Likewise, the current lean toward OpenAPI plus
admin reflects depth leverage, not permission to waive evidence or carry
two unbounded products.

**v0.21 theme: spend one breadth token where observed use reaches the
current boundary. Deepen an existing surface or add one host, but do not
quietly create three permanent maintenance lines.**

The goal is to lower walls—integrate CIaC's typed world with existing
systems, make a selected generated system usable to a new audience, or
make an honest host-language decision.

## Factual starting points that survive the forecast

The M0 reconciliation updates these facts for the actual v0.20 tree.

### OpenAPI is outbound only

The live v0.15 serializer:

- emits OpenAPI 3.0.3 from `SystemModel`;
- covers APIs, CRUD, records/enums, bearer auth/scope metadata, health;
- omits realtime channels;
- is the single spec served by both targets.

The TypeScript client is IR-driven and already proves one
record/route-to-TypeScript mapping.

Inbound support must reuse or deliberately reconcile that mapping. It
cannot create a second definition of CIaC UUIDs, timestamps, enums,
relations, validation, or error responses.

### `external_http` is intentionally untyped

The current capability has `base_url`; HIR has one generic HTTP effect
returning `Json`; Python/Rust lower it through httpx/reqwest. No operation
IDs, path/query params, response records, security scheme, or contract
digest reach IR/codegen.

A typed OpenAPI bridge therefore touches syntax dependency loading, sema,
HIR, model/protocol, both lowerers, configuration, dev watching, source
hashing, diagnostics, and tests. It is not just another client template.

### CRUD is a useful target-neutral admin model

`ResourceCtx` already carries names, routes, records, page size, auth, and
scope metadata. v0.19 is assumed to add server-enforced ownership/policy
metadata.

That is a good generator input, but not proof that a browser can enforce
policies. An admin artifact must remain an ordinary policy-constrained
client, never a privileged bypass.

### The backend seam is real but incomplete

The external protocol proves deterministic owned/seeded output. Its
v0.15 typed-handler representation contains opaque `NodeId`s without HIR,
and CLI validation/migrations/generated guidance hard-code Python/Rust
assumptions.

Later versions may have extended the protocol. M0 must inspect the actual
state. A “full TypeScript backend” must still lower every current
construct/provider/policy and participate in first-class verification;
routes-and-records-only is another spike, not the candidate.

## One token has two budgets

Breadth consumes both delivery capacity and permanent maintenance.

### Technical delivery budget

Use non-calendar evidence:

- number/invasiveness of compiler layers changed;
- number of new public schemas/formats;
- target/provider/browser matrices;
- security-critical paths;
- required real fixtures;
- golden/system-test expansion;
- CI runtime/resource cost;
- migration/compatibility obligations.

Reserve a fixed portion of the version for integration, regression,
documentation, and release reconciliation. A candidate fits only when
its conservative technical scope—including those costs—fits the same
bounded release envelope recent depth versions successfully carried.

The combined adoption track fits only when:

```text
OpenAPI minimum
+ admin minimum
+ shared integration
- demonstrated shared implementation
<= one breadth budget
```

“Demonstrated shared” means code exercised by both spikes. Browser
security and OpenAPI dialect maintenance do not disappear because both
touch TypeScript.

### Permanent maintenance budget

For each candidate record the steady-state matrices added to every future
language/provider release:

- schema dialects/reference forms;
- runtime targets and lowerings;
- provider/capability combinations;
- browser/Node support lines;
- policy shapes;
- live fixtures;
- dependency/security update cadence;
- CI minutes/storage.

The chosen track must fit without displacing core compiler work on every
subsequent version.

TypeScript is exclusive even if the initial implementation appears to
leave room: a third backend multiplies every later semantic decision.

## Evidence without invasive telemetry

CIaC adds no automatic usage reporting, installation ID, background
request, or hosted analytics service.

### Active-use definition

An active project is a non-example CIaC program whose maintainer can show
a recent successful build/verify against the current release. Tutorial
copies, repo fixtures, and unbuilt experiments do not count independently.

An independent team is a separately maintained codebase and decision
group. Several repositories from one experiment are one evidence source.

### Local-only usage receipt

M0 may add a local `ciac usage-report <paths...>` helper only if needed.
It:

- inspects explicitly provided paths;
- performs no network access;
- prints locally for deliberate sharing;
- shows the full report first;
- has a versioned, testable schema.

Allowed aggregate fields:

- CIaC/target versions;
- bucketed service/record/API/CRUD/policy/external HTTP/inline/extern
  counts;
- field-type category counts;
- single/multi-service;
- optional client/admin artifacts present.

Forbidden:

- names, paths, repository URLs, source text;
- OpenAPI contents/servers;
- auth issuers/audiences/scopes/header names;
- hashes or persistent IDs;
- handler bodies;
- exact timestamps or secrets.

Refusing to share has no product consequence.

### Qualitative evidence

Every counted request needs:

1. current project/target;
2. blocked workflow;
3. current workaround;
4. recurrence and maintenance burden;
5. smallest useful slice;
6. real or minimized fixture;
7. acceptance requirements;
8. willingness to pilot and report failures.

Stars/downloads are context, not a candidate score.

The checkpoint is evidence-triggered: selection occurs only after each
eligible candidate either meets its artifact/team floor or is recorded as
ineligible. Lack of response is not extrapolated into demand.

## Candidate evidence floors

### A — OpenAPI bridge

Require:

- three active projects from three independent teams;
- at least two currently use generic external HTTP or handwritten client
  code for a service with OpenAPI;
- redistributable/minimized specs for the operations they use;
- named operation subsets;
- two pilots against a real or faithful local server;
- the bounded supported subset covers at least 80% of pilot-used
  operations.

Coverage is over operations actually used, not every path in a vendor's
complete document.

### B — Generated admin UI

Require:

- three active projects from three teams;
- each has several real typed CRUD resources;
- at least two use v0.19 ownership/policy behavior;
- recurring internal data operations or an existing handwritten admin;
- safe minimized policy fixtures;
- two pilots with distinct permission/ownership postures.

General interest in a dashboard is insufficient.

### C — Full TypeScript backend

Require:

- three adoption-ready projects from three teams where runtime language
  is the stated blocker;
- at least two need a TypeScript server, not merely the existing client;
- concrete requirements beyond echo routes, including typed handlers and
  several capability families;
- willingness to validate the complete current provider/policy matrix;
- evidence that partial preview would not solve the stated blocker.

Existing `--client ts` use is corroborating evidence, not proof of server
runtime demand.

The rejection criteria are equally explicit: if reported need is
consuming CIaC output rather than running a TypeScript host, or if teams
can sustainably author an external backend, Track C does not win. The
decision record then names protocol conveniences, a conformance suite,
and backend-template scaffolding as the external-backend-DX follow-up
rather than beginning a hidden partial TypeScript target.

## Selection

Every candidate passes hard gates:

1. evidence floor;
2. explicit minimum useful slice;
3. conservative technical scope fits;
4. permanent maintenance fits;
5. security/policy invariants are testable;
6. fixtures can be retained safely;
7. at least two pilots;
8. no hidden future prerequisite.

Eligible candidates are scored:

| Criterion | Weight | Question |
|-----------|-------:|----------|
| adoption blockage | 30 | How many independent active projects stop here? |
| recurrence/cost | 20 | How often and expensively is the workaround repeated? |
| observed-slice coverage | 15 | How much submitted use does the minimum solve? |
| depth leverage | 10 | How much shipped records/CRUD/policies/OpenAPI/client/backend machinery is deepened? |
| delivery confidence | 15 | How strong are spike, fixtures, and subsystem inventory? |
| maintenance fit | 10 | How comfortably does steady-state ownership fit? |

Evidence quality is recorded:

- A: artifact + active project + pilot;
- B: active project/workflow, no retainable artifact;
- C: survey/issue without exercised project.

Tie-break:

1. independently blocked projects;
2. lower maintenance;
3. deeper reuse;
4. smaller semantic expansion.

This expresses the current lean toward A/B without making it a foregone
conclusion.

## Shared M0 — reconcile, spike, decide

### Reconcile actual v0.20

Inventory:

- live language/provider table;
- v0.16 field/relation/validation model;
- v0.19 policy enforcement and protocol representation;
- v0.20 provenance requirements;
- OpenAPI/client shape;
- external protocol and portable HIR status;
- generated validation/CI paths;
- all-example/system-test matrix and cost.

Any contradicted assumption in this file is removed or recosted.

### Three bounded spikes

These are feasibility instruments, not hidden implementation starts.

**OpenAPI spike**

- normalize submitted specs in memory;
- support-report by operation;
- resolve supported local refs;
- map live CIaC types/security;
- apply an explicit operation allowlist and prove excluded operations do
  not enter the normalized model;
- generate one typed call signature;
- no public syntax/import command unless selected.

**Admin spike**

- static wireframe from one `ResourceCtx` plus policy summary;
- two users against policy-filtered list/get/write;
- threat model token handling, XSS, CSRF, destructive actions,
  policy divergence, regeneration;
- no public `--admin` unless selected.

**TypeScript spike**

- type-check/run one service with typed API/CRUD, inline and extern
  handlers, database, outbound capability, policy;
- explicitly solve or demonstrate portable-HIR blocker;
- measure dependency/generated-CI cost;
- no registered preview target unless selected.

### Decision record

Publish:

- anonymized evidence counts/quality;
- artifact coverage;
- scores;
- technical/permanent cost;
- selected track;
- rejected tracks/reasons;
- frozen minimum and cut lines.

Valid mutually exclusive outcomes:

- Track A — OpenAPI;
- Track B — admin;
- Track AB — bounded adoption combination;
- Track C — TypeScript backend;
- Track 0 — no breadth feature.

Runner-up work stops after selection. A late failure triggers explicit
re-plan or Track 0, not automatic switching.

## Track A — Brownfield OpenAPI bridge

### Outcome

CIaC consumes a bounded deterministic OpenAPI contract and exposes
supported upstream operations as typed verbs on Python and Rust. A
companion `ciac import` emits an editable skeleton with explicit
omissions.

It is not arbitrary OpenAPI support and not reverse engineering of a
complete backend.

### Opt-in surface

Illustrative:

```ciac
service BillingBridge {
    use {
        external_http stripe {
            spec: "./contracts/stripe.openapi.json";
            base_url: "https://api.stripe.example";
            security: BearerAuth;
            operations: [CreatePayment, GetPayment, RefundPayment];
        }
    }

    handler Charge(req: StripeCreatePayment) -> StripePayment {
        return stripe.createPayment(req);
    }
}
```

Rules:

- `spec` opts in; existing base-URL-only behavior remains unchanged;
- `operations` is a required audited allowlist; only those operation IDs
  enter HIR/codegen;
- instance name namespaces operations (`stripe.createPayment`);
- operation IDs normalize deterministically to lower-camel handler
  methods (`CreatePayment` → `createPayment`), with post-normalization
  collisions rejected;
- explicit `base_url` overrides the spec server;
- one safe non-templated HTTPS server may be used when omitted;
- credentials never appear in source/spec output; runtime env provides
  them;
- generic `external_http.request` remains source-compatible.

Exact syntax freezes after spike, but opt-in and instance namespace are
required.

The allowlist is the dependency declaration. Vendor specs routinely
contain thousands of operations; generating all of them would create an
unauditable client, unstable output, and no answer to “which parts of this
vendor API does the system depend on?”

### One normalizer

A target-neutral normalizer (likely `ciac-openapi`) emits:

```text
NormalizedOpenApi
  operations(method, path, params, body, response, security)
  schemas(normalized CIaC-compatible types)
  stable names
  diagnostics/support report
```

Typed verbs and `ciac import` consume the same representation. No second
mapping exists.

The type vocabulary is the same `FieldTypeKind` seam proven first by the
v0.10 external-backend protocol and then in reverse by v0.15 OpenAPI
emission. Inbound support extends that one mapping rather than inventing
an importer-only type system.

### Inputs/dependencies

Minimum:

- local JSON only;
- path relative to declaring `.ciac`;
- spec included in source hash/manifest dependency;
- `ciac dev` watches it;
- diff/verify react to changes;
- the normalized spec/allowlist digest appears in v0.18 semantic diff, so
  a vendor contract bump is a typed review event rather than unexplained
  generated churn;
- normalized digest is reproducible;
- missing/unreadable is source-located diagnostic;
- no remote fetch.

Vendoring avoids mutable/network/auth/supply-chain semantics in every
check.

### Version/reference limits

Initial candidate supports OpenAPI 3.0.x JSON. If pilot corpus is
primarily 3.1, the spike must fit 3.1 honestly or mark Track A
ineligible—never parse it as 3.0 silently.

Supported:

- internal JSON Pointer component-schema refs;
- repeated refs;
- escaped pointer segments;
- deterministic resolution.

Rejected:

- remote/sibling refs;
- unresolved refs;
- dynamic refs/anchors;
- recursive/cyclic schemas unless v0.20's live type system explicitly
  supports them;
- conflicting normalized names.

Unsupported refs never degrade silently to `Json`.

### Type subset

The floor, expanded only where live v0.20 semantics support it:

| OpenAPI | CIaC |
|---------|------|
| string | `String` |
| string/uuid | `Uuid` |
| string/date-time | `Timestamp` |
| string enum | enum |
| integer | `Int` |
| number | `Float` |
| boolean | `Bool` |
| explicitly unconstrained object | `Json` |
| object component | generated prefixed record |
| empty success | `Unit` |

Every imported range/format/required/nested relation must either map to
the v0.16 validation/type model or reject. Lossy fallback is not default.

Potentially unsupported, depending on live model:

- optional/nullable;
- oneOf/anyOf/allOf/not/discriminators;
- tuples/recursive objects;
- anonymous nested objects;
- typed additional properties;
- binary/file/streaming;
- exact decimal;
- read/write projections that cannot preserve one record contract.

### Operation subset

A supported operation has:

- unique operation ID or deterministic imported name;
- supported HTTP method/path;
- representable required path/query params;
- zero/one JSON body;
- one usable success body or equal-body success variants;
- supported security.

Minimum excludes optional params when type system cannot express them,
forms/multipart/files, streaming, inferred pagination, retries/rate
limits/idempotency conventions, content negotiation, arbitrary response
headers.

Name normalization collisions are hard errors naming both operations.

### Security

Minimum:

- none;
- bearer token injection;
- one header API key.

No credential import. Env names derive deterministically from instance.

Excluded: OAuth acquisition/refresh, OIDC discovery, mTLS, signed
requests, cookie/query keys, AND combinations, runtime scheme choice.

### Runtime parity

Python/httpx and Rust/reqwest typed methods must equivalently:

- encode path/query;
- serialize only declared body;
- inject selected auth;
- reject non-success before decode;
- decode/validate response type;
- redact credentials and bound error excerpts;
- preserve tracing.

Generated clients reuse the established inter-service layout:
`app/clients/<instance>.py` and `src/clients/<instance>.rs`. They do not
create a third SDK-shaped output tree for the same injected runtime
dependency.

### Simulation integration

The v0.17 recording/scripted `external_http` fake becomes spec-aware for
selected operations. Scenario responses are validated against the
normalized vendor response schema before the handler runs, preventing a
test from stubbing a response the real API could never produce. This
makes third-party integration logic infrastructure-free without claiming
the fake proves the vendor wire implementation.

### `ciac import`

```sh
ciac import openapi.json --name stripe > contracts/stripe.ciac
ciac import openapi.json --name stripe --out contracts/stripe.ciac
```

- stdout default;
- `--out` refuses overwrite;
- deterministic;
- same normalizer/names;
- stderr summary and JSON emitted/skipped/rejected report;
- nonzero when no usable skeleton/fatal contract error;
- emits user-owned records and illustrative external declaration/handler
  signatures;
- emits supported API declarations with method/path/request type and
  explicit `TODO` pipelines rather than inventing behavior;
- maps supported security schemes to commented suggested `use` entries,
  never credentials;
- comments list omitted constructs;
- never infers DB, policy, worker, deployment, or business pipeline;
- rerun-to-stdout plus review is the update workflow; no merge engine.

Import is intentionally one-way, lossy scaffolding—the same v0.12
`ciac new` ethos applied to a contract. Its output must pass
`ciac check`; “80% starting point plus a complete unmapped report” is the
claim, not OpenAPI round-tripping.

### Track A touchpoints

- new normalizer crate/workspace registration;
- source dependency loading/hash/watch;
- sema external declaration and typed receiver operations;
- HIR operation-bearing HTTP verb;
- IR external contract side table;
- shared model/protocol operations/security/types;
- both lowerers/client templates/config;
- CLI and MCP `import` using the same normalizer/report;
- vocab/LSP/describe/docs/errors, including `spec`/`operations`
  completion and hover on imported operation names;
- real-spec corpus, mock-server parity, protocol schema.

### Track A acceptance

1. 80%+ pilot-used operations supported.
2. Deterministic normalization/import.
3. Unsupported constructs never silently degrade.
4. Every ref/security/type rejection has fixture.
5. Spec changes affect dev/diff/verify.
6. Python/Rust send equivalent method/path/query/header/JSON.
7. Empty/non-2xx/malformed response and credential redaction covered.
8. Generic no-spec programs remain byte-identical.
9. External protocol independently describes operations.
10. Live docs publish conspicuous supported/unsupported table.
11. Imported skeletons pass `ciac check`, and minimized representative
    Stripe, GitHub, and Petstore subsets remain in compatibility tests
    where licensing permits.

### Track A permanent cost/cuts

Owns OpenAPI dialect, refs, type/security mapping, real-spec corpus,
name stability, two clients, diagnostics, protocol review.

Cuts:

- remote fetch;
- registry-cached remote specs (a future option only after mutable-input
  and trust semantics are designed);
- arbitrary 3.1/YAML for convenience;
- full SDK;
- OAuth flows;
- proxy/gateway/webhooks;
- retry/rate-limit inference;
- binary/file/AsyncAPI;
- OpenAPI round-tripping; import is scaffolding and CIaC-emitted OpenAPI
  remains the generated contract;
- gRPC, GraphQL, and SOAP bridges, each of which needs its own type/wire
  design;
- deployment changes.

## Track B — Generated admin UI

### Outcome

Generate an opt-in static admin application for explicitly selected typed
CRUD resources. It uses ordinary CRUD APIs and server-enforced v0.19
policies. It never receives direct database access or a privileged bypass.

### Double opt-in

```ciac
crud Customer: Customer {
    admin: true;
    read_scope: "customers:read";
    write_scope: "customers:write";
}
```

```sh
ciac build system.ciac --target python --out ./build --admin
```

Both source and build flag are required. Requesting admin with no eligible
resource fails rather than emitting an empty/broad UI.

### Artifact

Target-independent compiler-owned project:

```text
admin/
  package.json
  tsconfig.json
  index.html
  src/
    main.ts
    api.ts
    auth.ts
    resources.ts
    render/
  tests/
  README.md
```

Minimum:

- navigation;
- paginated list/get;
- create/full update/delete confirmation;
- String/Int/Float/Bool/Uuid/Timestamp/enum and bounded safe JSON display;
- loading/empty/401/403/not-found/validation/server states.

Reuse one refactored TS type/CRUD request core from the existing client.
Framework choice is frozen from spike based on total maintenance, not
preference.

The preferred minimum is a dependency-free browser runtime and a
TypeScript-only build: no framework runtime, CDN script, or second fetch
stack. If the spike chooses a framework, it must demonstrate lower total
security/maintenance cost than this baseline.

### Policy safety

The browser is never authoritative.

- Server filters lists before serialization.
- Every item/write operation enforces policy.
- UI receives no bypass token/endpoint.
- Hidden button is UX only; direct HTTP denial tests are mandatory.

Admin eligibility:

- typed record;
- authenticated;
- explicit effective policy for enabled operations;
- policy-filtered list guarantee;
- stable response shape without leaked conditional fields;
- supported field/ID/update semantics;
- no `Reference` field while v0.16's deliberate relation-aware CRUD cut
  remains in force.

Ineligible `admin: true` is a source diagnostic, never silent omission.

If policy depends on user/row, the UI may show an action and handle 403;
it does not embed a policy interpreter. Conditional field-mask resources
are excluded unless server/client shapes prove safety.

### Auth

Minimum accepts an already issued bearer token:

- memory only;
- cleared on reload/sign-out;
- never source/bundle/URL/localStorage/log;
- sent only to configured service origin.

No registration/login/recovery/session/user management. Existing dev
Keycloak/token tooling may supply tokens. If browser OIDC is required by
pilots and does not fit, Track B is ineligible rather than smuggling in an
identity product.

### Browser safety

- escaped text, no unsanitized HTML/eval/CDN scripts;
- restrictive CSP;
- bearer headers, no ambient cookie;
- explicit origin;
- distinct 401/403;
- destructive confirmation/double-submit prevention;
- bounded/redacted errors;
- script-shaped values tested inert;
- accessibility: labels, keyboard/focus, headings, contrast.

### Forms

| CIaC field | UI |
|------------|----|
| String | text |
| Int/Float | validated numeric |
| Bool | checkbox |
| Uuid | text/read-only generated ID |
| Timestamp | RFC 3339 display/input |
| enum | select |
| Json | escaped read-only structured view initially |

No invented defaults. Create/update use existing CRUD projections.
Unknown newer fields are ignored safely; missing required fields are a
visible contract error. Relation-bearing records remain ineligible until
the core CRUD/storage model supports them; a relationship editor is depth
work, not something Track B infers from nested output.

### Ownership/regeneration

Entire `admin/` is owned:

- customization remains in `.ciac`;
- edits produce normal conflicts/sidecars;
- no seeded component/plugin/theme compatibility surface;
- policy/record change regenerates deterministically;
- removing admin uses normal orphan rules.

The artifact is not a UI framework: no custom pages, dashboard widgets,
or plugin API; bounded theming uses generated CSS variables only. A team
that outgrows the surface keeps the generated TypeScript client and
OpenAPI contract as the supported path to its own application.

### Track B touchpoints

- v0.19 effective policy/ownership summary in IR/model;
- CRUD `admin` attr and eligibility pass/diagnostics;
- shared TS record/client core;
- target-neutral admin generator;
- CLI/MCP generation option and manifest recipe;
- generated CI admin typecheck/tests;
- vocab/LSP/describe/docs/AGENTS;
- two-backend/two-user browser fixtures.

No backend route gains admin bypass.

### Track B acceptance

1. No files without both opt-ins.
2. Ineligible resource errors at source.
3. Admin type-check/build/tests clean.
4. Same artifact works against Python/Rust.
5. Two-user list/get policy isolation.
6. Direct denied create/update/delete tests.
7. No token in source/bundle/snapshot/URL/storage/log.
8. XSS-shaped values inert.
9. confirmation/no double action.
10. deterministic regeneration/sidecars.
11. no-opt-in output byte-identical.
12. selected-browser and accessibility tests.
13. no hosting/deployment/identity claim.
14. emitted static browser artifact runs without an npm install or
    runtime dependency; its source passes the pinned `tsc` check in CI.
15. the v0.16 commerce/domain flagship is driven end-to-end through the
    generated admin against both backend targets with two policy postures.

### Track B permanent cost/cuts

Owns frontend dependencies/security/accessibility, record controls,
CRUD/policy compatibility, browser matrix, two-backend tests, Node support.

Cuts:

- hosting/CDN;
- DB-direct/bypass;
- policy editor/simulator;
- users/roles/login;
- dashboards/analytics;
- arbitrary APIs;
- relationship editor unless in bounded selected minimum;
- bulk import/export, files, rich text, realtime;
- plugin/component framework;
- hand-owned frontend fork.

## Track C — Full TypeScript backend

### Outcome

```sh
ciac build system.ciac --target typescript --out ./build
ciac verify system.ciac --target typescript --out ./build
```

Generates a complete Node/TypeScript runtime with parity against every
construct/provider/policy supported by Python/Rust at the checkpoint.
“Full” is the release bar, not future intent.

### Full current parity

Includes, at minimum and updated for v0.16–v0.20:

- single/multi-service;
- all records/errors/enums/lists/references/validation;
- APIs/CRUD/pipelines/workers/jobs/channels/calls;
- inline and extern handlers;
- complete expression/effect vocabulary;
- transactions/outbox/idempotency/ownership/lints where target-relevant;
- Postgres/MySQL/SQLite, Redis, NATS/Kafka;
- JWT/OAuth2;
- object/email/search/external HTTP;
- scheduler/realtime/logging/metrics/tracing;
- OpenAPI, migrations/evolution, simulation/provenance;
- owned/seeded regeneration;
- compose/system verification/generated CI.

If this does not fit one budget, Track C is ineligible. A subset may
remain an internal spike but is not released as “full backend.”
The parity list is re-evaluated against the M0 inventory; any current
construct without a funded lowering, simulation adapter, and verification
path makes Track C ineligible.

### Runtime validation

TypeScript interfaces erase at runtime. The backend generates runtime
schemas/validators reused for:

- route/worker ingress;
- relation/field validation;
- external/call response decoding;
- handler records;
- database hydration.

`request.json() as T` is not parity.

### Portable handler/HIR debt

If the protocol still exposes opaque handler IDs, Track C must add a
portable resolved projection:

```text
PortableHandler
  params/return/locals
  typed statements/expressions
  capability instance + operation
  named records/tables/streams
  transaction/outbox/idempotency/provenance metadata
```

No unresolved graph-only IDs without referents.

The bundled TypeScript backend lowers from this projection even in
process, dogfooding the external representation. Protocol version/schema
and Go compatibility remain tested.

### Bundled backend and target descriptor

Add:

```text
crates/ciac-backend-typescript/
  src/lib.rs
  src/lower.rs
  templates/
```

Generator stays Rust; runtime is TypeScript.

Centralize host assumptions in a target descriptor:

- marker/manifest files;
- migration/seeded paths;
- validation commands;
- generated CI setup/test;
- worker command;
- URL schemes/data mount;
- generated AGENTS guidance.

Replace Python-vs-Cargo fallbacks in CLI/codegen. Scattering a third
string branch fails maintainability.

The framework, Node LTS, package manager, validator, database/broker
libraries, and lock strategy are chosen by the working spike and pinned
to one support line.

### Expected output

```text
package.json
lockfile
tsconfig.json
Dockerfile
src/
  main.ts config.ts state.ts error.ts schemas.ts
  db.ts auth.ts observability.ts queue.ts
  routes/ services/ logic/ workers/ clients/
tests/
migrations/
openapi.json
docker-compose.yml
```

Inline logic is owned; extern/classic handlers and migrations are seeded.
Infrastructure clients are lazy enough for static verify. Strict TS has
no suppressed type errors.

### Track C acceptance

1. Every Python/Rust example builds TypeScript unless a newly added
   provider has an explicit checkpoint-approved exception.
2. No unsupported live-language construct.
3. Strict typecheck/lint/tests without infrastructure.
4. Inline HIR from portable model; extern typed/seeded/safe.
5. CRUD/migrations all DB engines.
6. NATS/Kafka worker semantics.
7. Auth/policy outcomes match both targets.
8. call/channel/capability/system tests.
9. tracing/provenance/simulation parity.
10. shared OpenAPI, not framework auto-spec.
11. first-class `ciac verify`.
12. real generated CI recipe.
13. multi-service compose.
14. independently usable protocol typed handlers.
15. Go/protocol fixtures remain green.
16. no unrelated Python/Rust churn.
17. CI/resource cost inside checkpoint budget.

### Track C permanent cost/cuts

Owns a third lowering/provider/policy/runtime/test/dependency matrix in
every future version—the highest permanent cost and distinct largest
audience upside.

Cuts:

- admin/frontend;
- inbound OpenAPI;
- TypeScript-only providers;
- multiple framework/package-manager lines;
- serverless output;
- new deployment targets;
- full plugin marketplace/discovery;
- partial parity marketed as full.

## Track AB — Combined bounded adoption track

OpenAPI and admin both deepen shipped surfaces:

- A deepens outbound OpenAPI/types/client/external HTTP.
- B deepens records/CRUD/client/v0.19 policy.

That makes a combined adoption theme coherent, but coherence is not
capacity.

AB is eligible only when:

1. A and B independently meet evidence floors;
2. each has two pilots;
3. each passes its safety gate;
4. combined conservative delivery fits;
5. combined steady-state maintenance fits;
6. shared type/client implementation is demonstrated;
7. neither drops mandatory acceptance;
8. combined CI cost fits one ordinary track.

Demand is not double-counted when the same teams request both.

AB includes only minimum slices:

- local bounded OpenAPI JSON;
- typed operations on Python/Rust;
- deterministic import skeleton;
- explicit admin-visible typed CRUD;
- static CRUD UI;
- server-authoritative policy;
- memory-only bearer token;
- compiler-owned artifact;
- shared TS types/client core.

No stretch item enters AB. If fitting both requires lossy HTTP types,
unsafe list filtering, one-backend support, weak credential handling, or
broken regeneration, AB does not fit.

Selecting AB consumes the one token. TypeScript receives no
implementation milestones.

## Shared documentation and tool obligations

Whichever adoption track wins:

- add `docs/integration.md` for spec loading/allowlists, unsupported-
  construct rejection/diagnostics, one-way import, and the exact admin
  boundary where applicable;
- extend v0.18 semantic diff so vendor-spec/operation allowlist changes
  and admin-visible resource/operation changes are typed changelist
  entries;
- keep `vocab.rs`, `ciac describe`, LSP hover/completion, MCP schemas,
  and generated `AGENTS.md` synchronized with the selected surface;
- update the README reach story and publish a v0.16→v0.21 retrospective
  explaining why this one breadth token earned its cost.

## Milestones by selected track

### Common

1. **M0 — evidence/feasibility:** reconcile actual v0.20, optional local
   receipt, structured project evidence, three bounded spikes, security
   review, matrix/maintenance cost.
2. **M1 — selection:** publish scorecard; choose exactly A, B, AB, C, or
   0; freeze minimum/cuts.

### Track A

3. **A1 — normalizer:** local versioned JSON, refs/types/security,
   dependency tracking, diagnostics.
4. **A2 — typed IR surface:** instance verbs, operation-specific
   typechecking/HIR/model/protocol.
5. **A3 — Python/Rust clients and simulation:** equivalent
   wire/auth/error behavior plus spec-aware scripted fake validation.
6. **A4 — CLI/MCP import:** deterministic user-owned skeleton/report
   through one normalizer.
7. **A5 — pilots/reconciliation:** real specs/servers, docs, examples,
   goldens, release.

### Track B

3. **B1 — policy/admin model:** opt-in, effective policy metadata,
   eligibility diagnostics.
4. **B2 — shared TS core:** one record/CRUD mapping.
5. **B3 — safe read path:** app/auth/nav/list/get/two-user tests.
6. **B4 — write path:** create/update/delete, validation, confirmations,
   direct denial, regeneration.
7. **B5 — hardening/pilots:** browser security/accessibility/support,
   docs/goldens/release.

### Track AB

3. **AB1 — shared contract/type core:** bounded normalizer, TS mappings,
   dependencies, policy eligibility.
4. **AB2 — OpenAPI minimum:** typed verbs, spec-aware simulation, and
   CLI/MCP import.
5. **AB3 — admin minimum:** CRUD UI/auth/policy behavior.
6. **AB4 — combined conformance:** mock server, both backends, two-user
   browser, determinism/security.
7. **AB5 — both pilot sets/reconciliation.**

### Track C

3. **C1 — portable handler/target seams:** protocol/schema, target
   descriptor, Go compatibility.
4. **C2 — project/API/CRUD/auth/policy/runtime validation and verify.**
5. **C3 — typed logic/databases/migrations/cache/outbound effects.**
6. **C4 — async/operational parity:** brokers/workers/jobs/calls/channels/
   remaining capabilities/tracing/provenance, plus a first-class
   TypeScript adapter for v0.17 `ciac-sim` using the same
   quiescence/replay corpus as Python and Rust.
7. **C5 — all-example/provider/system/CI/docs matrix.**

### Track 0

3. **0.1 — publish non-selection:** retain evidence/feasibility report,
   reconcile factual docs found in M0, record whether external-backend
   conformance/template DX is the right follow-up, and add no hidden
   breadth API or partial target.

## Cross-track invariants

Whichever wins:

- unselected tracks get no public flags/syntax/placeholders;
- diagnostics stay append-only;
- vocab/LSP/describe/docs share truth;
- protocol schemas are generated/staleness-tested;
- unsupported constructs never silently become untyped;
- opt-in-off output stays byte-identical;
- ownership/sidecars remain intact;
- output is deterministic;
- fixtures contain no real credentials/customer data;
- both existing backends prove behavior unless the artifact is the new
  backend;
- external inputs are local/bounded/fingerprinted;
- CI growth stays inside checkpoint;
- live docs, not only this forecast, state limitations.

## Shared risks

- **Evidence favors public users.** Mitigation: local redacted receipts,
  direct interviews, minimized fixtures.
- **Loudest request looks largest.** Mitigation: independent-team floors,
  active-use definition, evidence grades, published score.
- **AB optimism.** Mitigation: sum both delivery/maintenance costs; count
  only demonstrated shared code.
- **Lossy contract temptation.** Mitigation: explicit subsets and hard
  diagnostics.
- **Public vendor specs can carry incompatible licenses or enormous
  fixtures.** Mitigation: retain minimized hand-written representative
  subsets unless redistribution is explicitly permitted.
- **`ciac import` can over-promise completeness.** Mitigation: command
  help and emitted comments say seeded/one-way/lossy, and the unmapped
  report is a required artifact.
- **Browser policy drift.** Mitigation: server-only authority; no client
  policy interpreter.
- **Admin requests can become an unbounded UI backlog.** Mitigation:
  requests beyond typed CRUD/forms route to the generated TypeScript
  client and OpenAPI as the supported custom-application path.
- **Third backend under-scoping.** Mitigation: all-example/full-provider
  bar and portable HIR repair.
- **Dependency churn.** Mitigation: cost steady-state ownership, pin one
  support line, reject candidates that do not fit.
- **Pressure to ship after failed spikes.** Mitigation: Track 0 is valid;
  a version number does not require a breadth feature.

## Explicitly not deployment maturity

No track adds:

- hosted CIaC;
- infrastructure application;
- cloud/Kubernetes maturity;
- API gateway;
- admin hosting/CDN;
- production identity management;
- secret distribution;
- release orchestration.

A TypeScript backend must match existing Docker/compose/verification
because that is backend completeness, not a new deployment initiative.

## Confidence and no v0.22 commitment

v0.21 ends with one evidence-backed reach decision, its bounded delivery,
and an honest record of what was not selected. Unselected candidates
return to an unranked backlog.

Confidence belongs to the evidence record, not to this forecast's current
lean. Track A, B, AB, C, and Track 0 are all legitimate checkpoint
outcomes; a runner-up does not continue as a stretch goal after selection.

This document assigns no v0.22 theme, sequence, or commitment.

The completed arc is coherent whichever track wins: v0.16 makes real
domains expressible; v0.17 verifies them instantly; v0.18 changes them
safely; v0.19 preserves meaning under failure; v0.20 maps failures back
to source; v0.21 lowers one evidence-selected adoption wall.

The through-line remains CIaC's moat: it owns the whole graph. Depth stays
the default use of that advantage, and breadth is spent deliberately and
recorded in public.
