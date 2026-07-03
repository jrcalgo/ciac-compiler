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
