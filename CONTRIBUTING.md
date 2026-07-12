# Contributing to CIaC

## Setup

Rust stable (pinned by `rust-toolchain.toml`) is all you need to build
and test the compiler. Validating generated Python output additionally
uses [uv](https://docs.astral.sh/uv/).

```sh
cargo test --workspace                 # unit + golden + negative + determinism
cargo fmt --all && cargo clippy --workspace --all-targets
```

## Golden snapshots

Compiler output (IR JSON, DOT, full generated projects) is snapshotted
with [insta](https://insta.rs). When an intentional change shifts
output:

```sh
cargo insta review          # or: INSTA_UPDATE=always cargo test -p ciac-integration-tests
```

Review the diff deliberately — snapshots are the compatibility record of
the compiler's observable behavior.

## Common changes

- **New diagnostic**: add a variant to the `error_codes!` registry in
  `crates/ciac-diagnostics/src/code.rs` (codes are append-only), document
  it in `docs/errors.md` (a test enforces this), and add a `tests/ui/`
  fixture with an `// expect: CIACnnnn` line.
- **New validation pass**: implement `Pass` in
  `crates/ciac-sema/src/passes/`, register it in `default_passes()`, and
  cover it in `crates/ciac-sema/tests/analyze.rs` plus a `tests/ui/`
  fixture.
- **New language construct**: grammar in `ciac-syntax` (update
  `docs/language.md`), lowering in `ciac-sema/src/build.rs`, then extend
  the shared model in `ciac-codegen/src/model.rs` and both backends'
  templates.
- **New backend**: see `docs/backends.md`.

## Generated-output validation

CI runs `ciac verify` over every example for both bundled backends. To
reproduce locally:

```sh
cargo run -p ciac -- verify examples/video-platform.ciac --target python --out /tmp/gen-py
cargo run -p ciac -- verify examples/video-platform.ciac --target rust --out /tmp/gen-rs
```

## Commit hygiene

Keep commits scoped to one concern with the tests that prove it. CI
requires fmt, clippy (warnings deny), the full test suite, and the
generated-output jobs to pass.
