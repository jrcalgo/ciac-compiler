# CIaC v0.18 — Evolution: the Second Week of Every Project (roadmap forecast)

> Forecast document. Assumes v0.16 domain semantics and v0.17
> infrastructure-free simulation have landed. Direction-setting; the
> implementation planning pass freezes the semantic-model/baseline schema,
> compatibility matrix, and multi-file rename transaction before public
> tooling ships.
>
> “Semantic diff” always means comparison of two validated architecture
> models constructed from `NormalizedIr`. It never means a textual diff
> between `.ciac` files, and it never changes the existing meaning of
> manifest-aware generated-file `ciac diff`.
>
> This is an evolution-safety release, not deployment maturity. The
> generated CI workflow gains a compile-time compatibility gate; there are
> no canaries, rollout waves, automatic rollback, or production migration
> orchestration.

## The gap this version closes

CIaC gives the first day of a system unusually strong support: model,
check, build, simulate, verify. The second week is where ordinary changes
begin:

- a route is removed or gains a scope;
- a request field is retyped;
- a record is renamed across imported files;
- a stream subject changes;
- a provider or capability is replaced;
- generated clients, schemas, tests, and owned files follow;
- seeded handler or migration files may still mention the old name.

The live v0.15 machinery answers adjacent questions:

- `ciac check` says whether one current program is valid;
- `ciac diff -t <target> -o <dir>` compares a generated tree with its
  regeneration manifest and can show a unified file patch;
- `migrations.rs` refuses non-additive table evolution;
- `evolution.rs` protects a narrow set of boundary-record removals and
  retypes stored in the output manifest;
- structured fixes apply local unambiguous edits;
- LSP has diagnostics, hover, completion, and quick fixes;
- MCP exposes the existing machine tools (`check`, `build`, `diff`,
  `verify`, `graph`, `explain`, `describe`, and `fix`).

None answers the architectural question:

> Compared with the accepted system contract, which routes, records,
> capabilities, policies, and edges changed; which modeled consumers are
> affected; and is each change breaking, additive, or internal?

Nor can the compiler perform the most common safe refactor:

> Rename one resolved symbol throughout the local `.ciac` source set,
> regenerate compiler-owned output, preserve seeded files, and identify
> the seeded references a human must reconcile.

Generated-file churn is especially poor evidence. A one-line route change
may rewrite many artifacts; a comment/import-layout edit may rewrite none.
Reviewers and agents need the semantic fact, not a proxy based on output
bytes.

**v0.18 theme: make architecture change a typed, reviewable object and
make the common mechanical rename safe across the complete local source
set.**

## Four distinct artifacts

The version keeps four concepts separate.

### 1. Source

Local `.ciac` files, imported `std/`/registry files, and OpenAPI/spec
dependencies if later versions have added them. Source is parsed and
resolved normally.

### 2. Normalized semantic model

`NormalizedIr` remains the validated contract. A canonical
`SemanticModel::from_ir(&NormalizedIr)` projection removes transient
indices and backend presentation details while retaining architecture
meaning.

### 3. Checked-in semantic baseline

A target-independent, reviewable snapshot of a previously accepted
`SemanticModel`. It is source-control input, not output-tree state.

### 4. Regeneration manifest

`.ciac/manifest.json` remains target/output specific: file ownership and
hashes, migration sequence/state, record snapshots, and generation
recipe. It is not silently promoted into the architectural baseline.

A successful `build` never updates the semantic baseline. Otherwise the
change being checked could accept itself.

## Pillar 1 — `ciac diff --semantic`

### Two validated inputs

The direct comparison form compiles both programs:

```sh
ciac diff current/main.ciac \
  --semantic \
  --against accepted/main.ciac
```

Each side independently runs:

```text
load complete source set
→ parse
→ blueprint expansion
→ semantic passes
→ NormalizedIr
→ canonical SemanticModel
```

If either side is invalid, comparison fails with that side's ordinary
diagnostics. No text parser, generated tree, or backend participates.

For CI and long-lived review, the historical side may be a checked-in
baseline:

```sh
ciac diff architecture/main.ciac \
  --semantic \
  --baseline architecture/.ciac/baselines/main.semantic.json
```

