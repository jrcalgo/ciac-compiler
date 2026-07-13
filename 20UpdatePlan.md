# CIaC v0.20 — Provenance: Source Maps for Infrastructure (roadmap forecast)

> Forecast document. Assumes v0.16–v0.19 have landed. References to
> simulation, semantic identities, outbox/idempotency effects, and
> ownership use the actual forms those versions ship; the v0.20 planning
> pass re-audits them before freezing this schema.
>
> This is a hypothesis release. Origin preservation inside the compiler is
> foundational; emitted maps and runtime instrumentation begin as one
> minimal opt-in vertical slice and earn full coverage only at a measured
> checkpoint.
>
> No deployment maturity. v0.20 adds no hosted trace backend, production
> collector, Kubernetes operator, Terraform observability stack, source-map
> registry, retention service, or operational control plane. `ciac trace`
> initially reads the compose Jaeger already generated for
> `tracing OpenTelemetry`.

## The gap this version closes

CIaC owns the source graph and generated project, but the relationship
between them largely disappears after code generation.

A route failure points to:

```text
app/api/submit.py:84
```

or:

```text
src/routes/submit.rs:117
```

It does not point to the pipeline step or handler expression that caused
the line to exist. A generated system test names a failed call or subject
but not the `.ciac` span that declared that edge. Jaeger shows framework
and worker spans, but no stable identity connecting them to a source
construct. An agent inspecting an owned file must reverse-engineer which
record, route, capability, or blueprint expansion produced it.

That is the final unclosed loop:

```text
spec → build → run → fail → generated target code → reverse-engineer
```

The desired loop stays at the source abstraction:

```text
spec → build → run → fail → order-system.ciac:42 → edit
```

The missing artifact is an infrastructure source map linking:

1. source declarations, statements, and expressions;
2. normalized graph nodes and edges;
3. generated routes, workers, jobs, clients, schemas, and wiring;
4. generated files and semantic regions;
5. compiler-owned runtime error, log, and span sites;
6. simulation/system-test assertions;
7. trace spans returned by Jaeger.

**v0.20 theme: every compiler-owned thing can answer “which `.ciac`
construct caused me to exist?”**

## The concrete starting point

The compiler already has fragments of the answer:

- `SourceMap` stores source text and line starts; `Span` is file plus
  byte range.
- services, graph nodes, pipelines, and steps retain optional spans
  in-process.
- those spans are `serde(skip)`, so graph JSON and codegen do not see
  them.
- graph edges have no span or cause list; duplicate edges merge and lose
  additional causes.
- records, fields, and tables do not retain source locations in IR.
- AST handler statements/expressions have spans, but typed HIR discards
  them.
- blueprint expansion mostly preserves definition spans while losing the
  expansion site and concrete instantiation.
- codegen contexts contain presentation names/strings but no source
  identity.
- `GeneratedFile` stores content and role, not mapped regions.
- OpenTelemetry propagation is real, but spans carry no stable CIaC
  construct/edge/site IDs. Existing trace tests therefore count spans
  rather than resolving them.

Transient `NodeId`/`EdgeId` values are not durable identities. Adding an
earlier declaration can renumber them; they cannot be written into
telemetry and called stable.

v0.19's “identity provenance” answers where an authenticated ownership
subject came from. v0.20 “source provenance” answers which source
construct caused generated/runtime behavior. They share execution
metadata where useful but are different guarantees and vocabularies.

## Product stance: minimal opt-in first

The public experiment is:

```sh
ciac build main.ciac --target python --out ./build --provenance
```

Without `--provenance`:

- generated runtime application artifacts remain byte-identical to the
  corresponding v0.19 build;
- no public provenance map/runtime table is emitted;
- no custom runtime attributes/wrappers are added.

Internal origin preservation lands unconditionally because simulation,
diagnostics, semantic diff, and future codegen need it. The emitted map
and runtime behavior are opt-in until evidence shows that failures lead
users directly to editable source.

Compiler intermediates, IR/golden snapshots, and internal protocol types
may change as origins are preserved. The external backend wire contract
changes only through its explicit provenance request/version path; the
no-provenance application tree does not churn merely because internal
origins exist.

