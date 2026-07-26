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
>
> Continuation update: Pillar 3's Parts 2-3 (the shared statement/
> expression dispatcher and the `HostSyntax` leaf-lowering trait),
> deferred at v0.22 M3 under that milestone's own pre-agreed
> fallback, have since landed — both bundled backends now implement
> `HostSyntax` against the shared walker in
> `ciac_codegen::lower::{lower_body_expr, lower_body_stmt}` rather
> than each hand-rolling a full walker. Step 6 below reflects the
> real recipe.

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
   the walk is shared; you implement `ciac_codegen::lower::HostSyntax`
   and hand your target's leaf constructors — roughly 50 methods —
   to a shared statement/expression dispatcher that already owns
   block/tail shaping, precedence, enum-literal use-site recovery,
   float-literal fidelity, and divergence truncation.
   - `ciac_codegen::lower::scan(ir, hir_body)` is the one shared HIR
     traversal every backend calls first — it returns a `Needs` struct
     naming every capability/table/record/enum a handler body touches,
     computed once so a backend can't independently drift on which
     verbs it actually lowers (`Needs::unguarded_verbs` feeds
     `ciac sim`'s capability-coverage refusal).
   - Pick your `Orientation`: `Expression` if `if`/`match`/db verbs are
     real nested values in your target (Rust), `Statement` if your
     target's control flow can't be nested inside another expression
     (Python; Go and Java are expected to be `Statement` too — see
     `24UpdatePlan.md`/`25UpdatePlan.md`).
   - Implement `HostSyntax` for a small struct holding `&NormalizedIr`
     (every leaf resolves table/record/enum names through it) plus
     whatever your target needs precomputed once per handler (Rust's
     `RustSyntax` also holds the bound db instance's engine, since
     every db-verb leaf needs the same placeholder style). Leaves
     receive already-lowered child strings plus stable IR ids
     (`TableId`/`RecordId`/`NodeId`) — never raw HIR trees, with one
     documented exception (`value_for_record_field`'s `original`
     parameter, for a clone-discipline hook a GC'd target no-ops).
   - Call `ciac_codegen::lower::lower_body_expr`
     (`Orientation::Expression`) or `lower_body_stmt`
     (`Orientation::Statement`) from your own `render()` — see either
     bundled backend's `lower.rs` for the expected shape, and
     `ciac_codegen::lower::{IdentitySyntax, IdentitySyntaxStatement}`
     (exercised in `tests/tests/host_syntax_identity.rs`) for a
     minimal reference implementation of every leaf against real HIR.
   - The contract is amendable, not frozen forever: a signature change
     (e.g. a new verb, or a target-specific tail-shaping need like
     Go's multiple-return error idiom) lands as its own commit — the
     trait change, an updated identity-syntax golden, and
     byte-identical goldens for every *existing* target — before a
     new target's leaves consume it.
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
factory work, the actual numbers after it, and this continuation's
own measured numbers for Pillar 3 Parts 2-3 (the shared dispatcher and
`HostSyntax`):

| | Before (v0.19.0, pre-factory) | After M1-M5 (v0.22) | After Parts 2-3 (this continuation) |
| --- | --- | --- | --- |
| lib.rs emission wiring | ~500 lines/backend | unchanged (~509 rust / ~374 python) — `Emit` exists (M5) but porting the two real backends' per-item loops onto it is disclosed, deferred follow-up | unchanged — out of this continuation's scope |
| lower.rs | ~1,200 lines/backend | ~800-1,060 lines/backend (scanner moved out; leaf/dispatch lowering still hand-written per backend, disclosed at M3) | **577 (rust) / 869 (python)** — leaves only; Python's figure still includes its ~320-line generated-behavioral-test family (`render_test`/`dummy_value`/...), which depends only on the shared scanner and was never in scope to move. Net of that, both land inside the original M3 cost model's own "~450-600 leaves only" estimate. |
| shared `ciac-codegen::lower` | 358 lines (scanner only) | 358 lines (scanner only) | **1,980 lines**: `scan.rs` 364 (moved verbatim), `dispatch.rs` 792 (the shared walker + utilities + unit tests), `host_syntax.rs` 317 (the `HostSyntax` trait + support types), `identity.rs` 461 (two complete reference implementations, one per orientation), `mod.rs` 46 |
| templates | ~2,800 lines/backend | unchanged — essential, by design | unchanged |
| model.rs per-language fields | ~27 `py_*`/`rust_*` fields | `FieldCtx`'s 4-field group deleted (M2); ~20 remaining fields (composed filters) disclosed, deferred | unchanged — a different, unrelated leftover from M2, not folded into this continuation |
| edits outside your crate | ~6 files, ~25 sites | 1 line (the registry), held by the grep fence | unchanged |
| conformance proof | ad hoc per arc | `tests/tests/conformance.rs` (C1-C5 wired, C6/C7 await a third target) | unchanged, plus a new tier: `tests/tests/host_syntax_identity.rs` proves the *contract itself* renders every typed handler in the 26-example corpus from both orientations |

**Honest summary:** total line count across the three files
`lower.rs`+`lower.rs`+`ciac-codegen::lower` went *up*, not down (2,220
→ 3,426) — Parts 2-3 is not a line-count optimization, and the M1-M5
table above should not be read as promising one. What moved is *where*
the lines live and what a *new* backend must write: precedence,
block/tail shaping, enum-literal use-site recovery, float-literal
fidelity, and divergence truncation are now defined exactly once,
proven against both orientations by `host_syntax_identity.rs`, and a
third backend (TypeScript/Go/Java) consumes that walker for free —
implementing `HostSyntax`'s roughly 50 leaves, not writing and
re-debugging a second (or third, or fourth) full walker. `22UpdatePlan.md`'s
own M3 note named exactly this as the deferred, harder half of seam 2;
this continuation is where it landed, byte-identical goldens intact
end to end.

## Divergence ledger

Two tables, drafted and populated once (`26UpdatePlan.md` Pillar 5/M7)
and kept current since: what differs across targets *by design*, and
what differs *because the work isn't done yet*. This is the front
matter for the narrative below (the `## Simulation (v0.17)` section
and this repo's other per-arc retrospectives carry the full story;
these tables are the index a reviewer reads first) — replacing no
prose, just giving "what, exactly, differs across targets, and is each
difference a decision or a debt?" an answer at a glance instead of a
paragraph-reconstruction exercise. The structural rule: a permanent row
needs a *reason*; an open row needs an *address* — a plan file that
owns closing it, or an explicit "no plan yet" (which a row is always
allowed to say; what it may not do is hide among the permanent rows).
`tests/tests/ledger_integrity.rs` enforces both rules mechanically:
every "Closes in" reference (other than an explicit "no plan yet") must
name a plan file that exists in the repo root, and no divergence string
may appear in both tables.

### Permanent by design

| Divergence | Targets | Why this is a decision |
| --- | --- | --- |
| Migrations executor | all five | Rust: `sqlx::migrate!` (compile-time). Java: Flyway. Python/TypeScript/Go: a hand-rolled generated runner (its own `_ciac_migrations` ledger table, applying `migrations/*.sql` in filename order) rather than a heavier third-party framework — `db.go.j2`'s own doc comment names this choice explicitly ("the simpler answer", over `golang-migrate`). All five run identical, CIaC-owned SQL, content-equality-tested cross-target; the *executor* is each ecosystem's own idiom (or, for three of five, a deliberately simple in-house runner), and replacing five with one bespoke cross-language runner would trade audited-or-deliberately-simple machinery for NIH risk. |
| Cron translation library | all five | Python: `croniter`. Rust: the `cron` crate. TypeScript: `croner`. Go: `robfig/cron/v3`. Java: Spring's own built-in `@Scheduled(cron = ...)` (no separate library at all). One shared 5-field cron expression, a different scheduler (or framework feature) per ecosystem; the equivalence suite proves schedule agreement across all five — the library choice is the ecosystem's, forever. |
| Deploy artifact shape/size | all five | venv + interpreter (Python) / stripped static binary (Rust) / `node_modules` image (TypeScript) / static binary (Go) / JRE base image, ~200MB (Java — `jlink`/GraalVM native-image slimming is real, disclosed future work, not attempted this arc; `25UpdatePlan.md`'s own retrospective recorded the measured number). The artifact shape is the language's. |
| Go's `time.Time.MarshalJSON` fractional-seconds trimming | Go | RFC 3339-compliant and wire-compatible; confirmed live with a standalone `encoding/json.Marshal` check at v0.24 M9 (not hypothesized from the stdlib docs), asserted-as-documented here since no checked-in example reaches the code path yet. Changing it would mean fighting the stdlib for cosmetics. |
| Error idiom in generated code (`Result` / exceptions / error returns) | all five | `24UpdatePlan.md` M4's amendment; the wire envelope is identical across every target, the in-language shape belongs to the language. |
| Executor-seam shape (context hooks + a depth cell, not a uniform-connection rewrite) | Rust | `26UpdatePlan.md` M1's design-A-vs-B decision, closed at M2 with a live rollback proof (Postgres/MariaDB/SQLite); recorded here as the permanent shape so future db verbs adopt it rather than re-litigate it. |

### Open (tracked)

| Gap | Targets | Closes in |
| --- | --- | --- |
| Simulation depth: only `db.insert` + publish faked | Go, Java | `27UpdatePlan.md` M7–M8 — Rust CLOSED at M4, TypeScript CLOSED at M6 (gate-emptiness proven across the whole example corpus, live-verified against nine corpus scenarios each; proof recorded in each milestone's own Shipped note) |
| Multi-service programs refused by `ciac sim` | all five | `28UpdatePlan.md` |
| `transaction {}` non-atomic in production | Rust | `26UpdatePlan.md` M1–M2 — CLOSED, live rollback proof recorded in that milestone's own Shipped note |
| `logging Structured` refused (`CIAC0011`) | Java | `26UpdatePlan.md` M3 — CLOSED, `LogShapeTest` proof recorded in that milestone's own Shipped note |
| OAuth2 scope tests excluded from the no-infra suite | all five | `26UpdatePlan.md` M4–M5 — CLOSED, five-target live-proof recorded in those milestones' own Shipped notes |
| Sim record/replay | Rust, TypeScript, Go, Java | no plan yet — visibly unscheduled, not hidden among the permanent rows |
| No external human security audit | repo | no plan yet; automated dependency/vulnerability scanning (`26UpdatePlan.md` M6) is the standing floor, not the ceiling |

## Simulation (v0.17)

See the Divergence ledger above for which of this section's own gaps
are permanent decisions and which are open, addressed debts — this
section is the linked detail those tables index, not a restatement.

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

TypeScript reached the identical narrow slice in 23UpdatePlan.md M9,
via the same two-part shape with one necessary substitution: since
TypeScript cannot `include_str!` Rust source, `src/world.ts`'s
`SimWorld` is a hand-written restatement (occupying the position
Python's own `sim/pyrunner/world.py` restatement already does) instead
of a vendored copy, but fakes the identical `db.insert`/broker
publish-consume pair, checked by `AppState.world`/
`createSimulationState()`/`queue.ts`'s world-guarded `publish()` free
function, and refused by `ciac_backend_ts::unsupported_sim_capabilities`
— computed from the exact same shared `ciac_codegen::lower::scan`
Rust's own gate uses, `pub(crate) use`-re-exported into this backend's
own `lower` module the same way Rust's already was. A generated `src/
sim_runner.ts` (a generic scenario interpreter, since — like Rust —
TypeScript needs concrete per-program route/worker/job names baked in
at codegen time) drives `app.inject()` for requests, matching Rust's
own `tower::ServiceExt::oneshot` no-live-listener approach; built with
`{ logger: false }` specifically, since Fastify's real logger would
otherwise interleave with the runner's own one-line JSON reply on
stdout, a wrinkle Rust's own `tracing` setup never had to solve because
it writes to stderr by default. One genuine target-specific
simplification, disclosed in [simulation.md](simulation.md): TypeScript
production code gives `transaction {}` real atomicity (matching Rust's
own production code since `26UpdatePlan.md` M1), but degrades to
non-atomic, unwrapped-statement behavior *only* under simulation, since
there is no live database for a real `BEGIN`/`COMMIT` to run against a
`SimWorld`.

Go reached the same narrow slice in 24UpdatePlan.md M9, structurally
identical to TypeScript's own shape (Go cannot `include_str!` Rust
source either, so `internal/world/world.go`'s `World` is a hand-written
restatement in the same position as Python's/TypeScript's own), fakes
the identical `db.insert`/broker publish-consume pair, checked by
`state.AppState.World`/`state.NewSimulation()`/`queue.PublishJSON`'s
world-guarded branch, and refused by
`ciac_backend_go::unsupported_sim_capabilities` — computed from the
same shared `ciac_codegen::lower::scan`, `pub(crate) use`-re-exported
into this backend's own `lower` module exactly the way Rust's and TS's
already were. A generated `cmd/sim_runner/main.go` (a generic scenario
interpreter, needing concrete per-program route/worker/job names baked
in at codegen time, same reason as Rust/TS) drives `net/http/httptest`
for requests (no live listener), matching Rust's `tower::ServiceExt::
oneshot`/TS's `app.inject()` approach exactly; unlike TS's Fastify
logger wrinkle, Go's `slog` default handler already writes to stderr,
so no `{ logger: false }`-equivalent construction option was needed.
Go's own production code gives `transaction {}` **real** atomicity
unconditionally (`database/sql`'s `*sql.Tx`, the same bar TS's and
Rust's own Postgres branches hold), and — like TypeScript — degrades to a
guarded no-op only under simulation: `transaction_stmt` declares its
`*sql.Tx` handle unconditionally (typed `nil`, since Go requires the
identifier to exist even on a path that never runs) but skips the real
`BeginTx`/`Commit`/`Rollback` calls when `state.World` is set, since
every db verb this checkpoint's sim gate allows inside a transaction
(`db.insert` only) already redirects to `World` itself.

Two real bugs the Go pass's own live sweep surfaced, neither
hypothesized from reading the templates: (1) `cmd/sim_runner/main.go`'s
worker-dispatch table was first written as a Go `switch` on the
message subject, which compiles-time-rejects two `case` arms sharing
the same constant value — exactly the shape `examples/sim-broker-
slice.ciac`'s two workers on one stream produce (the "first worker
registered for a subject wins" semantics `world.go`'s own doc
discloses); fixed by lowering to an `if`/`else`-chain with a
`delivered` flag instead, the same first-match-wins shape without the
switch's uniqueness constraint. (2) the runner's own unconditional
`bytes`/`net/http`/`net/http/httptest`/`context` imports left three
"imported and not used" `go vet` failures across the db/queue-only
corner of the example set (a program with no `api` never calls
`doRequest`; a program with no job and no worker whose pipeline has
steps never calls `context.Background()`); fixed by gating each import
on the same conditions that gate its only call site.

One more divergence-ledger row, confirmed but not yet exercised by any
checked-in example (no reachable program declares a `Timestamp`
field): Go's `time.Time.MarshalJSON` formats RFC 3339 with a trimmed
fractional-seconds component — `.5`, `.123456`, `.1`, and no fractional
part at all when it's exactly zero — rather than a fixed-width
precision. Confirmed live with a standalone `encoding/json.Marshal`
check at v0.24 M9 (not hypothesized from reading the stdlib docs);
asserted-as-documented per Pillar 7's own table rather than built out
into a new flagship example, since nothing in the current corpus
reaches this code path yet.

Java reached the same narrow slice in 25UpdatePlan.md M9, structurally
identical to TypeScript's/Go's own shape (Java cannot vendor `ciac-
sim`'s Rust source either, so `sim/World.java`'s `World` class is a
hand-written restatement in the same position as Python's/
TypeScript's/Go's own), fakes the identical `db.insert`/broker
publish-consume pair, and refused by
`ciac_backend_java::unsupported_sim_capabilities` — computed from the
same shared `ciac_codegen::lower::scan`, `pub(crate) use`-re-exported
into this backend's own `lower` module exactly the way Rust's/TS's/
Go's already were. Java's own architecture diverges from the other
three narrow targets in *how* the world-guard reaches generated code,
not in what it fakes: rather than one shared state struct/object
threaded through every call site (Go's `*state.AppState`, Rust's
`&AppState`, TS's `AppState`), every Java class already holding a
`JdbcClient`/`Queue` field (Spring constructor injection, per-class,
since Java's own DI-container architecture never had one central
state object to begin with) gains a constructor-injected, nullable
`World` too, resolved through Spring's own `ObjectProvider<World>` —
`null` in production (nothing ever registers a `World` bean; the class
is a plain POJO, never `@Component`), the real fake only when
`SimRunner` manually registers one. `Queue.publishJson` becomes the
one choke point every `publish` call site shares (pipeline `publish`
steps *and* the `publish <Stream>(..)` HIR leaf both call the same
method, unchanged), so unlike the other three targets, neither
`_steps.java.j2` nor `lower.rs`'s own `publish` leaf needed to become
world-aware at all — only `db_insert_tail` and `transaction_stmt` did,
since `JdbcClient`'s own fluent SQL calls have no single shared choke
point to intercept the way a broker publish does. Java's own
production code gives `transaction {}` **real** atomicity
unconditionally too (`TransactionTemplate`, the same bar Go's/TS's own
Postgres branch holds), and — like Go/TypeScript — degrades to a
guarded no-op only under simulation; unlike Go's own `*sql.Tx`
typed-`nil`-then-skip shape, Java's own `transaction_stmt` wraps its
(framework-pre-indented) body in a `Runnable __txBody` once and shares
it unchanged between the world/real branches, rather than duplicating
it once per branch — reusing caller-supplied, already-indented lines
inside a second nesting level would need reflowing their baked-in
whitespace to stay `spotless:check`-clean, which sharing one `Runnable`
avoids needing at all.

`SimRunner.java` (`src/test/java/.../sim/SimRunner.java`) resolves the
milestone's own pre-registered "SimRunner packaging" open question
concretely: it lives under `src/test/java` specifically because
`MockMvc`/`spring-test` only ever sit on the `test` classpath, is
driven by a new `exec-maven-plugin` entry in the generated `pom.xml`
(`./mvnw test-compile` once, then `./mvnw exec:java
-Dexec.args=<scenario>` once per scenario — Maven's own "compile once,
run repeatedly" shape, mirroring `go build`+`go run`/`cargo build`+
`cargo run`/`npm run build`+`node`), and — the actual design decision —
never calls `SpringApplication.run` at all. Instead it builds a plain
`AnnotationConfigApplicationContext`, `.scan()`s every package below
the service root *except* the one holding `Application` itself (whose
conditional `@EnableScheduling`/`@EnableWebSocket` would otherwise
activate Spring's own background `@Scheduled` timer and WebSocket
machinery the moment the context refreshes — exactly the real
wall-clock/network side effects a scenario's own explicit `advance`/
`drain` steps exist to replace with deterministic, scripted calls),
registers one `World` bean manually, and drives requests through
Spring's own standalone `MockMvc` (`MockMvcBuilders.standaloneSetup`
over every `@RestController` bean gathered by
`ctx.getBeansWithAnnotation`, `@RestControllerAdvice` beans registered
the same way) — no embedded servlet container, no bound port, the same
"real routes, real handlers, no live listener" contract Rust's
`tower::ServiceExt::oneshot`/TS's `app.inject()`/Go's
`net/http/httptest` already hold. Confirmed live, not just reasoned
from the Spring docs: skipping `SpringApplication` means Spring Boot's
own startup banner/INFO logging never fires at all, and `MockMvc`'s own
one-time "Initializing Spring TestDispatcherServlet" log lines land
*before* the scenario runs rather than interleaving with it — so
`SimRunner`'s one-line `SimScenarioOutcome` JSON reply is always the
true last line of stdout with no `{ logger: false }`-equivalent
construction option needed, the same freedom Go's own `slog`-to-stderr
default already had.

Both checked-in scenarios reproduce their canonical outcomes
byte-exact against the real toolchain — `{"ProcessOrder":3}`/
`{"Reconcile":1}` for `sim/vertical-slice.ciac-sim.json`,
`{"ProcessOrder":100}`/`{"Reconcile":7}` for `sim/virtual-week.ciac-
sim.json` — and `examples/order-system.ciac` is refused with both
reasons named (`auth`, and the four unguarded verbs `cache.delete`/
`cache.set`/`db.count`/`db.update`), matching Rust's/TS's/Go's own
refusal shape exactly. One design choice proactively avoided rather
than found live: `SimRunner`'s own worker-dispatch table is an
if/else-if chain over the message subject from the start (never a
`switch`), specifically because Go's own M9 had already discovered
live that a Java-equivalent `switch` construct (Go's own, in that
case) rejects two `case` arms sharing one constant at compile time —
exactly the shape `examples/sim-broker-slice.ciac`'s two workers on
one stream produce — so Java's own runner never risked repeating that
defect class at all.