That baseline is a serialized canonical model produced solely from an
earlier `NormalizedIr`. The comparator is therefore the same typed
model-to-model algorithm; the baseline avoids requiring an old checkout
and mutable registry dependencies in CI.

### Existing diff remains intact

This command keeps its current behavior:

```sh
ciac diff main.ciac --target python --out ./generated --patch
```

It reports `new`, `update`, `conflict`, `seeded-drift`, `orphan`, and
`orphan-delete` for generated files. Semantic mode conflicts with
`--target`, `--out`, and `--patch`; regeneration mode keeps requiring
them.

MCP also keeps the existing `diff` tool unchanged and adds a separate
`diff_semantic` tool.

### Baseline lifecycle

```sh
# Create.
ciac baseline architecture/main.ciac \
  --out architecture/.ciac/baselines/main.semantic.json

# Preview the pending replacement.
ciac baseline architecture/main.ciac \
  --out architecture/.ciac/baselines/main.semantic.json \
  --update

# Explicitly accept a replacement containing a breaking change.
ciac baseline architecture/main.ciac \
  --out architecture/.ciac/baselines/main.semantic.json \
  --update --accept-breaking
```

Rules:

- first creation succeeds;
- identical recreation is a byte-identical no-op;
- replacement requires `--update`;
- a breaking replacement additionally requires `--accept-breaking`;
- writes use a sibling temporary file and atomic replacement;
- no generated output or regeneration manifest is touched;
- baseline bytes are deterministic across checkouts;
- `build`, `verify`, simulation, rename, and generated CI never advance
  it automatically.

The default path, when no `--out`/`--baseline` is supplied, is a stable
entry-relative path under the source tree:

```text
<entry-directory>/.ciac/baselines/<entry-stem>.semantic.json
```

## Pillar 2 — Canonical semantic identities and schema

Raw `SystemGraph` JSON is not a durable baseline:

- `NodeId`, `ServiceId`, `RecordId`, and `TableId` are insertion
  indices;
- declaration reordering may renumber them;
- source spans are skipped;
- HIR local slots are compiler internals;
- serialization changes needed by codegen should not create fake
  architectural changes.

The canonical projection uses logical keys:

| Entity | Logical identity |
|--------|------------------|
| project | `project/<name>` |
| service | `service/<name>` |
| record/error | `record/<name>` |
| field | `record/<record>/field/<name>` |
| table | owning service/database plus table name |
| component | `service/<service>/<kind>/<name>` |
| capability | service + kind + instance |
| pipeline | identity of API/worker/job owner |
| route | service + declared API/resource operation |
| stream | declared stream identity; subject is a compared property |
| channel | service + channel identity |
| graph edge | kind + stable endpoint identities |

Name changes are not heuristically inferred. Without an explicit
persistent contract ID, a rename appears as removal plus addition. That
is conservative and reviewable; structural similarity does not prove
author intent.

The checked-in wrapper is independently versioned:

```json
{
  "semantic_baseline_version": 1,
  "semantic_model_version": 1,
  "compiler_version": "0.18.0",
  "entry": "architecture/main.ciac",
  "source_hash": "sha256:...",
  "semantic_hash": "sha256:...",
  "model": {
    "project": {},
    "services": [],
    "records": [],
    "tables": [],
    "routes": [],
    "streams": [],
    "channels": [],
    "capabilities": [],
    "pipelines": [],
    "handlers": []
  }
}
```

Audit metadata does not participate in `semantic_hash`. Formatting,
comments, source path movement, import order that preserves normalized
meaning, and equivalent blueprint expansion produce the same hash.

Typed values serialize as enums/objects, never Rust `Debug` strings.
Collections sort by logical identity.

A generated JSON Schema is checked in as
`docs/semantic-baseline-schema.json` and held byte-identical by a
staleness test. Readers reject unknown future incompatible versions;
they do not silently drop unknown variants.

## Pillar 3 — Typed changelists

### Change entry

Every change is machine-readable:

```json
{
  "id": "route.scope.changed:service/Billing/api/Charge",
  "kind": "route.scope.changed",
  "classification": "breaking",
  "symbol": {
    "kind": "api",
    "key": "service/Billing/api/Charge",
    "display": "Billing.Charge"
  },
  "before": {"scope": "payments:read"},
  "after": {"scope": "payments:write"},
  "consumers": [
    {
      "kind": "service_call",
      "service": "Checkout",
      "contract": "Billing.Charge"
    }
  ],
  "message": "Billing.Charge now requires payments:write; Checkout is a modeled caller."
}
```

Ordering is stable:

1. breaking;
2. additive;
3. internal;
4. kind;
5. symbol key.

The renderer may group nested field changes under a route/stream/table,
but JSON always retains each typed entry.

When one edit has different compatibility directions, the entry also
carries typed impacts:

```json
{
  "kind": "record.field.removed",
  "classification": "breaking",
  "impacts": [
    {
      "dimension": "request_acceptance",
      "classification": "additive"
    },
    {
      "dimension": "generated_client_source",
      "classification": "breaking"
    }
  ]
}
```

The top-level classification is always the maximum by the precedence
below. `--deny-breaking` and generated CI use that top-level value; the
impact list explains why a change that broadens server acceptance can
still break generated client source.

### Classifications

**Breaking** means an existing modeled consumer, public contract, or
retained state cannot be assumed to continue without coordinated change.

**Additive** means a compatible surface was added or an accepted contract
was broadened. It does not mean “operationally risk free.”

**Internal** means normalized architecture changed but no modeled
boundary was broken or expanded. It does not mean “safe to deploy.”

Severity precedence is:

```text
breaking > additive > internal
```

If one record participates in several boundaries, the most severe
classification wins while all consumers remain listed.

### Consumer-aware comparison

The current `evolution.rs` discovers callers of service calls and
consumers of cross-service streams. v0.18 generalizes that into typed
boundary uses:

- HTTP request/response;
- generated client operation;
- service-call request/response;
- stream/event payload;
- realtime channel payload;
- retained table state;
- external/unknown consumer where the model exposes a public route or
  subject.

Both old and new graphs matter:

- removals/restrictions name baseline consumers;
- additions name current consumers;
- removing the current edge does not erase the fact that an old consumer
  depended on it.

This corrects the narrow existing behavior where an old boundary can
disappear from the current graph before consumer reporting runs.

### Core classification matrix

#### Routes and auth

| Change | Classification |
|--------|----------------|
| add route | additive |
| remove route | breaking |
| method/path change | breaking |
| untyped request → typed request | breaking |
| typed request → untyped request | additive with validation-loss note |
| required request field added | breaking |
| request field removed | breaking top-level when generated client source changes; impacts record additive request acceptance separately |
| request/response field retyped | breaking |
| response field removed | breaking |
| response field added | additive unless rolling/nested requirements make old producer/new consumer incompatible |
| input enum value added | additive |
| input enum value removed | breaking |
| output enum value added | breaking for exhaustive consumers |
| output enum value removed | additive |
| auth/scope added or tightened | breaking |
| auth/scope removed or relaxed | additive with security-relaxation note |
| auth scheme changed | breaking |

CIaC currently often uses one payload record as both request and response
shape. In that case the stricter directional result applies.

#### Streams and channels

| Change | Classification |
|--------|----------------|
| add stream/channel | additive |
| remove used stream/channel | breaking |
| remove unused internal stream | internal |
| change subject/path/provider | breaking |
| change payload/reference cardinality/validation incompatibly | breaking |
| add producer | additive |
| remove last producer while consumers remain | breaking |
| add consumer | additive |
| remove private consumer | internal |

#### Records, relations, constraints, and validation

- Public/cross-service usage receives directional boundary rules.
- Table-backed usage additionally receives persistence rules.
- A private record change is internal.
- Field order is non-semantic.
- Reference target/cardinality changes are breaking.
- Validation tightening is breaking for accepted input.
- Validation relaxation is additive for accepted input.
- Unique/FK/index changes carry both compatibility and migration notes.

#### Persistence