One profile ships:

```text
--provenance
--provenance-paths relative|redacted
```

There is no premature `minimal|full|enterprise` profile family.

Minimal includes:

- source hashes/spans/origin chains;
- stable construct, edge, and runtime-site IDs;
- generated file/region maps;
- compact runtime metadata for errors/logs/traces;
- no source text, values, payloads, or credentials;
- no remote history or deployment integration.

## Pillar 1 — Five separate provenance concepts

### Source span

A source span remains:

```text
source file + start byte + end byte
```

Serialized maps include:

- zero-based half-open byte offsets;
- one-based line/column for display;
- explicit column encoding;
- no source text.

Byte offsets are authoritative when the source hash matches.

### Origin chain

One span is insufficient for expansion/desugaring. An interned origin
contains a primary span and ordered typed frames:

- `direct`;
- `expansion_site`;
- `blueprint_definition`;
- `parameter_argument`;
- `desugared_from`;
- `aggregate_member`;
- `inferred_from`.

For a blueprint-generated route:

```text
services/orders.ciac:14      expand Webhook<Order> ...
  via std/webhook.ciac:21    api Receiver
  via std/webhook.ciac:26    pipeline Receiver ...
```

The editable expansion site is normally primary; the definition remains
available as context. JSON carries the full chain; terse runtime output
shows:

```text
generated from services/orders.ciac:14
```

### Stable construct ID

A target-neutral `ConstructId` is separate from graph indices:

```text
ciac:v1:<kind>:<full-sha256>
```

The versioned canonical key excludes:

- physical/logical path;
- byte/line position;
- unrelated declaration order;
- target language;
- generated casing/path;
- compiler version.

Representative keys:

- system/service/component/capability: semantic owner + kind + name;
- record/table/field: semantic owner + name;
- pipeline: owner route/worker/job;
- route: owner plus operation identity;
- edge: kind plus stable endpoint IDs;
- runtime site: owner construct plus site kind/subkey.

Whitespace, comments, source movement, target switch, and unrelated
declaration insertion must not change IDs. Rename or service ownership
change intentionally does.

Anonymous pipeline/HIR steps use:

```text
owner + branch labels + operation kind + stable target
+ canonical syntax fingerprint + occurrence among identical siblings
```

Inserting a different sibling does not renumber later sites. Inserting a
truly identical sibling may affect equal-fingerprint occurrences; that
limit is documented rather than hidden.

The canonical semantic-key work from v0.18 is reused; v0.20 does not
invent a competing identity system.

### Runtime site ID

One construct can emit several observable sites. A route may have a
server span, auth log, handler invocation, call span, and error boundary.

The initial closed site kinds include:

```text
route.server
route.error
pipeline.step
handler.invoke
handler.fail
capability.call
service.call
broker.publish
broker.consume
worker.retry
worker.error
job.tick
job.error
channel.send
runtime.startup
runtime.health
runtime.log
```

Each `RuntimeSiteId` points to construct, origin, optional graph edge,
and later a generated region.

Third-party library internals are not assigned fake CIaC sites. They may
appear as unmapped framework children in traces.

### Generated region

A region is a byte/line range in one generated file linked to one or
more constructs, origins, edges, and runtime sites.

Regions may nest:

```text
file
└─ API router
   └─ route Submit
      ├─ Auth step
      ├─ handler invocation
      └─ call Billing.Charge
```

Lookup returns the smallest containing region plus parents. Every
non-metadata generated file receives a file-level origin; owned text has
semantic child regions where meaningful.

## Pillar 2 — Preserve origin through every compiler stage

### Logical versus physical source identity

`SourceFile::name` currently serves both filesystem access and display.
Split it into:

- physical path for loading/watching;
- logical reproducible path for maps/runtime/JSON;
- source SHA-256;
- in-memory text/line index;
- source identity.

Default root is the entry file's directory:

```text
main.ciac
services/checkout.ciac
```

Virtual sources retain stable identities:

```text
std/crud.ciac
registry:<owner>/<repo>/<path>.ciac@<ref>
```

