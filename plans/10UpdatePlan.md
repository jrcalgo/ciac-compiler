# CIaC v0.10 — Legibility: External Backends & Agent-Native Tooling (roadmap forecast)

> Forecast document. Assumes v0.9 (system verification actually
> automated) has landed. Also formally versions and ships the
> v0.8-cycle external-backend protocol work (`protocol.rs`,
> `ExternalBackend`, `backends/go/`) that exists in the repo today
> unversioned, on top of `0.8.0`. Direction-setting; the v0.10 planning
> pass finalizes exact schema format and flag names.

## The gap this version closes

The external-backend protocol (request/response JSON over stdin/stdout,
proven this cycle against a real, standalone Go process) answers "can
a backend exist outside ciac's Rust workspace" with a clear yes. It
does not answer "can someone write that backend without reverse-
engineering ciac's internals by hand" — which is exactly what happened
building the Go proof-of-concept: piping `codegen-request` output
through `python3 -m json.tool` and reading field names off the JSON to
write Go structs by eye, then hand-translating `rust_type` strings
(`"String"` → `string`, `"i64"` → `int64`, ...) because `FieldCtx` has
no field-type representation that isn't already committed to a
specific host language.

The same legibility gap exists for ciac's own CLI. `check`/`build`/
`verify` emit human-formatted text — confirmed directly this cycle,
there is no `--json` anywhere on any of them except `graph`. A tool
(or an agent) consuming ciac's diagnostics today has to parse prose.

**v0.10 theme: everything a backend author or an external tool needs
to know about a ciac program should be data, not something reverse-
engineered from a human-readable dump.** This serves both audiences at
once — a third-party backend author and a coding agent hit the exact
same legibility wall, from the same two gaps.

## Pillar 1 — Ship the external-backend protocol, properly versioned

- Tag `protocol.rs`'s `PROTOCOL_VERSION`, `ExternalBackend`, and
  `backends/go/` as an official, documented v0.10 feature rather than
  unversioned work sitting on `0.8.0` — a `docs/external-backends.md`
  replacing the ad-hoc knowledge currently split across commit messages
  and `backends/go/README.md`.
- No behavior change to the protocol itself in this pillar — this is
  documentation and version-truth catching up to code that already
  works, so Pillars 2-3 below have a stable, named thing to extend.

## Pillar 2 — `FieldCtx`: one generic type hook, not four host-specific ones

`FieldCtx` (`ciac-codegen/src/model.rs`) carries `py_type`, `rust_type`,
`db_rust_type`, `sql_type` — baked-in knowledge of exactly the two
built-in hosts. The Go backend's only option today is parsing
`rust_type` strings and falling back to `string` for anything it
doesn't recognize (an enum's generated Rust type name, for instance) —
the exact "silent-fallback risk" the original v0.8 M6 spike report
flagged and this cycle's Go backend reproduced verbatim.

- Add `pub type_kind: FieldTypeKind` to `FieldCtx` — a small enum
  mirroring `ciac_ir::FieldType`'s shape (`Str`, `Int`, `Float`, `Bool`,
  `Uuid`, `Timestamp`, `Json`, `Enum { name: String, variants: Vec<String> }`)
  but living in `ciac-codegen` so it can derive `Serialize`/`Deserialize`
  without pulling `ciac-ir` into the wire contract. Additive: existing
  `py_type`/`rust_type`/`db_rust_type`/`sql_type` fields stay exactly as
  they are, so Python/Rust codegen needs zero changes.
- Update `backends/go/main.go`'s `goType()` to switch on `type_kind`
  instead of pattern-matching `rust_type` strings — proves the hook
  actually removes the workaround for the one real external backend
  that exists, not just in theory.
- This is the smallest fix that unblocks *every* future non-Rust
  backend, which is why it's a v0.10 pillar rather than deferred again:
  it was found once by the v0.8 spike, found again independently this
  cycle, and will be found a third time by the next language attempted
  if left alone.

## Pillar 3 — Publish the protocol schema

- Derive a JSON Schema for `CodegenRequest`/`CodegenResponse`/
  `SystemModel`/`Ctx`/`FieldCtx`/etc. directly from the Rust types
  (`schemars`, already a natural fit given everything in `protocol.rs`
  and `model.rs` derives `Serialize`/`Deserialize`) rather than hand-
  maintaining a spec that drifts from the real structs — the schema is
  regenerated as part of the build and checked into `docs/` or emitted
  by a `ciac codegen-schema` verb, so it can never silently go stale.
- `docs/external-backends.md` (from Pillar 1) walks through
  `backends/go/` as the worked example, now referencing the schema
  instead of a hand-derived field list.
- Stretch, if time allows: generate typed stub structs for one or two
  more languages (TypeScript interfaces, Python `TypedDict`s) from the
  same schema, purely as documentation-by-example — not new backends,
  just proof the schema is enough to bootstrap one.

## Pillar 4 — Agent-facing CLI ergonomics

- `--json` on `check`/`build`/`verify`: structured diagnostics
  (`file`, `line`, `column`, `code`, `severity`, `message`, `help`) as
  an array, mirroring the shape the diagnostics infrastructure already
  carries internally — this is exposing existing structure, not
  inventing new information.
- A dry-run/explain verb extending `codegen-request`'s existing
  read-only instinct: given a `.ciac` file and an existing output
  directory's `.ciac/manifest.json`, report what a real build would
  change (new files, files that would be regenerated vs. left alone
  per the regeneration/`Seeded` rules, schema migrations that would be
  produced) without writing anything — lets an agent (or a human)
  evaluate the blast radius of an edit before committing to it.

## Secondary items

- A minimal MCP server or Claude Code skill wrapping `check --json`/
  `build --json`/the dry-run verb — explicitly optional, stretch scope;
  the CLI contract from Pillars 3-4 is the real deliverable, a
  particular agent-tooling wrapper is not load-bearing for it.

## Milestones

1. `FieldCtx.type_kind` addition; `backends/go/` updated to consume it
   instead of `rust_type` string-matching.
2. `schemars`-derived JSON Schema generation for the protocol types;
   `docs/external-backends.md`.
3. `--json` structured output for `check`/`build`/`verify`.
4. Dry-run/explain verb (manifest-aware, no disk writes).
5. Version bump and formal changelog entry covering both this
   version's work and the previously-unversioned M1-M3 external-backend
   protocol; version 0.10.0.

## Risks

- **Two API surfaces to keep in sync.** `--json` output and the
  protocol schema both become contracts external tools depend on the
  moment they ship — mitigate by versioning the JSON output shape
  alongside `PROTOCOL_VERSION` from day one, not retrofitting a version
  field after the first breaking change.
- **`type_kind` becoming another hardcoded enum that ages the same way
  `FieldType` did.** Mitigate by keeping it a closed, small set
  matching `ciac_ir::FieldType` exactly — it inherits that type's own
  discipline (a new field type requires a language-level grammar
  change anyway, so the enum is never the bottleneck).
- **Scope creep into a full agent SDK.** The secondary MCP/skill item
  is explicitly not required for this version to be complete — the
  CLI/schema contract stands alone and is useful to any tooling, agent
  or otherwise.

## After v0.10

ciac's own output — diagnostics, the wire protocol, what a build would
do before it does it — is now data any tool can consume, and the one
concrete gap blocking non-Rust backends from real type fidelity is
closed. v0.11 spends this legibility on breadth: more capability
providers, Kafka, and a real production deployment story, all of which
benefit from the same structured-verification (v0.9) and structured-
introspection (v0.10) foundation when things inevitably need debugging
at scale.