| Change | Classification |
|--------|----------------|
| add table | additive |
| safe additive column/index/link table | additive with migration note |
| remove table/column/constraint | breaking |
| retype column/reference | breaking |
| rename table/column | remove+add; breaking without alias semantics |
| internal migration sequence/file change only | not a semantic change |

Semantic diff reports meaning. `migrations.rs` remains authoritative for
whether concrete SQL can be generated; semantic diff does not create or
approve a migration.

#### Services, capabilities, pipelines, handlers

- adding a service is additive;
- removing a service with public/consumer surface is breaking;
- provider or capability changes are internal unless they alter a public
  auth/contract guarantee;
- concurrency, retries, schedule, and implementation digest are internal
  with operational notes;
- adding/removing a publish or call edge is classified from old/new
  consumers;
- inline handler bodies compare through a canonical typed structural
  digest, reported as internal. v0.18 does not claim behavioral
  equivalence.

### Cascade suppression

If an entire API is removed, report the route removal and affected
consumers; do not also flood the report with every now-unreachable field.
Nested changes remain visible when the parent contract survives.

### Exhaustiveness across later versions

The semantic model and differ are a maintained compiler surface, not a
one-release snapshot. Adding a later declaration, attribute, provider
guarantee, policy, or runtime semantic is incomplete until it has:

- a canonical representation;
- a before/after comparator;
- a classification rule;
- consumer extraction where relevant;
- baseline-version compatibility tests.

In particular, v0.19 must extend this matrix for transactional versus
direct publish, idempotency, ownership, ordering/fan-out budgets, lint
policy, and compiler-owned correctness tables. Unknown semantic fields
cannot be silently ignored merely because the v0.18 reader predates them.

## Pillar 4 — Breaking-change gates in generated CI

`ciac build --deploy ci` gains explicit baseline configuration:

```sh
ciac build architecture/main.ciac \
  --target python \
  --out . \
  --deploy ci \
  --semantic-baseline architecture/.ciac/baselines/main.semantic.json
```

The source entry and baseline must resolve beneath the generated
workflow's repository root. Generated workflow paths are normalized and
relative; developer absolute paths never enter YAML.

The workflow adds:

```text
semantic-compat
      │
      ▼
     test
    /    \
build-image  compose-smoke
```

The job:

- installs the exact CIaC release recorded by generation, never
  “latest”;
- runs `ciac diff --semantic --deny-breaking --json`;
- writes/uploads the complete changelist even on failure;
- appends a human job summary;
- fails on missing/corrupt baseline, compile diagnostics, or breaking
  changes;
- passes additive/internal changes while keeping them visible;
- never updates or commits the baseline.

Image work cannot proceed after a failed compatibility gate.

This is a source-contract check. It does not inspect a live environment,
sequence a rollout, or guarantee an operationally safe deploy.

## Pillar 5 — Mechanical rename

### CLI

Rename is position-based and dry-run by default:

```sh
ciac rename architecture/main.ciac \
  --file architecture/records/order.ciac \
  --line 3 --column 8 \
  --to PurchaseOrder
```

Apply explicitly:

```sh
ciac rename architecture/main.ciac \
  --file architecture/records/order.ciac \
  --line 3 --column 8 \
  --to PurchaseOrder \
  --apply
```

Known generated outputs may participate:

```sh
ciac rename architecture/main.ciac \
  --file architecture/records/order.ciac \
  --line 3 --column 8 \
  --to PurchaseOrder \
  --out generated-python \
  --out generated-rust \
  --apply
```

An optional semantic baseline adds a post-rename preview. It is never
updated by rename.

### Symbol resolution, not text replacement

The first supported set includes:

- project/service;
- record/error, field, enum variant;
- table;
- stream;
- API, worker, job, channel, CRUD/events declaration;
- explicit/implicit handler and pipeline owner;
- capability instance;
- local blueprint, type/scalar parameter, and body declaration;
- handler parameter and lexical `let`.

It rejects:

- keyword, builtin step, primitive, capability kind, provider;
- import path or arbitrary string;
- embedded `std/`/immutable registry definition;
- one selected hygienic expansion with no unique source token.

Namespaces mirror sema. A record and table with the same spelling remain
distinct; service-local names resolve within their service; lexical locals
never rewrite a field with the same text.