Registry cache and checkout absolute paths never serialize.

Imports outside the root receive deterministic aliases, not absolute
paths.

Path policies:

- `relative`: normalized project-relative/virtual paths;
- `redacted`: stable `source/<digest>.ciac` aliases.

There is no absolute-path mode. Construct IDs do not depend on policy.

### AST and blueprint expansion

Introduce interned `OriginId` beside ordinary spans on every
code-generating boundary:

- declarations;
- fields/attributes;
- pipeline steps/arms;
- handler statements/expressions/predicates;
- blueprint/expand arguments.

Parser output creates a direct one-frame origin.

Blueprint expansion creates chains retaining:

- expansion site;
- body definition;
- argument span where substitution changed meaning.

Two expansions of one body therefore have distinct caller frames and
construct IDs even when they share definition spans.

Existing `.span()` access remains for diagnostics; provenance and
diagnostic presentation can choose different primary labels while
sharing the complete chain.

### Located HIR

Typed HIR must stop dropping exactly the sites runtime needs:

```rust
pub struct HirExpr {
    pub kind: HirExprKind,
    pub ty: HirType,
    pub origin: OriginId,
}

pub struct HirStmt {
    pub kind: HirStmtKind,
    pub origin: OriginId,
}
```

Exact internal shape may differ, but:

- every statement has origin;
- every effectful expression has origin;
- branches/arms retain their own;
- `fail`, `publish`, queries, capability verbs, records, returns, and
  v0.19 transaction/outbox/idempotency effects retain exact sites;
- synthesized HIR inherits a documented parent origin;
- origin is metadata, not part of type equality.

A traversal test fails whenever a new HIR variant has no explicit origin
handling.

### Graph/record/table completeness

Services, nodes, pipelines, steps, records, fields, tables, resources,
event streams, and v0.19 runtime-task semantics receive origins/stable
constructs.

Edges gain stable identity and multiple causes:

```text
Edge
  transient EdgeId
  stable ConstructId
  kind/endpoints
  causes[]
```

When graph deduplication merges equal edges, distinct source causes merge
deterministically rather than disappearing.

Examples:

- service-call edge → exact `call Service.Api` step;
- typed DB data-flow edge → all verb-call sites;
- stream→worker → worker `on` and/or expansion origin;
- CRUD edges → `crud` declaration with `desugared_from`;
- outbox relay runtime site → transactional `publish`, not a fictional
  source worker.

### Codegen contexts

`SystemModel` carries compact references, not repeated source chains:

```text
ProvenanceRef {
  construct_id,
  origin_id
}
```

Add them to:

- service/context roots;
- APIs and each generated route;
- CRUD operation routes;
- workers/jobs/channels/consumers;
- steps/arms/handlers/calls;
- records/fields/tables/resources;
- capability instances;
- generated simulation/system checks;
- v0.19 relays/idempotency/policy sites.

The arena is serialized once and shared.

## Pillar 3 — Versioned deterministic map

A provenance build emits:

```text
.ciac/provenance.json
```

Top-level shape:

```json
{
  "schema": "ciac.provenance",
  "schema_version": 1,
  "stable_id_version": 1,
  "build_id": "sha256:...",
  "compiler": {
    "version": "0.20.0",
    "target": "python"
  },
  "path_policy": "relative",
  "column_encoding": "utf8_bytes",
  "source_set_hash": "sha256:...",
  "sources": [],
  "origins": [],
  "constructs": [],
  "edges": [],
  "runtime_sites": [],
  "artifacts": []
}
```

This schema version is independent of CLI JSON, external backend
protocol, compiler version, semantic baseline, simulator replay, and
regeneration manifest versions.

### Sources/origins

A source entry contains logical path/hash/length, not text:

```json
{
  "id": "source:sha256:...",
  "path": "services/checkout.ciac",
  "sha256": "sha256:...",
  "byte_length": 2184
}
```

An origin references source IDs and typed frames. Labels are bounded
compiler summaries, not snippets.

### Constructs/edges/sites

A route:

