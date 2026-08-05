# Agents working on this repository

CIaC compiles a declarative `.ciac` architecture description into a
runnable backend project (Python or Rust today, with a documented
external-backend protocol for more). This file is about working on
the *compiler*; `ciac new` and `ciac build` also emit an `AGENTS.md`
into every project *they* produce — that one is about working in the
generated output, not in here.

## Layout

- `crates/ciac-syntax` — lexer, parser, AST, module resolution.
- `crates/ciac-sema` — semantic analysis passes (`src/passes/`),
  blueprint expansion.
- `crates/ciac-ir` — the normalized graph both backends lower from.
- `crates/ciac-codegen` — the shared codegen framework: the per-target
  context model (`model.rs`), migrations, evolution/compatibility
  checks, compose/k8s/terraform assembly, the external-backend wire
  protocol, and the shared handler-body lowering walker plus the
  `HostSyntax` leaf-lowering contract (`src/lower/`).
- `crates/ciac-backend-python`, `crates/ciac-backend-rust` — the two
  bundled targets (minijinja templates + a `HostSyntax` impl each: a
  new target's own handler-body lowering is ~50 leaf methods against
  the shared walker in `ciac-codegen::lower`, not a hand-rolled
  walker — see `docs/backends.md`).
- `crates/ciac-diagnostics` — the `error_codes!` registry
  (`src/code.rs`, append-only), spans, rendering.
- `crates/ciac` — the `ciac` binary: `check`, `build`, `diff`,
  `verify`, `dev`, `new`, `lsp`, `describe`, `mcp`, `graph`, `explain`,
  `targets`.
- `examples/` — checked-in `.ciac` programs; `ciac new`'s templates
  and the golden/system-test suites are built from these, not a
  separate starter dialect.
- `docs/` — the language reference and per-topic guides.

## Build and test

```sh
cargo test --workspace                              # unit + golden + negative + determinism
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo insta review                                   # review intentional golden churn
```

`ciac verify` is the project's own truth signal, and it applies to
this repo's example fixtures too:

```sh
cargo run -p ciac -- verify examples/<name>.ciac --target python --out /tmp/gen-py
cargo run -p ciac -- verify examples/<name>.ciac --target rust --out /tmp/gen-rs
```

Full detail (the append-only diagnostic registry, adding a validation
pass, adding a language construct, adding a backend) lives in
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## The plan-file arc

`plans/NNUpdatePlan.md` files (`plans/06UpdatePlan.md`,
`plans/07UpdatePlan.md`, …) are forward-looking roadmap notes for a
version, written before that version's work starts. They record
intent, not history — once a version ships, its plan file becomes
historical color, not a spec to re-validate against.
`docs/language.md`'s provider support table and `docs/errors.md` are
the live, authoritative surface.

## Machine-readable front door

`ciac check|build|diff|verify --json` each print one versioned JSON
envelope on stdout (diagnostics resolved to file/line/column, plus
success); human narration stays on stderr. `ciac describe` prints the
language's full vocabulary — capabilities, providers with per-target
support, field types, builtin pipeline steps, declaration kinds, error
codes, scaffold templates — as one versioned JSON document; `ciac lsp`
renders its hover/completion from the same tables
(`crates/ciac/src/vocab.rs`), so the two can't drift apart. `ciac mcp`
exposes `check`/`build`/`diff`/`verify`/`graph`/`explain`/`describe` as
Model Context Protocol tools over stdio, for a client that would
rather call a tool than parse a CLI's stdout.

## Commit hygiene

Keep commits scoped to one concern with the tests that prove it. CI
requires fmt, clippy (warnings deny), the full test suite, and the
generated-output verification jobs to pass.
