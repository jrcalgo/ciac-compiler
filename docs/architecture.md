# Compiler Architecture

```text
.ciac source
  │
  │  ciac-syntax        logos lexer → recovering recursive-descent parser
  ▼
AST (ciac_syntax::ast)
  │
  │  ciac-sema::build   resolve names, satisfy capabilities,
  │                     expand crud/events into primitives
  ▼
SystemGraph (ciac-ir)
  │
  │  ciac-sema::passes  cycle-detection → reachability →
  │                     auth-placement → composition
  ▼
NormalizedIr (ciac-ir)  ← the validated contract
  │
  │  ciac-codegen       Backend::generate(&NormalizedIr) →
  │                     GeneratedProject (sorted, in-memory file tree)
  ▼
ciac-backend-python / ciac-backend-rust
  │
  │  ciac (CLI)         writes the tree, renders diagnostics
  ▼
runnable project
```

## Design rules

1. **Library crates never abort.** Every stage reports problems through
   `ciac_diagnostics::Diagnostics`; only the CLI decides exit codes.
   Parser errors recover at declaration boundaries so one mistake does
   not hide the rest of the program.

2. **Validation gates generation by type.** `NormalizedIr` is only
   produced by `ciac_sema::analyze` after all passes run error-free.
   Backends take `&NormalizedIr`, so generating from an unvalidated
   graph is unrepresentable.

3. **Expansion before validation.** `crud`/`events` lower into primitive
   nodes during graph building, so the passes validate exactly what the
   backends will generate.

4. **Determinism everywhere.** Graph iteration follows declaration
   order; generated files live in a `BTreeMap`; templates reject
   undefined variables; no timestamps or randomness. The determinism
   test generates every example twice and asserts byte equality.

5. **Backends are plugins behind one trait.** `ciac_codegen::Backend`
   plus the shared `ciac_codegen::model` context is the entire seam —
   see [backends.md](backends.md).

## Crate dependency graph

```text
ciac (CLI)
 ├── ciac-backend-python ─┐
 ├── ciac-backend-rust ───┼── ciac-codegen ── ciac-ir ── ciac-diagnostics
 ├── ciac-sema ───────────┤        (model, templates)
 │        └── ciac-syntax ┴────────── ciac-diagnostics
 └── ciac-diagnostics
```

## Validation passes

Passes implement `ciac_sema::passes::Pass`, are read-only over the
graph, and run in fixed order:

| Pass | Reports |
|------|---------|
| `cycle-detection` | `CIAC0006` — cycles over request/message/dependency edges (data flow excluded) |
| `reachability` | `CIAC0007` (warning) — apis without pipelines, workers nothing feeds, unused capabilities |
| `auth-placement` | `CIAC0008` — `Auth` not first, or in a worker pipeline |
| `composition` | `CIAC0009` — misplaced `Return`, repeated `Queue` |

Name resolution, capability checks (`CIAC0005`), and duplicate detection
(`CIAC0003`/`CIAC0012`) run during graph building, where declaration
spans are at hand.

## Testing strategy

| Layer | Where | What |
|-------|-------|------|
| Unit | each crate | lexer, parser recovery, graph ops, each pass |
| Golden | `tests/tests/golden.rs` | IR JSON, DOT, and full generated trees per example × backend (insta) |
| Negative | `tests/ui/*.ciac` | invalid programs annotated with `// expect: CIACnnnn` |
| Determinism | `tests/tests/determinism.rs` | double-generation byte equality |
| Regeneration | `tests/tests/regen.rs` | manifest stability, conflicts, seeded drift, orphans, adoption |
| Generated-output | `ciac verify` / CI | regeneration drift check plus `ruff` + `pytest` on Python output or `cargo check` on Rust output |