```json
{
  "id": "ciac:v1:route:...",
  "kind": "route",
  "name": "Submit",
  "owner": "ciac:v1:api:...",
  "origin": "origin:sha256:...",
  "service": "Checkout",
  "attributes": {"method": "POST", "path": "/checkout"}
}
```

An edge:

```json
{
  "id": "ciac:v1:graph_edge:...",
  "kind": "service_call",
  "from": "ciac:v1:pipeline_step:...",
  "to": "ciac:v1:route:...",
  "causes": []
}
```

A site:

```json
{
  "id": "ciac:v1:runtime_site:...",
  "kind": "service.call",
  "construct": "ciac:v1:pipeline_step:...",
  "origin": "origin:sha256:...",
  "edge": "ciac:v1:graph_edge:..."
}
```

### Artifacts/regions

```json
{
  "path": "checkout/app/api/submit.py",
  "role": "owned",
  "sha256": "sha256:...",
  "origins": ["origin:sha256:..."],
  "regions": [
    {
      "kind": "route",
      "start_byte": 913,
      "end_byte": 1487,
      "start": {"line": 31, "column": 1},
      "end": {"line": 49, "column": 1},
      "constructs": ["ciac:v1:route:..."],
      "origins": ["origin:sha256:..."],
      "runtime_sites": ["ciac:v1:runtime_site:..."]
    }
  ]
}
```

Arrays sort deterministically:

- sources by logical path;
- origins by ID;
- constructs/edges/sites by stable ID;
- artifacts by path;
- regions by start/end/IDs.

No timestamp, host, process ID, temporary path, Jaeger URL, or random
value appears.

### Build identity

`build_id` hashes:

- schema/stable-ID versions;
- compiler version;
- target and generation-affecting options;
- path policy;
- sorted source identities/hashes.

It is not a hash of the map itself. Construct IDs remain stable across
compatible compiler versions; build ID intentionally changes.

A generated `docs/provenance-schema.json` is staleness-tested against the
Rust serializer.

## Pillar 4 — Generated file/region mapping

Public region recording, validation, and map emission in this pillar run
only when `--provenance` is enabled. Internal contexts may carry origin
references unconditionally, but the ordinary renderer emits the same
unmapped application bytes as v0.19.

`GeneratedFile` becomes conceptually:

```rust
GeneratedFile {
  content,
  role,
  origins,
  regions
}
```

### Map while rendering, never parse generated code

Post-hoc Python/Rust/YAML/JSON parsing is rejected. It would duplicate
target parsers and lose template intent.

Bundled rendering:

1. contexts carry IDs;
2. mapped helpers place internal boundary tokens around semantic blocks;
3. renderer removes tokens while computing final byte offsets;
4. line ranges resolve after removal;
5. no token reaches output;
6. HIR lowerers return mapped fragments for effect statements.

The marker mechanism is internal and replaceable; the public map is not.

### Coverage validator

For each owned text file:

- file-level origin exists;
- byte ranges are in bounds/UTF-8 boundaries;
- line ranges agree;
- nesting/overlap is valid;
- every referenced ID exists;
- every compiler-owned runtime site has a region;
- every emitted route has route construct/region;
- every graph edge has an origin;
- no internal marker remains.

Static imports/blank lines inherit file-level origin. “Complete” does
not mean a separate region for whitespace.

### Every existing generated artifact participates

When already emitted, map:

- backend source;
- OpenAPI;
- TypeScript client;
- migrations;
- compose/collector configuration;
- generated tests/system tests;
- Dockerfiles/manifests;
- generated CI;
- README/AGENTS;
- optional existing k8s/Terraform files.

Mapping those deployment artifacts adds no deployment behavior.

### Owned versus seeded honesty

Owned regions are authoritative only while on-disk hash matches the map.

For seeded files:

- original seed may have mapped boundaries;
- user edits mark interior as `seeded_drift`;
- generated invocation boundary still maps to handler declaration;
- user-authored line is never falsely attributed to DSL.

Runtime output may say:

```text
error while invoking handler ChargeCard
generated from services/billing.ciac:27
user-owned frame billing/app/services/charge_card.py:44
```

## Pillar 5 — Runtime errors, logs, and spans

