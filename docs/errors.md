# Error Code Index

*Reader: anyone who hit a `CIAC####` code and wants its meaning.*

Stable diagnostics emitted by the compiler. Codes are append-only: once
published, a code's meaning never changes. `ciac explain <code>` prints
the same explanations; this page is checked against the registry by a
test.

Some mechanical, unambiguous diagnostics also carry an applyable fix
(v0.15 M7, widened at `29UpdatePlan.md` M8) — `--json`'s `fixes`
field, `ciac lsp`'s quick-fix, `ciac mcp`'s `fix` tool. Today:
`CIAC0005` (missing capability — inserts the right `capability
Provider;` line into an existing `use { .. }` block), `CIAC0013`
(unknown provider — a nearest-match rename), `CIAC0025`'s `auth
OAuth2` missing-`issuer` case (inserts a placeholder), `CIAC0041`
(unknown record field — a nearest-match rename), `CIAC0017` (unknown
stream — a nearest-match rename against declared streams), `CIAC0022`
(unknown capability instance — a nearest-match rename against the
service's own declared instances of that capability kind), and
`CIAC0042` (unknown table — a nearest-match rename against declared
tables). Every nearest-match fix uses the same bar: offered only when
one known name is close enough (Levenshtein distance <= 3, or <= 5 for
the two-word `capability Provider` form CIAC0013 compares) to be a
plausible typo rather than a coincidence — silence, not a wrong
guess, otherwise. See `docs/agents.md`.

| Code | Severity | Title |
|------|----------|-------|
| CIAC0001 | error | invalid token |
| CIAC0002 | error | unexpected token |
| CIAC0003 | error | duplicate declaration |
| CIAC0004 | error | pipeline has no matching component |
| CIAC0005 | error | missing capability for construct |
| CIAC0006 | error | cyclic dependency |
| CIAC0007 | warning | unreachable component |
| CIAC0008 | error | invalid auth placement |
| CIAC0009 | error | incompatible composition |
| CIAC0010 | error | missing service declaration |
| CIAC0011 | error | construct not supported by backend |
| CIAC0012 | error | duplicate capability |
| CIAC0013 | error | unknown provider |
| CIAC0014 | error | empty pipeline |
| CIAC0015 | error | unknown type |
| CIAC0016 | error | payload type mismatch |
| CIAC0017 | error | unknown stream |
| CIAC0018 | error | unknown attribute |
| CIAC0019 | error | invalid attribute value |
| CIAC0020 | error | invalid match |
| CIAC0021 | error | non-exhaustive match |
| CIAC0022 | error | unknown capability instance |
| CIAC0023 | error | ambiguous capability binding |
| CIAC0024 | error | invalid handler binding |
| CIAC0025 | error | unsupported provider configuration |
| CIAC0026 | error | duplicate service |
| CIAC0027 | error | unknown service |
| CIAC0028 | error | unknown service member |
| CIAC0029 | error | cross-service payload mismatch |
| CIAC0030 | error | invalid service scope |
| CIAC0031 | error | invalid shared stream topology |
| CIAC0032 | error | invalid call |
| CIAC0033 | error | regeneration conflict |
| CIAC0034 | warning | seeded file drifted |
| CIAC0035 | warning | orphaned generated file |
| CIAC0036 | error | output directory has no manifest |
| CIAC0037 | error | invalid cron expression |
| CIAC0038 | error | inline handler bodies are not implemented yet |
| CIAC0039 | error | unknown name in handler body |
| CIAC0040 | error | type mismatch in handler body |
| CIAC0041 | error | unknown record field |
| CIAC0042 | error | unknown table |
| CIAC0043 | error | invalid capability verb call |
| CIAC0044 | error | verb on unbound capability |
| CIAC0045 | warning | unused let binding |
| CIAC0046 | error | unsupported schema change |
| CIAC0047 | error | import cycle |
| CIAC0048 | error | unknown blueprint |
| CIAC0049 | error | blueprint arity mismatch |
| CIAC0050 | error | blueprint constraint violation |
| CIAC0051 | error | breaking record change |
| CIAC0052 | error | `where` clause on a non-query verb |
| CIAC0053 | error | unsupported field type |
| CIAC0054 | error | unknown reference target |
| CIAC0055 | error | invalid reference definition |
| CIAC0056 | error | cross-storage reference |
| CIAC0057 | error | cyclic reference graph |
| CIAC0058 | error | reference requires typed storage |
| CIAC0059 | error | invalid storage constraint |
| CIAC0060 | error | invalid field validation |
| CIAC0061 | error | invalid transaction block |
| CIAC0062 | error | non-transactional effect inside a transaction |

### Reserved

`26UpdatePlan.md` M8 reserves `CIAC0063`–`CIAC0072` for the language
deprecation ladder (`docs/language.md`'s `## Stability and
versioning`) — step 2 of that ladder (a compiler warning on every use
of a deprecated-but-still-accepted construct) will draw from this
range. Empty today: no construct has been deprecated yet, so no code
in this range is emitted by any diagnostic, and none appears in
`ErrorCode::ALL`. The range is documented here, ahead of first use, so
a future deprecation warning's code number is picked from a pre-agreed
block rather than improvised at the point of need.

## Notes

- **CIAC0005** covers every capability requirement: `Auth` steps need
  `auth`, `Queue` steps and worker pipelines need `queue`, `crud` needs
  `db`, `events` needs `queue`.
- **CIAC0006** includes asynchronous cycles, e.g. a worker publishing to
  the queue it consumes from.
- **CIAC0007** is a warning: compilation succeeds, but the component is
  dead weight — wire it into a pipeline or remove it.
- **CIAC0011** is reported at `ciac build` time when the selected
  backend cannot implement a declared component (e.g. `queue Kafka`
  with the bundled backends); choose another provider or target.
- **CIAC0015** covers undeclared records referenced by `stream`/`api`/
  `crud` declarations and unknown field types inside records.
- **CIAC0016** is the publish-site type check: the pipeline's payload
  type (its api's request record, or the consumed stream's record for
  workers) must match the record of every stream it publishes to.
- **CIAC0017** means `publish X` or `worker .. on X` references a
  stream that no `stream X: <Record>;` declares.
- **CIAC0018** means an attribute is not supported for that declaration
  kind; attributes are a closed registry, not free-form metadata.
- **CIAC0019** covers wrong attribute value types, out-of-range numeric
  values, and attribute preconditions such as scoped apis without an
  `Auth` gate or `cache_ttl` without `cache`.
- **CIAC0020** covers invalid `match` usage: untyped/non-enum fields,
  unknown variants, nested matches, wildcard placement, and non-terminal
  top-level matches.
- **CIAC0021** means a `match` over an enum omits one or more variants
  without a trailing `_` wildcard.
- **CIAC0022** means a handler binding references a named capability
  instance that no `use` entry declares.
- **CIAC0023** means CIaC needs a default capability instance but several
  exist and none is named `default`; add a handler binding or declare the
  default instance explicitly.
- **CIAC0024** means a `handler` declaration binds an unsupported
  capability kind.
- **CIAC0025** means a provider-specific config is missing or unsupported,
  such as `external_http` without `base_url`.
- **CIAC0026** means a project declares the same `service` block name
  more than once.
- **CIAC0027** means a `call Service.Api` target names an unknown service.
- **CIAC0028** means the target service exists, but the named API does not.
- **CIAC0029** means the caller pipeline payload does not match the target
  API request record.
- **CIAC0030** means a project mixes `service { ... }` blocks with flat
  service-local declarations.
- **CIAC0031** is reserved for invalid shared-stream topologies.
- **CIAC0032** means a `call` target is malformed, e.g. not `Service.Api`.
- **CIAC0033** means a compiler-owned file was edited after the last
  build; CIaC wrote the newly generated content to a `.ciac-new` sidecar.
- **CIAC0034** means a user-owned seeded file exists but the generated
  seed changed; reconcile the `.ciac-new` sidecar manually.
- **CIAC0035** means a previously generated file is no longer produced.
- **CIAC0036** means a non-empty output directory has no regeneration
  manifest; use a clean directory, `--force`, or `--adopt`.
- **CIAC0037** means a job schedule is not a valid five-field cron
  expression.
- **CIAC0038** was v0.7 M1's blanket "not implemented yet" gate for every
  typed-signature or inline-body handler. Superseded by real type
  checking as of M2 (CIAC0039-CIAC0045) — a handler that type-checks now
  passes `ciac check`; this code no longer triggers for well-formed
  programs, but stays published per the append-only rule.
- **CIAC0039** means an identifier in a handler body doesn't resolve to a
  parameter or an in-scope `let` binding.
- **CIAC0040** means an expression's type doesn't match what's required:
  mismatched operator operands, disagreeing `if`/`match` branches, a
  `return` that doesn't match the declared return type, or a wrongly
  typed record field value.
- **CIAC0041** means a field access, record construction, or functional
  update names a field the record doesn't have.
- **CIAC0042** means a `db.*` verb's table argument isn't a declared
  `table <Name>: <Record>;`.
- **CIAC0043** means a capability verb call isn't in that capability's
  closed verb set, or is called with the wrong arity/argument types.
- **CIAC0044** means a verb is called on a capability kind with no bound
  instance in the enclosing service; add it to the `use { .. }` block.
- **CIAC0045** means a `let` binding is never read; remove it.
- **CIAC0046** means a `table`'s schema changed in a way the additive-only
  migration differ can't express safely — a column was removed or
  retyped, or the whole table was removed. Write a manual migration file
  for the drop/retype, then rerun `ciac build`.
- **CIAC0047** means a chain of `import "path";` declarations forms a
  cycle; move the shared declarations into a third file both sides
  import instead.
- **CIAC0048** means `expand` names a blueprint that isn't declared (or
  isn't imported); check the spelling and `import`s.
- **CIAC0049** means an `expand` site's params don't match the
  blueprint's declared `params { .. }` block — missing, unknown, or
  wrong-typed.
- **CIAC0050** means `expand`'s type argument isn't a declared
  `record`; every v0.8 blueprint's type parameter is constrained to
  `record`.
- **CIAC0051** means a record used across a service boundary (a `call`
  payload, or a stream published in one service and consumed in
  another) had a field removed or retyped since the last build — the
  two services redeploy independently, so this would break wire
  compatibility. Adding a field is always fine; revert the removal/
  retype, or coordinate the change across every consumer named in the
  error before rebuilding.
- **CIAC0052** means a `where <predicate>` clause was attached to a
  capability verb call that doesn't accept one — only `db.query`,
  `db.count`, and `db.delete_where` do (v0.14 M1).
- **CIAC0053** means a `[Type]` list type was used somewhere the
  compiler doesn't yet support it: valid as a handler parameter/return
  type, or as the value of `db.query`/`object_store.list`/
  `search.query`, but not yet as a `record` field type (v0.14 M1). It
  is also used, temporarily, for a `Reference<T>` field: the surface
  grammar is frozen (v0.16 M1) but relation resolution is still being
  implemented.
- **CIAC0054** means a `Reference<T>` field named a `T` that isn't a
  declared record, or whose `references:` attribute doesn't name a
  `table`/typed `crud` backed by that record (v0.16).
- **CIAC0055** means a `Reference<T>` field is missing a required
  attribute (`references`, `cardinality`, `on_delete`, `on_update`),
  has an invalid value for one of them, or combines `unique: true`
  with `cardinality: many` (v0.16). Every reference states its
  cardinality and both referential actions explicitly — there is no
  default cascade.
- **CIAC0056** means a `Reference<T>` field's source and target
  belong to different services or different database capability
  instances (v0.16). A foreign key cannot cross a network or two
  physical databases; use a `call`/typed HTTP client instead.
- **CIAC0057** means a chain of required `Reference<T>` fields forms a
  cycle — no row in the cycle could ever be written first (v0.16).
- **CIAC0058** means a record containing a `Reference<T>` field is
  stored through an untyped keyed-document `crud X;` (v0.16); declare
  an explicit `table` or a typed `crud X: Record` instead.
- **CIAC0059** means a `unique: true`/`index: true` field attribute,
  or a top-level `index` declaration, names a field that doesn't
  exist, isn't backed by typed storage, or is a `Json` field (v0.16).
- **CIAC0060** means a validation attribute (`non_empty`, `min_length`,
  `max_length`, `min`, `max`, `format`) was used on a field type it
  doesn't apply to, given a contradictory bound, or given an unknown
  `format` value (v0.16).
- **CIAC0061** means a `transaction { .. }` block is empty, contains
  no database verb, nests another `transaction` inside it, contains a
  `return` (which could bypass the generated commit/rollback
  epilogue), or spans more than one database capability instance —
  CIaC does not synthesize two-phase commit (v0.16).
- **CIAC0062** means a `publish`, `cache`, `http`, `email`,
  `object_store`, or `search` verb appeared inside a
  `transaction { .. }` block (v0.16); none of these effects roll back
  with the database transaction. `publish` specifically: the
  transactional outbox is planned for v0.19 — publish after the
  transaction commits, or move it outside the block.
