# Writing a Code-Generation Backend

Backends turn the validated IR into a project. The language, IR, and
validation never change when a target is added.

## The seam

```rust
pub trait Backend {
    fn id(&self) -> &'static str;                 // "python", "rust", "go", ..
    fn description(&self) -> &'static str;
    fn supports(&self, component: &Component) -> bool;
    fn generate(&self, ir: &NormalizedIr, opts: &GenOptions)
        -> Result<GeneratedProject, BackendError>;
}
```

- `supports` is checked for every node before `generate` runs
  (`ciac_codegen::check_support`); unsupported constructs become the
  user-facing `CIAC0011`, never a template crash.
- `GeneratedProject` is an in-memory file tree: relative paths only, no
  traversal, no duplicate writes, sorted iteration. Determinism rules:
  no timestamps, no randomness, iterate only ordered collections.

## Recipe

1. **Crate**: `crates/ciac-backend-<target>` depending on `ciac-codegen`,
   `ciac-ir`, `include_dir`, `minijinja`.
2. **Model**: call `ciac_codegen::model::build(ir, opts)` — the shared,
   language-neutral context: casing variants, per-pipeline steps,
   capability flags, and which handlers need db/cache injection. Add
   fields there (not in your backend) if every target would need them.
3. **Templates**: a flat `templates/*.j2` directory embedded with
   `include_dir!`. Build the environment with
   `ciac_codegen::template::environment(..)` — it installs `snake_case`
   / `pascal_case` / `kebab_case` filters and *fails on undefined
   variables*, so template bugs fail generation instead of corrupting
   output.
4. **Register**: add one line to `backends()` in
   `crates/ciac/src/commands.rs` (mirrored in
   `tests/src/lib.rs::backends`).
5. **Tests**: golden + determinism coverage is automatic once
   registered (every example × every backend). Add a CI job that
   compiles/lints your generated output like `generated-python` /
   `generated-rust` in `.github/workflows/ci.yml`.

## Quality bar

Both bundled backends hold the line the next target should match:

- generated projects build/lint clean (`ruff` for Python, zero-warning
  `cargo check` for Rust) with **no infrastructure running** — clients
  connect lazily;
- business logic lives in stub handler files that are documented as
  user-owned; everything else is compiler-owned and regenerable;
- a `docker-compose.yml` provisions exactly the declared capabilities;
- structure mirrors the other targets (routers / services / workers /
  config), because backends share the same model.