Everything in this pillar is conditional on `--provenance`. A build
without the flag packages no runtime table, emits no CIaC provenance
attributes or semantic wrapper spans, adds no `generated from` context,
and preserves v0.19 stderr/log/span behavior exactly.

### Compact runtime table

Generated applications package only:

- build/schema versions;
- site → construct/kind;
- site → primary `path:line`;
- site → edge where relevant.

They do not package source text, full generated regions, or complete
blueprint chains.

Python loads one package resource once. Rust embeds one static table and
decodes through one-time initialization. Per-request map parsing is
forbidden.

### Runtime error contract

Every compiler-owned error boundary includes:

```text
generated from <logical-path>:<line>
```

Python preserves original exception/traceback and adds context. Rust
preserves source error in its context chain. Typed DSL errors keep their
public response shape.

Source paths are not added to public HTTP responses by default. They
appear in local stderr, structured logs, simulation/system reports, and
traces. Repository layout is not leaked to callers.

### OpenTelemetry attributes

Resource:

```text
ciac.provenance.schema_version
ciac.provenance.build_id
ciac.compiler.version
```

Compiler-owned spans/logs:

```text
ciac.construct.id
ciac.construct.kind
ciac.site.id
ciac.site.kind
ciac.source.path
ciac.source.line
ciac.source.column
ciac.origin.depth
ciac.edge.id
ciac.edge.kind
```

No source text, payload, record value, credential, user ID, token, SQL
argument, or absolute path is attached.

Low-cardinality semantic spans use stable names such as:

```text
ciac.route
ciac.step
ciac.handler
ciac.service_call
ciac.publish
ciac.consume
ciac.capability_call
ciac.job
```

Construct names/IDs are attributes, not dynamic span names.
Auto-instrumented framework spans remain as children. CIaC does not infer
their exact source when it did not emit/configure a site.

Templates may not add a compiler-owned log/span/error site without
registering a `RuntimeSiteId`; a generated-output test enforces that
registry.

## Pillar 6 — `ciac trace <id>`

### Command

```sh
ciac trace 4f6d6f8a0acbb9624d4dd3c9c78f6712 \
  --out ./build \
  [--jaeger http://localhost:16686] \
  [--map ./.ciac/provenance.json] \
  [--allow-stale] \
  [--json]
```

Rules:

- normalize 64/128-bit hex trace IDs;
- discover map/compose metadata from `--out`, or accept archived map;
- resolve Jaeger URL from flag, environment, then local compose default;
- never boot compose;
- read only;
- initial backend is the pinned compose Jaeger query API;
- no hosted-vendor auth/query abstraction in v0.20.

### Mapping

1. validate map;
2. fetch trace by exact ID;
3. normalize process/resource/span tags;
4. read trace `build_id`;
5. require matching map by default;
6. resolve site/construct/edge IDs;
7. order by parent/start;
8. render construct, origin chain, edge, duration, error;
9. show third-party spans as `framework/unmapped`;
10. warn on missing/contradictory IDs.

It never parses span names to guess identity.

Example:

```text
trace 4f6d...
└─ POST /checkout                         36 ms
   services/checkout.ciac:31  api Submit
   ├─ handler ValidateOrder               4 ms
   │  services/checkout.ciac:39
   ├─ call Billing.Charge                21 ms
   │  services/checkout.ciac:42
   │  edge service_call 8b29...
   └─ publish OrdersCreated               2 ms
      services/checkout.ciac:43
      └─ worker Fulfill
         services/fulfillment.ciac:19
```

JSON returns shared `ResolvedOrigin`, generated location, construct/site/
edge references, map status, and warnings through the current versioned
CLI envelope.

### Semantic trace tests

Upgrade current span-count continuity tests to assert:

- route site ID;
- exact call edge ID;
- publish/consume async edge IDs;
- one trace ID across hops;
- every compiler-owned span resolves through map;
- Python and Rust emit identical target-neutral IDs.

Trace discovery filters by route site ID, not “newest service trace.”

## Pillar 7 — Simulation/system failures

### Simulation

Simulation already runs inside compiler-owned plan/source context and
always reports origins, even without an emitted runtime map:

```text
simulation failed: Billing.Charge returned 503
generated from services/checkout.ciac:42
  pipeline Submit
  step call Billing.Charge
```

JSON uses the same construct/site/edge/origin types as trace output.
Blueprint failures include expansion and definition frames.

### Generated system tests

Every generated check carries:

- tested construct;
- edge where applicable;
- origin;
- generated test region;
- build ID.

Human assertion:

```text
system failure: expected message on orders.created
generated from services/orders.ciac:43
generated test tests/system/test_delivery.py:51
```

`verify --system --json` returns structured failures. A harness import or
dependency failure remains distinct and is not attributed to a random
DSL edge.

## Stale-map lifecycle

A confident wrong mapping is worse than none.

Manifest provenance metadata records:

```json
{
  "provenance": {
    "schema_version": 1,
    "build_id": "sha256:...",
    "path": ".ciac/provenance.json",
    "hash": "sha256:...",
    "runtime_hashes": {}
  }
}
```

Map/manifest update with successful regeneration:

- clean build installs files, map, runtime table, manifest coherently;
- conflict uses existing sidecars-only mode;
- active map/manifest remain untouched;
- candidate map may appear as `.ciac-new`;
- failed `ciac dev` retains last good map.

Statuses:

```text
current
source_stale
generated_stale
seeded_drift
trace_map_mismatch
schema_incompatible
missing
corrupt
```

Strict defaults:

- generated-line lookup refuses authoritative result on hash mismatch;
- trace refuses a different build map;
- IDs/trace-captured source attrs may still be shown as unverified.

`--allow-stale` may resolve a stable construct in the current map while
clearly separating captured versus current location. It never claims
generated regions current when hashes differ. Corrupt/incompatible maps
remain hard failures.

No automatic historical map database is created. CI/users may retain the
map as a build artifact and pass `--map`.

## CLI, MCP, graph, and protocol

### Direct lookup

```sh
ciac provenance --out ./build \
  --generated checkout/app/api/submit.py --line 84

ciac provenance --out ./build \
  --source services/checkout.ciac --line 42

ciac provenance --out ./build \
  --construct ciac:v1:route:...
```

Returns smallest/enclosing regions, ownership role, stale status, IDs,
and full origin chain.

### Build/diff/verify

`build`, `diff`, `dev`, and `verify` accept provenance/path-policy
options through shared generation settings.

Regeneration diff summarizes map churn by construct/artifact counts;
`--patch` may still show JSON text explicitly.

If manifest says a provenance map exists, ordinary verify validates it.
`verify --provenance` additionally requires the build to have one.

### Graph

Default graph output stays compatible. Opt-in:

```sh
ciac graph main.ciac --format json --provenance
ciac graph main.ciac --format dot --provenance
```

Shows stable IDs and origins, never transient indices as public identity.

### MCP

Add read-oriented tools:

- `provenance_lookup`;
- `trace`.

Existing build/diff/verify tools gain optional provenance args.
`trace` reads supplied local Jaeger/map state; it does not start Docker
or mutate the backend, preserving MCP's policy.

`describe` exposes schema/ID versions, path policies, construct/site
kinds, and telemetry keys.

### External backend protocol

A provenance-enabled request includes:

- stable construct/site/edge IDs;
- origins/source table;
- path policy;
- complete-region requirement.

Response files may return mapped regions. When provenance is requested:

- file-level origin is required;
- unknown/invalid IDs fail;
- incomplete owned-file coverage fails;
- compiler assembles shared artifact regions and validates final map.

Older external backends may remain usable without provenance according to
the protocol version active after v0.19. They cannot claim provenance
with whole-file guesses.

## Implementation map

### Diagnostics/syntax

- `ciac-diagnostics::source`: physical/logical paths, hashes, origin
  arena/frames, exported position metadata.
- Syntax AST/parser: origins on every code-generating boundary.
- Module loader: reproducible paths for local/std/registry.

### Sema/IR

- Blueprint chains and parameter origins.
- Located HIR.
- Stable construct keys from v0.18.
- Origins on records/fields/tables/resources/runtime effects.
- Edge causes and deterministic dedup.
- Provenance-aware graph output.