Before a plan is offered, the new identifier must:

- satisfy identifier syntax;
- avoid reserved words;
- avoid target-namespace collisions;
- leave the complete edited program semantically valid.

The final authority is a full in-memory compile of the edited source set.

### References rewritten

The resolved index covers:

- record uses in APIs, streams, tables, CRUD, handlers, and blueprints;
- field access, construction, update, predicates, and match positions;
- stream publish/worker/channel references;
- pipeline owner/handler steps;
- qualified `call Service.Api`;
- table targets in every DB verb;
- capability bindings;
- blueprint declarations/expansions/body-local references;
- lexical handler locals.

Comments and unrelated string literals are untouched.

### Imports and blueprint provenance

The current loader textually splices imports and discards useful module
editing metadata. v0.18 returns:

```text
LoadedProgram {
  program,
  sources,
  module_graph,
  source_origins
}
```

Sources are local-editable, embedded-read-only, or registry-read-only.
Diamond imports edit one physical file once.

Rename indexing occurs before blueprint expansion erases source-level
identity; final validation occurs after normal expansion.

Blueprint rules:

- rename a local blueprint declaration and all local expand sites;
- rename one body declaration and all body-local references, changing
  every expansion together;
- rename type arguments/parameters at their source occurrences;
- reject selective rename of a derived generated name with no source
  token;
- never edit embedded/registry source.

### Dry-run plan

Dry-run performs all expensive checks:

1. load/resolve source set;
2. resolve selected symbol;
3. compute source edits;
4. apply to an in-memory overlay;
5. parse/expand/analyze edited program;
6. compute optional semantic preview;
7. reconstruct requested generated outputs;
8. compute regeneration plans/conflicts;
9. scan seeded files for old source/generated spellings;
10. return deterministic source patches, generated plan, and warnings.

It writes nothing.

The result contains a `plan_id` hashing source/output preconditions.
Preview→apply can require that ID to detect stale state.

### Multi-file apply transaction

Ordinary filesystems cannot make many replacements literally
crash-atomic. The promise is:

- stale-input detection before first write;
- all-or-none behavior for detected errors;
- staged sibling files;
- rollback backups;
- a small recovery journal for interruption.

Apply:

1. recomputes/verifies plan and hashes;
2. rejects every generated conflict before writing;
3. stages local source, safe owned updates, new files, sidecars, and
   manifests;
4. writes sources/owned output;
5. deletes only safe untouched owned orphans;
6. writes manifests last;
7. rechecks source;
8. removes journal/backups.

No source edit is committed when a requested output cannot regenerate
safely.

## Pillar 6 — Generated ownership and seeded references

Rename must replay the exact generation recipe. The current manifest
records a target but not every generation-affecting option. A versioned
manifest therefore records:

```json
{
  "manifest_version": 2,
  "recipe": {
    "entry": "architecture/main.ciac",
    "target": "python",
    "name": null,
    "deploy": ["ci"],
    "profile": "dev",
    "secrets": false,
    "image_prefix": null,
    "image_tag": "latest",
    "clients": ["ts"],
    "simulation": true,
    "semantic_baseline": "architecture/.ciac/baselines/main.semantic.json"
  }
}
```

Legacy manifests remain readable. Rename can edit source without an
output, but refuses `--out` against a legacy recipe rather than guessing.
A normal fresh build upgrades the recipe.

Existing seeded files are never rewritten, moved, or deleted by rename.
For each requested output, the engine reports possible old references:

- path, line, column;
- matched old source/generated spelling;
- old seeded path becoming orphan;
- new seeded path receiving a fresh seed;
- migration SQL containing renamed table/column text.

This is deliberately labeled `possible_reference`: CIaC does not parse
arbitrary Python, Rust, TypeScript, SQL, or external-backend code in
v0.18.

The operation may succeed with
`manual_reconciliation_required: true`; preserving user work is more
important than pretending it was ported.

## Pillar 7 — JSON, MCP, and LSP

### JSON

The envelope advances from the version current after v0.17 and uses a
tagged result:

