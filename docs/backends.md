# Writing a Code-Generation Backend

Backends turn the validated IR into a project. The language, IR, and
validation never change when a target is added.

> This page covers **in-process** backends (a Rust crate implementing
> the `Backend` trait). A backend can also be a standalone executable
> in any language, speaking JSON over stdin/stdout — see
> [external-backends.md](external-backends.md) (v0.10).

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
  traversal, no duplicate writes, sorted iteration. Each file also has an
  ownership role (`Owned` or `Seeded`) used by regeneration manifests.
  Determinism rules: no timestamps, no randomness, iterate only ordered
  collections.

## Recipe

1. **Crate**: `crates/ciac-backend-<target>` depending on `ciac-codegen`,
   `ciac-ir`, `include_dir`, `minijinja`.
2. **Model**: call `ciac_codegen::model::build_system(ir, opts)` — the
   shared, language-neutral `SystemModel { project_name, multi,
   services: Vec<Ctx> }`. Single-service programs yield one `Ctx`
   (emit it at the output root); multi-service programs yield one per
   service (emit each under `<ctx.dir>/`, skip per-service compose
   files, and render root system compose/README from the whole model).
   Each `Ctx` precomputes casing variants, per-pipeline steps,
   capability instances, handler injection, scheduled jobs, realtime
   channels, and typed `call` client targets. Add fields there (not in
   your backend) if every target would need them.
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
- business logic lives in stub handler files marked as `Seeded`
  user-owned files; everything else is compiler-owned and regenerable;
- a `docker-compose.yml` provisions exactly the declared capabilities;
- structure mirrors the other targets (routers / services / workers/jobs
  / channels / config), because backends share the same model.

## Simulation (v0.17)

`ciac sim`/`verify --sim` drive a generated project's real code through
in-memory fakes instead of real provider containers — see
[simulation.md](simulation.md) for the claim boundary and status. A new
backend does not get simulation support for free: `ciac sim --target
<other>` refuses cleanly rather than silently no-op'ing until that
target's port/adapter seam and fakes are built. Python fakes every
capability; Rust (v0.17 M11) fakes a deliberately narrow slice —
`db.insert` and broker publish/consume/cron jobs only — and refuses,
naming the specific gap, for any program using something wider
(`db.get`/`update`/`delete`/`query`/`count`, cache, object store,
email, search, external HTTP, or `auth`).

v0.17 M11 shipped in two passes. The first closed a real, separate gap
this bullet list above already claimed but the Rust backend didn't yet
meet: the broker client and OAuth2 JWKS lookup were the last two eager
infrastructure dependencies in generated Rust code (every db pool was
already lazy) — both are now lazy and cached, matching the "no
infrastructure running" quality bar for real. That alone was a
production-path fix, not simulation support itself — but it's the
precondition `AppState::simulation` needs (constructing every field
must already be infrastructure-free before a `world` field can safely
override just the parts that need faking).

A follow-up pass then built simulation support itself: `crates/ciac-
sim/src/world.rs`'s `SimWorld` (a `FakeDatabase`/`FakeQueue` wired to
`ciac-sim`'s own real `FailureEngine` — Rust can depend on `ciac-sim`
directly, unlike Python, which must restate it narrowly), vendored via
`include_str!` into every generated project that declares `db`/`queue`;
an `AppState.world`/`AppState::simulation()`/`AppState::publish()`
world-guard mirroring Python's `AppState.production()`/`.simulation()`
split; a generated `src/bin/sim_runner.rs` scenario interpreter
(per-program generated, not embedded at CLI-invocation time like
Python's, since Rust needs concrete types at compile time); and `ciac
sim --target rust` itself, gated by a capability-coverage check
(`ciac_backend_rust::unsupported_sim_capabilities`) that refuses with
the specific unsupported verb/capability list rather than letting an
unguarded verb silently fall through to real, unreachable
infrastructure. See 17UpdatePlan.md's M11 entry for the full account,
including a real pre-existing Rust codegen bug (E0382, unrelated to
either pass) the first pass's live sweep surfaced and fixed.