### Shared codegen

- Provenance refs in model.
- Mapped `GeneratedFile`/fragments/rendering.
- Schema/map serializer and coverage validator.
- Manifest/regen integration.
- Source-aware system tests.
- Mappings for OpenAPI/client/migration/CI/compose and existing optional
  deploy artifacts.
- Versioned external protocol.

### Python/Rust

- Route/worker/job/queue/client/error/observability/main templates;
- mapped HIR fragments;
- compact runtime table;
- error context preserving original causes;
- semantic spans/log attrs;
- route/CRUD/schema regions;
- explicit tests that provenance-disabled output has no churn.

### CLI/docs

- `trace` and `provenance` commands;
- shared lookup/Jaeger normalization;
- JSON origin/failure structures;
- MCP tools and describe vocabulary;
- new `docs/provenance.md`;
- trace attributes in operations/observability docs;
- blueprint/external backend/AGENTS guidance;
- checked-in provenance schema.

## Validation checkpoint

Before annotating every template, one multi-file, blueprint-expanded
Python slice contains:

- declared API and CRUD route;
- inline handler capability verb and `fail`;
- cross-service call;
- publish/consume;
- generated log/error;
- edited seeded handler;
- tracing.

It must answer the same source question through:

1. generated line;
2. graph edge;
3. runtime error/log site;
4. real Jaeger trace.

### Go criteria

- joins use IDs, not names/generated parsing;
- blueprint output shows expansion + definition;
- HIR effect resolves exact statement;
- source move/reformat preserves construct ID;
- two absolute checkouts produce byte-identical maps;
- no absolute path leaks;
- owned edit creates strict stale result;
- seeded edit is not falsely attributed;
- trace/map mismatch is caught first;
- provenance map/region generation adds no more than 20% to generation
  wall time and no more than 500 ms absolute on the checkpoint fixture;
- provenance-enabled runtime instrumentation adds no more than 2% to p95
  request latency in the local generated-service benchmark;
- the full map is no larger than 1.5 times the mapped generated text;
- the compact runtime table is no larger than 512 KiB or 5% of the
  application artifact, whichever allowance is larger.

`cargo bench -p ciac-codegen --bench provenance` measures map generation
and region validation. A checked-in `provenance_runtime_perf` integration
benchmark sends 10,000 requests through prebuilt Python and Rust
checkpoint services with provenance off/on and records p50/p95 plus
artifact sizes. Five measured runs after one warm-up must satisfy every
cap; raw benchmark JSON is retained as the release artifact.

### Kill/de-scope criteria

Do not ship runtime/trace claims if:

- HIR/blueprint origin requires post-lowering guessing;
- regions require parsing generated target code;
- stale maps can look authoritative;
- IDs change under formatting/path/unrelated reorder;
- path privacy requires absolute paths;
- Jaeger mapping depends on span names;
- determinism breaks;
- overhead remains above the published checkpoint;
- Python and Rust need incompatible identity semantics.

Fallback is honest: keep internal origins/stable IDs and, if sound,
experimental build-time maps; do not rename a partial implementation
“complete provenance.”

### Product adoption criterion

Dogfood source-aware diagnosis across at least:

- route;
- HIR effect;
- blueprint expansion;
- service call;
- broker delivery;
- owned/seeded drift.

Most seeded failures must lead directly to the editable `.ciac` site
without inspecting generated code before default-on work is considered.

## Verification strategy

- Path/position: CRLF, UTF-8, EOF, root relocation, redaction, no
  absolute/cache path leaks.
- Stable IDs: formatting, move, import reorder, unrelated insertion,
  target/path policy; intentional rename/service move changes.
- Blueprints: two expansions, args, hygiene, std/registry logical paths.
- HIR: every expression/statement/effect variant has origin.
- Graph: every entity/edge has origin; dedup merges causes.
- Schema/determinism: generated schema staleness, byte-identical map,
  valid references/ranges, no machine values.
- Regions: every golden example × target, every route/site, nested arms,
  mapped HIR, marker removal, shared artifacts.
