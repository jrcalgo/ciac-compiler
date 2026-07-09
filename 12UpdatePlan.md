# CIaC v0.12 — Authoring Experience & Ecosystem (roadmap forecast)

> Forecast document. Assumes v0.9 (verification), v0.10 (legibility),
> and v0.11 (provider/deployment breadth) have landed. Direction-
> setting; the v0.12 planning pass finalizes the registry resolution
> syntax and LSP feature cut line.

## The gap this version closes

Every version through v0.11 makes CIaC more capable and more trustworthy
once someone is already writing `.ciac` source. None of them touch the
actual first contact: today that's an empty file and the examples
directory as the only reference material — there is no `ciac new`, no
scaffold, no starter template shipped as a CLI verb. And every version
through v0.11 leaves editing a `.ciac` file exactly as unassisted as
v0.1 did — zero IDE integration, no inline diagnostics, no
autocomplete on capability names or record fields, despite the
compiler already producing exactly the structured information (v0.10's
`--json` diagnostics) an editor integration would consume.

Blueprints (v0.8) solved reuse *within* a project — `std.Crud`,
`std.EventPipeline`, the audited-CRUD pattern, written once and
expanded per record. They solve nothing *across* projects or teams:
there is no way to publish a blueprint and no way for another project
to depend on one beyond copy-paste, which is exactly the DRY violation
blueprints exist to close, just moved up one level.

**v0.12 theme: the compiler is trustworthy, legible, and broad by this
point (v0.9-v0.11) — the remaining ceiling on adoption is entirely
about the first ten minutes and the tenth project, not the language or
runtime.**

## Pillar 1 — `ciac new`: scaffolding, not a blank file

```sh
ciac new my-service --template crud
ciac new my-system --template multi-service
```

- Templates are drawn from `examples/` itself (dogfooding — no
  parallel template format to maintain): `--template crud` starts from
  a trimmed `crud-notes.ciac` shape, `--template multi-service` from a
  trimmed `multi-service-media.ciac` shape, etc. New templates are
  just new example files with a scaffold-worthy trim, not a new
  subsystem.
- Output includes a starter `.ciac` file, a `README.md` pointing at
  `docs/language.md` and the relevant example, and nothing else — no
  generated output, no infra assumptions. Running `ciac build` on the
  scaffold immediately after `ciac new` is the smoke test this
  milestone ships with.

## Pillar 2 — Language Server Protocol

- A new binary (`ciac-lsp` or a `ciac lsp` subcommand) wrapping the
  existing `ciac-diagnostics`/`ciac-syntax`/`ciac-sema` pipeline behind
  the LSP wire protocol — the compiler's own parse/check pass is the
  implementation; this is not a second, separately-maintained
  understanding of the language, the same discipline v0.10's schema
  work applied to the wire protocol applied here to editor tooling.
- Cut line for v0.12, explicitly bounded to avoid an open-ended IDE
  tooling project:
  1. **Diagnostics on save/change** — the exact `CIAC00xx` errors
     `ciac check` already produces, surfaced inline, reusing v0.10's
     `--json` diagnostic shape as the LSP-to-compiler interface.
  2. **Hover** — capability/provider documentation, record field types,
     on hover.
  3. **Autocomplete** — capability names, provider names (from v0.11's
     expanded provider set), declared record field names, declared
     stream/api/handler names for pipeline step completion.
- Explicitly deferred past v0.12: rename/refactor support, find-all-
  references, code actions/quick-fixes. Diagnostics + hover +
  autocomplete is the version that makes editing `.ciac` feel like
  editing a typed language instead of a text file; the rest is
  incremental from there.

## Pillar 3 — Blueprint registry

- Extends the existing module loader (`ciac-syntax::module`, which
  already resolves `std/`-prefixed blueprint imports) rather than
  inventing a second resolution mechanism: a registry-prefixed import
  (`import "registry:org/blueprint-name@1.0";`, exact scheme finalized
  in planning) resolves over HTTP/git to plain `.ciac` blueprint
  source, cached locally, then flows through the identical parse →
  hygienic-expansion → validation path every local or `std/` blueprint
  already uses.
- No new execution model: a registry blueprint is `.ciac` source text,
  subject to exactly the same compiler validation as a hand-written
  one. There is no scripting, no build-time code execution beyond what
  `ciac build` already does for any input file — the registry adds a
  *resolution* mechanism, not a *trust* boundary the compiler doesn't
  already police.
- v0.12 scope is the resolution mechanism and a minimal reference
  registry (a plain versioned directory of blueprints, git-hosted) —
  not a package-manager-grade index, search UI, or namespace-ownership
  system. Those are plausible follow-ups once the mechanism has real
  usage to design against.

## Secondary items

- Editor syntax highlighting (Tree-sitter grammar, since it's reusable
  across more editors than a single TextMate grammar) — LSP gives
  diagnostics and completion but not colors without one.
- A "state of CIaC" documentation pass reconciling `docs/` against
  everything v0.9-v0.12 shipped, mirroring the reconciliation milestone
  that closed v0.8.

## Milestones

1. `ciac new` scaffold command + templates drawn from `examples/`.
2. LSP server: diagnostics on save/change (wrapping the existing
   check pipeline via v0.10's structured diagnostic shape).
3. LSP hover (capability/provider docs, record field types).
4. LSP autocomplete (capability/provider/field/step names).
5. Blueprint registry resolution mechanism, extending
   `ciac-syntax::module`; minimal reference registry.
6. Tree-sitter grammar; "state of CIaC" docs pass; version 0.12.0.

## Risks

- **LSP scope creep.** The three-feature cut line (diagnostics, hover,
  autocomplete) is deliberately conservative — mitigated by treating
  rename/refactor/code-actions as explicitly out of scope for this
  version rather than a stretch goal that erodes the milestone
  schedule.
- **Registry as an unbounded trust/security surface.** Mitigated by
  the design choice in Pillar 3: registry content is plain `.ciac`
  source validated by the exact same compiler passes as local code,
  never executed as anything richer than that — no new sandboxing
  problem is introduced because no new execution capability is.
- **Template drift.** `ciac new` templates sourced from `examples/`
  can go stale relative to the language the moment an example changes
  shape — mitigated by the same golden-snapshot discipline already
  covering `examples/`, extended to assert scaffolded output still
  builds cleanly.

## After v0.12

At this point CIaC is trustworthy end-to-end (v0.9), legible to tools
and agents (v0.10), broad enough for teams with real infrastructure
(v0.11), and approachable from first contact through the tenth shared
pattern (v0.12) — the four project goals stated at the start of this
whole arc (whole systems in `.ciac`, compiled to interchangeable
hosts, DRY composition, compiler-guaranteed correctness) now hold for
both a human developer and a coding agent, not just the language
core. Whether a v0.13 is warranted past this point is a question worth
re-asking against real usage once v0.9-v0.12 have shipped, not
pre-committing to now.
