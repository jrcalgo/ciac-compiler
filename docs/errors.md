# Error Code Index

Stable diagnostics emitted by the compiler. Codes are append-only: once
published, a code's meaning never changes. `ciac explain <code>` prints
the same explanations; this page is checked against the registry by a
test.

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
- **CIAC0038** means a handler uses the v0.7 typed signature or inline
  body syntax (`handler Name(..) -> Type { .. }` or
  `extern handler Name(..) -> Type;`); only the classic
  `handler Name { capability: instance; .. }` binding form is implemented
  so far — the typed HIR and emitters land in a later v0.7 milestone.
