# Writing a Code-Generation Backend

Backends turn the validated IR into a project. The language, IR, and
validation never change when a target is added.

> This page covers **in-process** backends (a Rust crate implementing
> the `Backend` trait). A backend can also be a standalone executable
> in any language, speaking JSON over stdin/stdout — see
> [external-backends.md](external-backends.md) (v0.10).
>
> v0.22 rewrite: this page now reflects "the backend factory"
> (`22UpdatePlan.md`) — the whole per-target integration surface
> lives on `TargetInfo` instead of being scattered across six files,
> a shared HIR scanner is computed once, and a conformance harness
> proves cross-target parity mechanically. See that plan's Pillar 6
> for the design reasoning; this page is the resulting how-to.

## The seam

```rust
pub trait Backend {
    fn id(&self) -> &'static str;                 // "python", "rust", "go", ..
    fn description(&self) -> &'static str;
    fn supports(&self, component: &Component) -> bool;
    fn generate(&self, ir: &NormalizedIr, opts: &GenOptions)
        -> Result<GeneratedProject, BackendError>;
    fn target_info(&self) -> &'static TargetInfo;
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
- `target_info()` (v0.22 M1) is the whole CLI/CI/compose/dev-loop/sim
  integration surface, as data: project marker, migrations directory
  and filename mapping, `ciac verify`'s validate steps, the generated
  CI workflow's test steps, compose parameterization, `ciac dev`'s
  rebuild/restart commands, and `ciac sim` support level
  (`Full`/`Narrow { unsupported }`/`None { reason }`). Every CLI
  surface that used to `match target { "python" => .., "rust" => .. }`
  reads this instead — a repo test (the "grep fence",
  `tests/tests/target_literal_fence.rs`) fails the build on a new
  unjustified target-name string literal outside a backend crate, so
  this can't quietly regrow.

## Recipe

0. **Start from the skeleton**, not a blank crate:
   `backends/skeleton-internal/` is a compiling, gated (`supports()`
   always `false`) reference crate demonstrating everything below —
   copy it to `crates/ciac-backend-<target>` and work from there.
1. **Crate**: depends on `ciac-codegen`, `ciac-ir`, `include_dir`,
   `minijinja`. `cargo build`/`cargo test` on the new crate should be
   green immediately — the skeleton's own three tests
   (`emits_its_declared_file_set`, `supports_nothing_yet`,
   `target_info_is_populated_and_gated`) are the shape yours starts
   from.
2. **`TargetInfo`**: fill in the `static TARGET_INFO: TargetInfo`
   struct literal — marker file, migrations directory (+ the identity
   filename mapping unless your target needs a real rename, like
   Java/Flyway's `0001_slug.sql` → `V0001__slug.sql`), `validate`
   steps (what `ciac verify` runs), `ci_test_steps` (the literal YAML
   `ciac build --deploy ci` embeds), `compose` (the existing
   `BackendComposeOpts`, now reached through the trait), `dev`
   (rebuild/restart commands), `source_extension` (your seeded
   handler files' extension, so `ciac dev`'s watch loop picks them
   up), and `sim` (start with `SimSupport::None { reason }` — a
   simulation slice is real, optional, later work, see
   [simulation.md](simulation.md)). One struct literal buys every CLI
   surface's target-aware behavior for free.
3. **Model**: call `ciac_codegen::model::build_system(ir, opts)` — the
   shared, language-neutral `SystemModel { project_name, multi,
   services: Vec<Ctx> }`. Single-service programs yield one `Ctx`
   (emit it at the output root); multi-service programs yield one per
   service (emit each under `<ctx.dir>/`, skip per-service compose
   files, and render root system compose/README from the whole model).
   Each `Ctx` precomputes casing variants, per-pipeline steps,
   capability instances, handler injection, scheduled jobs, realtime
   channels, and typed `call` client targets. Add fields there (not in
   your backend) if every target would need them — and check
   `FieldTypeKind`/`NameForms` first: a per-language *spelling* of
   already-neutral data belongs in your own backend as a filter (next
   step), not as a new field on the shared model.
4. **Type filters**: register `<lang>_type`-style minijinja filters
   against `FieldCtx`'s neutral `type_kind: FieldTypeKind` — see
   `ciac-backend-python`/`-rust`'s `filters.rs` for the pattern
   (`env.add_filter("py_type", filters::py_type)`, consumed in
   templates as `{{ field | py_type }}`). This is how a field's Python
   `str`/Rust `String` spelling gets computed without the shared model
   ever hardcoding it.
5. **Templates**: a flat `templates/*.j2` directory embedded with
   `include_dir!`. Build the environment with
   `ciac_codegen::template::environment(..)` — it installs `snake_case`
   / `pascal_case` / `kebab_case` filters and *fails on undefined
   variables*, so template bugs fail generation instead of corrupting
   output. For the standard file set (build file, Dockerfile, README,
   config, state, observability, ...), declare it as data instead of a
   hand-rolled sequence of `project.add_file(..)` calls:
   ```rust
   const EMIT: &[ciac_codegen::emit::Emit] = &[
       ciac_codegen::emit::Emit::always("README.md", "README.md.j2"),
       ciac_codegen::emit::Emit::when("src/auth.rs", "auth.rs.j2", |c| c.has_auth),
       // ...
   ];
   ```
   then call `ciac_codegen::emit::run(&env, ctx, EMIT, prefix, &mut project)`.
   `Emit` (v0.22 M5) covers the always/conditional-single-file tier;
   per-item files (one per declared api/worker/job/consumer/channel/
   resource/call-target) are still a hand-written loop today — see
   either bundled backend's `emit_service` for that half, and
   `22UpdatePlan.md`'s M5 section for the disclosed scope boundary.
6. **HIR lowering** (only if you support typed inline handler bodies):
   `ciac_codegen::lower::scan(ir, hir_body)` is the one shared HIR
   traversal both bundled backends call — it returns a `Needs` struct
   naming every capability/table/record/enum a handler body touches,
   computed once so a backend can't independently drift on which
   verbs it actually lowers. Expression/statement lowering itself
   (`py_expr`/`lower_tail` vs `rust_expr`/`rust_stmt`) is still
   hand-written per backend — v0.22 M3 shipped the scanner unification
   only (the correctness-bearing half: `Needs::unguarded_verbs` feeds
   `ciac sim`'s capability-coverage refusal); a shared leaf-lowering
   contract (`HostSyntax`) remains real, scoped follow-up work. Study
   `ciac-backend-python`/`-rust`'s `lower.rs` for the expected shape
   of your own leaf-lowering (`FieldAccess`, `RecordCons`, each
   `Verb`, control flow).
7. **Register**: add one line to `backends()` in
   `crates/ciac/src/commands.rs` (mirrored in
   `tests/src/lib.rs::backends`). Nothing else changes — no edits to
   `ci.rs`/`vocab.rs`/`dev.rs`/`compose.rs`; they all read the
   registry via `target_info()`.
8. **Tests**: golden + determinism coverage is automatic once
   registered (every example × every backend, `tests/tests/golden.rs`).
   The conformance harness (`tests/tests/conformance.rs`, v0.22 M4)
   automatically starts asserting your target's `openapi.json` is
   byte-identical to every other target's for the same program, and
   that every subject/queue-group/cron-schedule/table-name the model
   declares actually appears in your generated output — no test edits
   needed, it's registry-driven. Add a CI job that compiles/lints your
   generated output like `generated-python` / `generated-rust` in
   `.github/workflows/ci.yml`.

## Quality bar

Both bundled backends hold the line the next target should match:

- generated projects build/lint clean (`ruff` for Python, zero-warning
  `cargo check` for Rust) with **no infrastructure running** — clients
  connect lazily;
- business logic lives in stub handler files marked as `Seeded`
  user-owned files; everything else is compiler-owned and regenerable;
- a `docker-compose.yml` provisions exactly the declared capabilities;
- structure mirrors the other targets (routers / services / workers/jobs
  / channels / config), because backends share the same model;
- `openapi.json`, migration SQL, and declared topology (subjects,
  queue groups, cron schedules, table names) are byte-identical or
  provably present across every target for the same program — the
  conformance harness enforces this, it isn't just a style goal.

## What a backend costs today

Measured, not estimated — `22UpdatePlan.md`'s audit before the M1-M5
factory work, and the actual numbers after it:

| | Before (v0.19.0, pre-factory) | After (v0.22, M1-M5 shipped) |
| --- | --- | --- |
| lib.rs emission wiring | ~500 lines/backend | unchanged (~509 rust / ~374 python) — `Emit` exists (M5) but porting the two real backends' per-item loops onto it is disclosed, deferred follow-up |
| lower.rs (scanner + leaves) | ~1,200 lines/backend | ~800-1,060 lines/backend (scanner moved out; leaf/dispatch lowering — the larger remaining opportunity — still hand-written per backend, disclosed at M3) |
| shared `ciac-codegen::lower` scanner | did not exist (duplicated ~150 lines/backend instead) | 358 lines, one copy, both backends consume it |
| templates | ~2,800 lines/backend | unchanged — essential, by design |
| model.rs per-language fields | ~27 `py_*`/`rust_*` fields | `FieldCtx`'s 4-field group deleted (M2, backend-owned filters instead); ~20 remaining fields (composed filters — `ExtraDepCtx`, `CfgFieldCtx`, `ArmCtx`, `BindingCtx`, ...) are disclosed, deferred follow-up |
| edits outside your crate | ~6 files, ~25 sites | 1 line (the registry) — closed at M1, held by the grep fence |
| conformance proof | ad hoc per arc | `tests/tests/conformance.rs`: C1 (generate succeeds), C2 (goldens), C3 (OpenAPI byte-equality), C4 (topology equality) all run automatically once registered; C5 delegates to CI; C6/C7 await a third target |

**Honest summary:** the factory closed the seam that was cheapest to
fix and most dangerous to leave (seam 3 — scattered per-target
`match` sites, seam 1's clearest instance, and seam 2's
correctness-bearing scanner half). The largest remaining line-count
opportunity — unifying per-backend leaf/statement lowering behind a
shared `HostSyntax` contract, and porting real per-item emission onto
`Emit` — is real, was scoped and explicitly deferred rather than
rushed, and is recorded as follow-up work in `22UpdatePlan.md`'s M2,
M3, and M5 milestone notes. A new backend today is still primarily
template-writing plus per-backend leaf lowering, not primarily
archaeology across the compiler's shared crates — which was the
factory's actual goal.

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
email, search, external HTTP, or `auth`). Both refusals are now driven
by `TargetInfo::sim` (v0.22 M1) rather than a per-call-site `match`.

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
(`ciac_backend_rust::unsupported_sim_capabilities`, now surfaced
through `TargetInfo::sim`'s `Narrow { unsupported }` variant) that
refuses with the specific unsupported verb/capability list — computed
from the same shared `ciac_codegen::lower::scan`'s `Needs::
unguarded_verbs` (v0.22 M3) every backend's `render()` already calls,
so it cannot drift from what's actually scanned. See 17UpdatePlan.md's
M11 entry for the full account, including a real pre-existing Rust
codegen bug (E0382, unrelated to either pass) the first pass's live
sweep surfaced and fixed.