```json
{
  "command": "diff",
  "success": false,
  "diagnostics": [],
  "tool_error": null,
  "result": {
    "kind": "semantic-diff",
    "semantic_diff_version": 1,
    "policy": {"deny_breaking": true, "passed": false},
    "summary": {"breaking": 1, "additive": 2, "internal": 3},
    "changes": []
  }
}
```

Regeneration diff uses `kind: "regeneration-diff"`.

Rules:

- report-only semantic diff succeeds even when it reports breaking
  entries;
- `--deny-breaking` makes policy failure set `success: false`;
- a policy failure still contains the complete valid result;
- compile errors remain diagnostics;
- baseline/rename transaction failures use stable tool-error kinds;
- a valid breaking change is not a `CIAC00xx` source diagnostic.

### MCP

Add:

```text
diff_semantic
rename
```

`diff_semantic` accepts optional `deny_breaking: true` and returns the
same changelist and `policy.passed` semantics as CLI. A breaking policy
result is valid tool data rather than an MCP protocol error. `rename`
previews by default; `apply: true` is required for writes and may include
`plan_id`. The existing `diff` remains regeneration-only.

### LSP rename

The language server advertises:

```text
prepareRenameProvider = true
renameProvider = true
```

`prepareRename` resolves/rejects the symbol and returns its exact range.
`textDocument/rename` returns a versioned multi-document
`WorkspaceEdit` across local imported files.

The LSP does not:

- write files itself;
- regenerate outputs;
- edit manifests;
- update baselines;
- edit seeded host-language code.

The current server validates from disk and only keeps changed text for
hover/completion. Rename requires an overlay VFS:

- open buffers override disk;
- unopened imports read disk;
- document versions protect edits;
- byte positions convert correctly to UTF-16 LSP columns.

Diagnostics may remain save-driven for this version; rename itself must
honor unsaved source.

## Implementation map

### Semantic model and evolution

- Add canonical model/key types near `ciac-ir` or `ciac-codegen`
  evolution, with no backend strings.
- Generalize boundary-consumer discovery in `evolution.rs`.
- Keep legacy manifest record checks as a compatibility adapter.
- Keep migration SQL generation separate.
- Add baseline serializer/schema/differ and complete classification
  matrix.

### Source and rename

- `ciac-syntax::module`: module graph, source origin, overlay provider.
- `ciac-diagnostics::source`: canonical paths and byte/UTF-16 positions.
- `ciac-sema`: resolved definition/reference index and blueprint
  provenance.
- `regen.rs`: staging/preflight/rollback primitives.
- `manifest.rs`: versioned complete generation recipe.

### CLI/codegen

- `main.rs`: `baseline`, `rename`, disjoint semantic-diff args.
- `commands.rs`: baseline/diff/rename internals.
- `json_out.rs`: tagged result/tool errors.
- `ci.rs`: semantic compatibility job.
- `mcp.rs`: two tools.
- `lsp.rs`: VFS, prepare/rename, multi-file edits.
- generated `AGENTS.md`: explain baseline versus manifest and seeded
  reconciliation.

### Documentation

- New `docs/evolution.md`.
- Update regeneration, deployment/CI, agents/MCP, authoring/LSP, and
  errors where applicable.
- Check in semantic baseline JSON Schema.

## Verification strategy

### Semantic invariance

- declaration/import reorder with same IR meaning yields same hash;
- formatting/comments/path movement yield no changes;
- transient IR indices do not leak;
- field types are typed values, never debug strings;
- change order is deterministic;
- old/current consumer direction is correct;
- parent removal suppresses redundant child churn.

### Classification corpus

Fixtures cover routes, method/path, auth/scope, request/response fields,
enums, streams/subjects, channels, relations/validation, constraints,
tables/columns, capabilities/providers, retries/schedules, handlers, and
pipeline edges. Tests assert kind/classification/key/consumer—not prose
alone.

### Baseline/CLI/CI

- deterministic create/no-op/update/accept-breaking;
- future schema refusal;
- semantic versus regeneration argument separation;
- report-only versus deny-breaking exits;
- single JSON document;
- generated workflow pins version, gates downstream jobs, uploads result,
  and never writes baseline.