- Regeneration: first/clean/change/conflict/seeded/orphan/failed dev/
  adoption/corrupt map.
- Runtime: exact message, original error cause, no public HTTP leak,
  log/span fields, one-time table load, disabled output unchanged.
- Simulation/system: structured origins and harness distinction.
- Jaeger: call, publish→worker, combined chain, exact ID lookup,
  framework children, stale/malformed/backend errors.
- External protocol: old non-provenance compatibility and strict mapped
  response conformance.

## Milestones

1. **M1 — Vocabulary/schema candidate:** re-audit v0.19, enumerate
   constructs/sites/artifacts, freeze path/ID/origin/schema candidates
   and perturbation tests.
2. **M2 — Origin-preserving front end/IR:** logical sources, origin arena,
   blueprint chains, located HIR, records/tables, stable IDs, edge causes.
3. **M3 — Mapped codegen framework:** model refs, mapped rendering,
   regions/validator, deterministic map, manifest integration, lookup.
4. **M4 — Python vertical slice/checkpoint:** route, HIR, call, broker,
   error/log/trace, seeded/stale cases; apply go/kill criteria.
5. **M5 — Complete Python coverage:** all routes/effects/artifacts/sites.
6. **M6 — Rust/cross-target parity:** mapped regions, errors, spans,
   identical target-neutral IDs.
7. **M7 — `ciac trace`:** Jaeger client, exact IDs, strict map join,
   rendering/JSON, semantic trace tests.
8. **M8 — Simulation/system failures:** structured source-aware results.
9. **M9 — External protocol/MCP/shared artifacts:** complete conformance.
10. **M10 — Privacy/determinism/performance/docs/v0.20.0:** leak/size/
    overhead suites, full verification, adoption recommendation.

## Explicit cuts

- Data lineage, row/field/value ancestry, payload history, or personal
  data flow.
- Source-level debugger, breakpoints, stepping, variable inspection.
- Supply-chain attestation, SBOM, SLSA, signatures, image provenance.
- Production tracing backend/collector deployment or retention.
- Default-on commitment.
- User-authored stable IDs in DSL.
- Automatic historical map archive.
- Universal trace-vendor abstraction.
- Source paths in public HTTP errors.
- New deployment maturity of any kind.

## Risks

- **Origin propagation is invasive.** Mitigation: intern chains, keep
  `Span` cheap, exclude origins from semantic equality, land IR first.
- **Stable IDs create expectations.** Mitigation: version key algorithm,
  exhaustive perturbation corpus, document anonymous duplicate limit.
- **Blueprint chains can confuse.** Mitigation: caller-oriented primary,
  typed frames, bounded human/full JSON.
- **Template annotation can become busywork.** Mitigation: mapped
  fragments/macros and checkpoint before broad rollout.
- **Whitespace control can shift ranges.** Mitigation: compute after
  marker removal and validate bytes/lines.
- **Map size can dominate output.** Mitigation: intern everything, no
  source text, size gates.
- **Runtime spans can alter latency/shape.** Mitigation: opt-in,
  low-cardinality constants, one-time table, benchmark gate.
- **Paths can leak.** Mitigation: no absolute mode, relative/redacted,
  automated leak scans.
- **Stale maps can mislead.** Mitigation: hashes/build IDs, strict joins,
  transactional regeneration, qualified stale mode.
- **Jaeger API varies.** Mitigation: support compose-pinned version first,
  fixtures, no premature vendor abstraction.
- **Auto-instrumented spans lack semantic identity.** Mitigation:
  explicit CIaC parents; do not guess.
- **The map may be correct but unused.** Mitigation: adoption/kill
  criteria; default-on must be earned.

## Confidence and v0.21 handoff

v0.20 is a named hypothesis. If it works, a generated route, graph edge,
simulation assertion, system test, runtime error, log, and trace span all
name one target-neutral construct and lead to the exact source origin
chain. If it does not, the valuable internal result still remains:
origins are no longer discarded before the places that need them.

v0.21 spends one deliberate breadth token only after usage evidence is
available. Provenance does not pre-commit that choice or turn deployment
maturity back into the roadmap.