### Rename

- multi-file record rename;
- diamond import edits once;
- duplicate service-local names;
- same spelling in different namespaces;
- API/pipeline/call, worker/job/pipeline, stream/publish/on/channel,
  table/verb, field/access/constructor/predicate/match;
- lexical locals;
- comments/strings untouched;
- invalid/collision/read-only rejection;
- blueprint body/argument/derived-name cases;
- dry-run no writes;
- stale/conflict abort;
- injected apply failure rollback/recovery;
- exact generation recipe;
- seeded files byte-identical and references reported.

### MCP/LSP

- tool schemas/results match CLI;
- preview/apply safety and stale IDs;
- prepare rename;
- edits across multiple URIs;
- unsaved imports;
- UTF-16 columns;
- server performs no direct writes.

## Milestones

1. **M1 — Canonical model and baseline:** stable keys, typed projection,
   deterministic hash/schema, baseline lifecycle.
2. **M2 — Consumer-aware differ:** old/current boundary graph,
   classification matrix, cascade suppression, human/JSON rendering.
3. **M3 — CLI and generated CI gate:** `diff --semantic`,
   `--deny-breaking`, tagged JSON, pinned workflow job/artifact.
4. **M4 — Source index and rename engine:** namespaces, definitions,
   references, modules, blueprints, overlay validation.
5. **M5 — Transactional apply and regeneration replay:** manifest recipe,
   staging/journal/rollback, repeated output roots, seeded scanner.
6. **M6 — MCP and LSP:** `diff_semantic`, rename preview/apply,
   prepare/rename, multi-file versioned edits.
7. **M7 — Reconciliation and v0.18.0:** schemas, docs, examples,
   goldens, full compiler/generated/system verification, whole-version
   analysis.

## Explicit cuts

- Replacing or renaming current regeneration diff.
- Text comparison for semantic conclusions.
- Automatic baseline updates.
- Heuristic rename inference.
- Import/file/directory rename.
- Rewriting seeded Python/Rust/TypeScript/SQL.
- Porting extern implementations.
- Generating destructive migrations.
- Behavioral equivalence of handler bodies.
- Cross-repository/registry rename.
- Editing `std/` or registry cache.
- Selective rename of one expansion with no source identity.
- General find-references UI.
- Deployment waves, canaries, rollback, or live rollout planning.
- Claiming `internal` means operationally safe.

## Risks

- **Compatibility rules can over-promise.** Mitigation: name modeled
  consumers, represent unknown external consumers, use conservative
  rolling-boundary rules, and state that behavior is not proven.
- **Current IR lacks persistent user IDs.** Mitigation: no heuristic
  rename inference; remove+add stays visible.
- **Blueprint provenance is subtle.** Mitigation: index before expansion,
  validate after, and reject source-less selective renames.
- **Multi-file writes are not physically atomic.** Mitigation: stage,
  hash-check, journal, rollback, and document the exact guarantee.
- **Legacy manifests lack recipes.** Mitigation: require a fresh build,
  never guess.
- **Seeded scanning can be incomplete/noisy.** Mitigation: report
  possible references, never rewrite, and show path changes separately.
- **LSP VFS increases infrastructure.** Mitigation: one source-provider
  abstraction shared with CLI tests and scope it to rename correctness.
- **Generated CI must install a reproducible compiler.** Mitigation:
  exact-version pin and tested setup; do not turn this into delivery
  platform work.
- **Semantic diff and rename are each substantial.** Mitigation: land
  semantic comparison first; both MCP/LSP/CLI paths reuse one rename
  engine.

## Confidence and v0.19 handoff

Semantic diff is a confirmed pillar: the current record-evolution
machinery already proves the pattern, and the existing file diff is
demonstrably the wrong abstraction for contract review. Baseline gates
and resolved rename are low-risk extensions only if identities,
multi-file writes, and seeded ownership remain explicit.

After v0.18, CIaC can say what changed and where manual work remains.
v0.19 addresses the next, different question: whether the generated
system remains correct when a process crashes between effects, a broker
redelivers, a client repeats a request, or one authenticated subject
tries to access another subject's row.
