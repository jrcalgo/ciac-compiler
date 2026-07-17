# CIaC v0.22-file — The Backend Factory: O(1) Marginal Cost per Target (implementation plan)

> Implementation plan, not a forecast. This file's number is a document
> number, not a release number — the same naming quirk 17UpdatePlan.md
> already disclosed (the v0.17 simulation arc shipped as compiler
> version 0.19.0 because v0.18 landed first). The release version this
> plan ships as is assigned at execution time from whatever the
> workspace version actually is when its final milestone lands.
>
> **Sequencing:** this plan executes BEFORE 23UpdatePlan.md
> (TypeScript), 24UpdatePlan.md (Go), and 25UpdatePlan.md (Java), and
> exists because of them. It is the answer to a direct question — "is
> the compiler abstracted finely enough that a new backend is mostly
> generation, not hardcoded wiring?" — whose honest answer, measured
> against the real codebase below, is: **largely yes at the model and
> deployment layers, no at three specific seams.** Those three seams
> are cheap to fix once and expensive to pay three more times. This
> plan fixes them once.
>
> This plan does not consume or reorder the v0.19–v0.21 forecast
> documents (outbox/idempotency, provenance, breadth). Those remain
> open forecasts; whether they execute before or after the backend arc
> is a separate scheduling decision this document does not make. One
> interaction is named now so it is not rediscovered later: v0.21's
> "full TypeScript backend" breadth candidate is superseded by
> 23UpdatePlan.md for the backend-generation portion; v0.21's
> selection machinery survives for its other candidates.
>
> **Confidence:** high. Every milestone here is a refactor of working,
> golden-snapshotted code under a byte-identical-output discipline —
> the same discipline the v0.9 compose dedup used, which is the direct
> precedent for this entire plan (that milestone moved per-backend
> compose emission into one shared assembler, proven golden-identical,
> parameterized by what turned out to be only seven values). The one
> genuinely uncertain unification (Python's statement-oriented
> lowering tail) carries a pre-agreed fallback scope so no milestone
> can stall on a judgment call.

## The gap this version closes

CIaC has two bundled backends (Python, Rust) at full feature parity,
and three more are planned (TypeScript, Go, Java — plans 23–25). The
question this plan answers with a measured audit: what does backend
number three actually cost, and how much of that cost is essential
versus accidental?

The distinction matters because the essential cost of a backend is
irreducible and *good*: templates are the product. A backend that
generates a real Fastify service or a real Spring Boot service must,
somewhere, contain the text of a real Fastify service and a real
Spring Boot service. Attempting to "abstract away" that text — a
cross-language mega-template, a language-neutral AST rendered five
ways — is the classic failure mode of multi-target generators, and
this plan names it in Explicit cuts precisely to forbid it. The
accidental cost is everything else: walker logic re-debugged per
backend, language trivia precomputed in shared structs, integration
switches scattered through the CLI. Accidental cost is what turns
"add a backend" from a bounded template-writing project into an
archaeology project, and it is what this plan eliminates.

### The audit: what a backend is made of today

Measured against the working tree at the end of the v0.17 arc
(compiler version 0.19.0). Every number below was produced by
counting the actual files, not estimated.

| Layer | Lines | Target-neutral? |
| --- | --- | --- |
| `ciac-codegen` shared crate | 10,750 | yes, with three exceptions below |
| — `model.rs` (context building) | 2,377 | mostly — 27 `pub py_*`/`pub rust_*` fields are not |
| — `compose.rs` | ~600 | yes — parameterized by `BackendComposeOpts` (7 string values) |
| — `k8s.rs`, `terraform.rs`, `openapi.rs`, `ts_client.rs`, `users.rs`, `ci.rs` | ~1,700 | yes, except `ci.rs`'s per-target test-step consts |
| — `migrations.rs`, `evolution.rs`, semantic model/diff, `regen.rs`, `backfill.rs` | ~3,800 | yes — SQL and semantics are engine-keyed, not language-keyed |
| — `system_tests.rs` | 1,064 | yes — system suites are Python/pytest regardless of target |
| `ciac-backend-python` | 374 (lib.rs) + 1,305 (lower.rs) + 31 templates | no (it *is* the target) |
| `ciac-backend-rust` | 509 (lib.rs) + 1,088 (lower.rs) + 37 templates | no |
| both backends' templates combined | 5,625 | no |
| scattered per-target match sites outside backend crates | ~25 across 6 files | **no — this is accidental cost** |

The template inventories, for concreteness (these are the essential
product, listed so plans 23–25 can size themselves against reality
rather than a guess):

- Python (31): `pyproject.toml`, `Dockerfile`, `README`,
  `system-README`, `main.py`, `workers_main.py`, `config.py`,
  `state.py`, `db.py`, `models.py`, `schemas.py`, `observability.py`,
  `auth.py`, `cache.py`, `queue.py`, `email.py`, `object_store.py`,
  `search.py`, `http_clients.py`, `api.py`, `resource_api.py`,
  `resource_store.py`, `channel.py`, `worker.py`, `consumer.py`,
  `job.py`, `service.py`, `logic.py`, `client.py`, `conftest.py`,
  `test_smoke.py`.
- Rust (37): the same concerns plus Rust-specific structure
  (`lib.rs`, module-mod files ×4, `error.rs`, split
  `route_api`/`route_resource`, `workers_bin`) and two artifacts
  Python lacks a file for (`scope_tests.rs`, `sim_runner.rs` — both
  v0.14–v0.17 arrivals whose Python analogs live in other mechanisms).

The verdict in one sentence: **a new backend today costs roughly
4,500–5,500 lines (≈500 lib.rs emission wiring + ≈1,200 lower.rs HIR
lowering + ≈2,800 templates + scattered edits), of which roughly
1,000–1,300 lines are accidental** — duplicated walker structure,
per-language fields in the shared model, and wiring edits sprinkled
across `commands.rs`/`ci.rs`/`vocab.rs`/`dev.rs` that ought to be one
registration line.

### What is already right (and must not be broken)

Credit where the architecture has already earned it — these are the
reasons three more backends are feasible at all, and every milestone
below carries a "golden-identical" proof obligation precisely so none
of this regresses:

1. **The `Backend` trait is honest and minimal.** `id`,
   `description`, `supports(Component)`,
   `generate(NormalizedIr, GenOptions) -> GeneratedProject`.
   Registration really is "one line here plus the backend crate
   itself" (`commands.rs::backends()` says so, and it's true — for
   the trait; the six-file integration tax this plan removes is
   everything the comment doesn't mention).
2. **`model.rs` does the heavy semantic lifting once.** Pipeline-step
   flattening (`StepCtx` trees with match arms), capability instance
   naming (`InstanceCtx` with default-instance compatibility),
   session/bind ordering, precomputed SQL fragments in bind order
   (`RecordCtx::select_cols`/`insert_placeholders`/
   `update_assignments`/`update_where` — the v0.13 M1 placeholder
   discipline that makes `$N`→`?` substitution safe), scope
   collection, call-target resolution (`CallTargetCtx` with env-var
   and default-URL conventions), record/enum shaping, worker/job
   config surfacing — a new backend consumes `SystemModel`/`Ctx` and
   never re-derives any of it. This is ~2,400 lines the new backends
   do NOT write, and it is the single strongest asset in the audit.
3. **`FieldTypeKind` (v0.10 M1) already proved the neutral-type
   pattern.** It exists because the Go external-backend spike was
   string-matching Python type names to infer field kinds; the fix
   was a language-neutral tagged enum carried alongside the host
   spellings. This plan extends that exact pattern to everything the
   27 `py_*`/`rust_*` fields still hardcode — it is an extension of a
   proven in-repo decision, not a novel bet.
4. **Deployment and tooling are fully shared.** compose (7-value
   parameterization via `BackendComposeOpts`), k8s manifests,
   Terraform modules, generated CI workflows, OpenAPI documents, the
   TS client, generated system tests, migration SQL and its
   sequencing/differ, the manifest/sidecar ownership discipline,
   evolution/semantic-diff/rename/backfill, AGENTS.md emission — a
   new backend gets all of it for zero lines. This is the majority of
   what "a CIaC backend" means to a user, and it is already O(1).
5. **`openapi.json` is target-neutral and embedded in every
   project** — which hands this plan its single best conformance
   oracle for free: the same program generated for two targets must
   produce byte-identical OpenAPI documents. That assertion (Pillar
   4) catches whole classes of route/schema/scope drift with one
   `assert_eq!`, and it costs nothing because the artifact already
   exists in every generated tree.
6. **The external-backend protocol exists but is not the parity
   path.** `CodegenRequest` ships the `SystemModel` over stdio, and
   the reference Go backend works — but typed handler bodies would
   cross the wire as HIR an external process has no lowering spec
   for, and simulation/scope-tests/validators/dev-loop have no
   protocol surface at all. Plans 23–25 therefore build internal
   crates, like Python and Rust. The protocol remains supported for
   third parties at its existing, documented, narrower capability
   level, and this plan touches it exactly once (Pillar 2's schema
   change), versioned and documented.

### The three seams that are accidentally expensive

**Seam 1 — per-language fields in the shared model.** `model.rs`
precomputes 27 `pub py_type`/`py_ann`/`py_args`/`py_expr`/
`py_module`/`py_getter`/`rust_type`/`rust_module`/`rust_state_field`/
`rust_db_field`/`rust_cache_field`/`rust_variant`/... fields, for
both languages, on every build, regardless of which target is being
generated. The pattern scales as O(languages × call sites): following
it for TS/Go/Java means roughly 40 more fields, five-way duplication
at every construction site in a 2,377-line file, and a `SystemModel`
wire schema (the external protocol payload!) that drags every
language's naming trivia along to every consumer. Worse, it puts
language-specific bugs in the shared crate: a wrong `py_ann` is a
Python bug that lives outside the Python backend, invisible to anyone
auditing `ciac-backend-python` in isolation. `FieldTypeKind` is the
proven alternative sitting immediately adjacent to the problem — the
neutral kind plus a per-backend rendering function.

**Seam 2 — the duplicated lowering walker.** `lower.rs` in each
backend is ~40% structural walker and ~60% genuinely per-language
leaf emission. The walker half comprises: the `Needs` scanner
(params/return/body walk; verb→capability flags; `db_get_tables`
model-import tracking; enum-name collection through
`field_access_enum_name`; since v0.17 M11, `unguarded_verbs` for
simulation coverage), statement dispatch
(`Let`/`Expr`/`Return`/`Fail`/`Publish`/`Transaction`), expression
dispatch with precedence and parenthesization
(`strip_outer_parens`'s depth-scan subtlety), enum-literal use-site
resolution (a bare `EnumLit` is un-lowerable without its comparison
or record-field context — both backends carry the identical
`unreachable!` and the identical recovery logic), float-literal
fidelity (`1.0` must not print as `1`), and block/tail shaping. The
walker halves have already drifted once — Python grew a
`Sink`/`lower_tail` split Rust doesn't need because Python statements
aren't expressions; Rust grew clone discipline
(`rust_field_value_expr`, the `__row` clone) Python doesn't need —
and each new backend re-writes and re-debugs the whole thing. Three
more backends means three more independent chances to reintroduce
the class of bug the E0382 episode (v0.17 M11) demonstrated lives
exactly in this layer, and three more places `unguarded_verbs` can
silently fall out of sync with what a backend's sim world actually
fakes — a correctness hole, not a style complaint, because that list
is what makes `ciac sim` refuse rather than mis-simulate.

**Seam 3 — registration is one line, integration is six files.** The
audit found ~25 per-target match sites outside the backend crates:

- `commands.rs`: the project marker constant (`pyproject.toml` vs
  `Cargo.toml`) in `find_project_dirs` call sites (verify, sim,
  system), `validate_python_project`/`validate_rust_project` (the
  uv/ruff/pytest vs cargo-check/-D-warnings/test sequences), the
  migrations directory mapping (`"python" => "app/migrations"`,
  `"rust" => "migrations"`) used by build/regen/rename-replay, and
  `sim_inner`'s target dispatch (`sim_drive_python` vs
  `sim_drive_rust` selection plus the rust-narrowness refusal).
- `ci.rs`: `PYTHON_TEST_STEPS`/`RUST_TEST_STEPS`/`GENERIC_TEST_STEPS`
  and the target match that selects them.
- `vocab.rs`: `const BOTH: &[&str] = &["python", "rust"]` — the
  provider-support table that feeds `ciac describe`, LSP hover, and
  the docs matrix names every target inline, in ~20 rows.
- `dev.rs`: the watch-loop's rebuild/restart command selection.
- `compose.rs`: `BackendComposeOpts` handed over at each backend's
  compose call site (already trait-shaped data — just not on the
  trait).

None of these is individually large; all of them are landmines for
backend number five, and every one is a fact *about a target* that
belongs behind the trait that names the target.

**Explicitly NOT a seam — the templates.** ~2,800 lines of templates
per backend is the essential product: they ARE the generated code,
reviewed by humans who know that language, snapshot-tested as bytes.
The goal is not fewer templates; it is that templates be the ONLY
large per-backend artifact, supported by a documented, schema-
published context contract so writing them is mechanical
transcription of "what should this file look like in language X,"
not reverse-engineering of `model.rs`.

### The host-field disposition preview

Pillar 2 deletes the per-language model fields; this table previews
the disposition of the significant ones so M2's full disposition
table (a milestone deliverable) starts from an audited draft rather
than a blank page. Three dispositions exist: **filter** (a pure
per-language rendering of neutral data), **composed filter** (a
per-language composition rule over structured neutral data the model
keeps), and **stays** (not actually per-language — mislabeled by
proximity):

| Field(s) | Where | Disposition | Neutral source |
| --- | --- | --- | --- |
| `FieldCtx::py_type`, `py_read_type` | field decls | filter | `FieldTypeKind` |
| `FieldCtx::rust_type` (and read variants) | field decls | filter | `FieldTypeKind` |
| `RecordCtx`/`TableCtx` class/snake names | everywhere | stays → gains `NameForms` | declared name |
| `HandlerRef::py_args` | route/worker call sites | composed filter | `bindings` + `db_session` (kept) |
| `HandlerRef::rust_db_field`, `rust_cache_field` | logic/service templates | filter | `BindingCtx` instance names |
| `BindingCtx::py_attr`, `rust_field` | handler ctors | filter | binding kind + instance `NameForms` |
| `ExtraImportCtx::py_module`, `py_getter` | route/worker imports | composed filter | ontology instance identity (kept) |
| `ExtraDepCtx::rust_module`, `rust_type`, `rust_state_field` | handler extras | filter | capability kind + instance names |
| `SessionCtx::dep`, `key_arg`, `WorkerCtx::session_with` | Python session plumbing | composed filter | `db_sessions` (kept) |
| `ArmCtx::rust_variant` | match arms | filter | enum name + variant label (kept as neutral `label`) |
| `JobCtx::cron_crate_schedule` | Rust job template | filter (rust-owned) | source `schedule` (kept) — confirmed Rust-only by plans 23/24 |
| `InstanceCtx::url_field`/env naming | config templates | stays | env-var conventions are cross-target contract, not language trivia |
| `ApiCtx::route`, methods, scope | routes | stays | wire contract |
| `HandlerRef::handler_package` | ownership split | stays | shared seeded/compiler-owned discipline |
| precomputed SQL fragments (`select_cols`, `insert_placeholders`, `update_assignments`, `update_where`) | db templates | stays | SQL is a shared artifact by design (v0.13 M1); `sqlph` already parameterizes the placeholder style |

The last three rows are the audit's second-order finding worth
stating: a meaningful fraction of what *looks* per-language in
`model.rs` is actually cross-target contract (env names, routes, SQL
text) that must NOT move into backends — moving it would let targets
drift on exactly the surfaces the conformance harness exists to pin.
The disposition table is therefore as much a fence as a demolition
list.

### Cost model this plan commits to

| | Today | After this plan |
| --- | --- | --- |
| lib.rs emission wiring | ~500 lines | ~350 (shared emission-plan helper for the standard file set) |
| lower.rs | ~1,200 lines (walker + leaves) | ~450–600 (leaves only, via `HostSyntax`) |
| templates | ~2,800 lines | ~2,800 (unchanged — essential) |
| model.rs growth per language | ~+9 fields × N construction sites | 0 (backend-owned filters) |
| edits outside the crate | ~6 files, ~25 sites | 1 line (registry) — everything else supplied by `TargetInfo` |
| conformance proof | ad hoc per arc | one harness: examples × targets × (validate, golden, OpenAPI-equality, topology-equality) |

These numbers are commitments, not aspirations: M6's retrospective
publishes the measured actuals, and 23UpdatePlan.md's M5 checkpoint
grades the factory against them with "pause and amend this plan" as
an explicitly valid outcome.

## Pillar 1 — `TargetInfo`: the whole integration surface as data on the trait

The `Backend` trait gains one method, `fn target_info(&self) ->
&'static TargetInfo`, returning a struct that carries every fact the
CLI, CI generator, compose assembler, dev loop, and vocab tables
currently hardcode per target:

```rust
pub struct TargetInfo {
    /// Project marker file that identifies a generated project root,
    /// e.g. "pyproject.toml", "Cargo.toml", "package.json", "go.mod",
    /// "pom.xml". Consumed by find_project_dirs everywhere.
    pub project_marker: &'static str,
    /// Where CIaC-owned migration SQL lives inside the project, e.g.
    /// "app/migrations", "migrations",
    /// "src/main/resources/db/migration".
    pub migrations_dir: &'static str,
    /// Per-target migration filename mapping. Identity for every
    /// current target; Java/Flyway (25UpdatePlan.md) maps
    /// `0001_slug.sql` to `V0001__slug.sql`. The regen/rename-replay
    /// machinery resolves ownership through this mapping so renames
    /// of migration-bearing programs stay transactional.
    pub migration_filename: fn(seq: u32, slug: &str) -> String,
    /// Commands `ciac verify`/`build` run to validate a generated
    /// project, in order, each with its env (e.g. RUSTFLAGS) and a
    /// human-readable purpose string for error messages.
    pub validate: &'static [ValidateStep],
    /// The literal CI test-step YAML `ci.rs` embeds for this target
    /// (replaces PYTHON_TEST_STEPS/RUST_TEST_STEPS/GENERIC).
    pub ci_test_steps: &'static str,
    /// Compose parameterization — the existing BackendComposeOpts,
    /// moved onto the trait unchanged (db_url_scheme,
    /// workers_command, mysql_url_scheme, sqlite_url_prefix/suffix,
    /// data dir, and friends).
    pub compose: BackendComposeOpts,
    /// Commands `ciac dev` uses to rebuild/restart on change.
    pub dev: DevCommands,
    /// Simulation support: Full (Python), Narrow { unsupported:
    /// fn(&NormalizedIr) -> Vec<String> } (Rust's
    /// unsupported_sim_capabilities, promoted from a free function),
    /// or None { reason: &'static str }. `ciac sim`'s dispatch reads
    /// this instead of matching target-name strings.
    pub sim: SimSupport,
}
```

Consumption changes, file by file:

- `commands.rs`: `find_project_dirs(out, target.project_marker)`;
  one `validate_generated(project, target)` loop replacing the two
  named validators; the migrations mapping and rename-replay path
  resolution read `migrations_dir`/`migration_filename`; `sim_inner`
  matches on `SimSupport` (the Python arm keeps the bounded child
  protocol; `Narrow` runs the generated-runner drive built in v0.17
  M11; `None` refuses with the stated reason — behavior identical,
  provenance moved).
- `ci.rs`: `target_info.ci_test_steps` replaces the const match; the
  `GENERIC_TEST_STEPS` fallback survives for external-protocol
  backends, which have no `TargetInfo`.
- `dev.rs`: rebuild/restart from `DevCommands`.
- `vocab.rs`: the support tables stop naming targets. Each provider
  row keeps only its capability/provider identity and docs; support
  is derived at render time by asking every registered backend
  `supports(component)` — so `ciac describe`, LSP hover, and the
  docs matrix can never again disagree with the code. The
  `docs/language.md` provider table gains generated-block markers and
  a test that fails when the committed table drifts from the derived
  one (same checked-in-but-machine-verified pattern as the codegen
  schema from v0.10 M2).

The three supporting types, defined now so their scope is fixed
before implementation:

```rust
pub struct ValidateStep {
    /// e.g. "uv" / "cargo" / "npm"
    pub program: &'static str,
    pub args: &'static [&'static str],
    /// e.g. [("RUSTFLAGS", "-D warnings")], [("CGO_ENABLED", "0")]
    pub env: &'static [(&'static str, &'static str)],
    /// One clause for the error message: "type-checks", "lints",
    /// "unit tests pass" — so a failure names what broke, not just
    /// which process exited nonzero.
    pub purpose: &'static str,
}

pub struct DevCommands {
    /// Re-run after a successful regeneration (e.g. `npm run build`,
    /// `go build ./...`, `./mvnw -q -B -DskipTests package`).
    /// Empty for targets whose restart implies rebuild.
    pub rebuild: &'static [ValidateStep],
    /// Whether `ciac dev` restarts processes or delegates to the
    /// target's own watcher; today both targets are restart-style,
    /// and this field exists so a future watcher-style target
    /// doesn't force a dev.rs special case.
    pub restart: RestartStyle,
}

pub enum SimSupport {
    Full,
    Narrow { unsupported: fn(&NormalizedIr) -> Vec<String> },
    None { reason: &'static str },
}
```

Deliberate scope limits, disclosed now: `validate` steps run through
the same `run_in` plumbing that exists today — this plan does not
build a sandboxed runner or a toolchain installer; if `npm`/`go`/
`mvn` are missing, verify fails with the same honest process error
`uv` already produces, and the docs say which toolchains each target
needs. `SimSupport` carries no promise that a future backend must
implement simulation — `None` with a reason is a permanently valid
state, per the v0.17 discipline that a refusal must be clean and
specific, never a silent no-op.

**The grep fence.** M1's exit checklist includes an audit test, not
just an audit: a repo test greps for `"python"`/`"rust"` target-name
string literals outside the backend crates and the registry, and
every surviving site must carry a `// target-literal-ok:` comment
naming its justification (the known survivors: the Python-runner
`sim/pyrunner` embedding, which is target-honest by nature;
docs/prose; test fixtures that exercise specific targets). New
unjustified sites fail the test — the fence that keeps seam 3 from
regrowing while three more backends land.

## Pillar 2 — Neutral naming and types: retiring the `py_*`/`rust_*` fields

Two moves, executed strictly under byte-identical goldens.

**Move 1: `NameForms` everywhere a name crosses the model boundary.**
Every context struct that today carries `snake` plus host-specific
spellings gains one neutral form set:

```rust
pub struct NameForms {
    pub original: String,   // As declared: "PlaceOrderApi"
    pub snake: String,      // place_order_api
    pub pascal: String,     // PlaceOrderApi
    pub camel: String,      // placeOrderApi
    pub kebab: String,      // place-order-api
    pub screaming: String,  // PLACE_ORDER_API
}
```

computed once in `model.rs` via the existing heck dependency.
Templates and backends pick the casing their language wants
(`{{ api.name.camel }}`); nothing per-language remains precomputed in
the shared crate. Existing `snake`/`class_name` fields survive
through a deprecation window as aliases so both template suites
migrate incrementally under goldens, then are removed in the same
milestone's final commit.

**Move 2: backend-owned minijinja filters replace host-type
fields.** Each backend registers filters at environment
construction — `py_type(field)`, `py_ann(field)`, `rust_type(field)`,
and later `ts_type`, `go_type`, `java_type` — implemented in the
backend crate against `FieldTypeKind` and `NameForms`. A worked
example of the migration shape, because this is the pattern repeated
~27 times:

```text
before (model.rs, shared):
    FieldCtx { py_type: "datetime", rust_type: "chrono::DateTime<chrono::Utc>", ... }
    template: {{ field.py_type }}

after (backend crate):
    fn py_type(kind: &FieldTypeKind) -> String { match kind { Timestamp => "datetime", ... } }
    env.add_filter("py_type", ...);
    template: {{ field | py_type }}
```

The 27 fields are deleted from `model.rs` only after both backends'
full golden suites are byte-identical through the filter path. The
template environment helper in `ciac-codegen::template` already
installs shared filters (`sqlph`, `pascal_case`); this extends the
same mechanism — no new machinery, no new template language.

Two subtleties called out so they are estimated, not discovered:

- `py_args` (the precomputed constructor-argument string for handler
  invocation) and `session_with` (Python's pre-joined `async with`
  items) are not type renames — they are small per-language
  *composition* rules. They move into the Python backend as filters
  taking the structured data (`bindings`, `db_sessions`) they are
  composed from, which the model already carries. Same for Rust's
  `rust_variant` on match arms (composed from enum name + variant —
  both neutral facts).
- `HandlerRef::handler_package` (`services` vs `logic`) is NOT
  per-language — it is the ownership split, shared by all targets,
  and stays in the model. The audit's field-by-field disposition
  table (produced in M2, committed with the milestone) classifies
  every field explicitly so nothing is moved or kept by accident.

**Wire-protocol consequence, handled honestly:** `SystemModel` is the
external-backend protocol's payload, so deleting fields is a breaking
protocol change. `protocol_version` increments; the checked-in schema
(`ciac codegen-schema`, v0.10 M2) regenerates;
`docs/external-backends.md` documents the migration (external
backends were already directed to `FieldTypeKind` when v0.10 M1
created it for exactly this reason, and the reference Go backend
already consumes it — its live proof re-runs in this milestone). The
protocol gets smaller and more stable, not merely different: after
this move, adding bundled backend number six changes the wire schema
not at all.

## Pillar 3 — `lower_core`: one walker, per-language leaves

A new shared module `ciac-codegen::lower` owns the HIR traversal both
backends currently duplicate. Its three parts:

**Part 1 — the `Needs` scanner, unified.** One implementation of the
params/return/body walk producing the shared `Needs` struct: verb→
capability flags (db/cache/queue/uuid/datetime), `tables`,
`db_get_tables` (the verbs that spell a model type and therefore
drive imports — the doc comment explaining WHY only get/query need
the model import moves here, once), `records`, `enums` (use-site
resolution), and `unguarded_verbs`. That last one is the correctness
argument for this pillar in one line: the list that makes `ciac sim`
refuse un-fakeable programs is currently maintained per backend by
hand, and a backend that forgets to push a verb silently mis-scopes
its own simulation refusals. After this pillar it is computed once,
and a backend's `SimSupport::Narrow` coverage function consumes it.

**Part 2 — dispatch with a documented contract.** Statement dispatch
(`Let`/`Expr`/`Return`/`Fail`/`Publish`/`Transaction`) and expression
dispatch, owning centrally: precedence/parenthesization (every
composite wraps, `strip_outer_parens` unwraps exactly the outermost
redundant pair for condition positions — the depth-scan
implementation moves verbatim), enum-literal use-site resolution
(`field_access_enum_name` recovery from comparison LHS / record-field
/ match-scrutinee contexts, with the shared `unreachable!` for a bare
literal), float-literal fidelity (the must-contain-a-dot rule), and
statement-vs-expression orientation as an explicit mode:
`ExpressionOriented` targets (Rust) lower `if`/`match`/`db.insert` as
expressions; `StatementOriented` targets (Python — and, plans 24/25
note, Go and Java) get the `Sink`/`lower_tail` shaping as a shared
code path with the target supplying only assignment/return syntax.

**Part 3 — the `HostSyntax` trait of leaf constructors.** Roughly 30
methods, the complete per-language surface:

```text
literals: int, float (pre-fidelity-checked), str, bool
locals & access: local(name), field_access(base, field),
    index(base, key)
records: record_cons(type, fields, base) — with a value-semantics
    hook (CloneNeeded/None) that is where the E0382 discipline
    lives for Rust and a documented no-op for GC targets
operators: binary(op, lhs, rhs, operand_types) — string-concat
    special case routed here; unary(op, expr)
control: if_expr / match_expr (ExpressionOriented) or
    if_stmt / match_stmt sink shaping (StatementOriented)
verbs, one method each: db_insert/get/update/delete/query/count/
    delete_where, cache_get/set/delete, object_store_put/get/
    delete/list, email_send, search_index/query, http_call
builtins: uuid_new, timestamp_now
statements: let_binding, return_, fail(error, args),
    publish(subject, value), transaction(body) — transaction is a
    leaf because atomicity strategy is genuinely per-target
    (Python session vs Rust's disclosed gap vs the new targets'
    real transactions)
```

Both existing backends port onto it **golden-byte-identically** — the
port is done when `git diff` on every generated-project snapshot is
empty, the same acceptance the v0.9 compose dedup used. The measured
acceptance criterion: the Rust backend's `lower.rs` shrinks to leaf
implementations plus clone discipline; a test-only "identity"
`HostSyntax` (emitting s-expression-ish pseudo-code) demonstrates and
documents that a new language implements ~30 leaf methods against a
frozen contract, and that identity backend's output is itself
snapshot-tested so the *contract* has goldens, not just its
consumers.

**A worked lowering, both modes, to fix what "shared" means.** Take
one HIR fragment — the vertical-slice worker handler's body shape:

```text
let processed = db.insert(ProcessedOrders,
    ProcessedOrder { id: Uuid.new(), order_id: order.id });
return order;
```

What the shared dispatch owns, identically for every target: the
statement sequence (`Let` then `Return`); that the `Let`'s value is
a `VerbCall(DbInsert)` whose argument is a `RecordCons` with two
fields; that `order.id` is a `FieldAccess` used as a record-field
value (the value-semantics hook fires here — this is where Rust's
clone discipline attaches and where GC targets no-op); that
`Uuid.new()` is `Builtin::UuidNew`; that the scanner marks
`needs.db`, the `ProcessedOrders` table, the `ProcessedOrder`
record, `needs.uuid`, and nothing in `unguarded_verbs`. What each
`HostSyntax` supplies, and nothing more:

```text
ExpressionOriented (Rust leaf output):
  let processed = {
      let __row = ProcessedOrder { id: uuid::Uuid::new_v4()
          .to_string(), order_id: order.id.clone() };
      /* world-guard */ sqlx::query("INSERT INTO …")…; __row
  };
  return Ok(order);

StatementOriented (Python leaf output, via the shared sink shaping):
  __row = ProcessedOrder(id=str(uuid.uuid4()), order_id=order.id)
  session.add(models.ProcessedOrders(**…)); …
  processed = __row
  return order
```

The block-expression vs sink-assignment difference is the MODE; the
insert SQL, the clone/no-clone decision point, the builtin
spellings, and the record-construction syntax are LEAVES; everything
else in that lowering — and it is most of it — is walker. That
proportion (most of it is walker) is the pillar's whole argument,
and it is checkable: the M3 LOC metrics record exactly how much of
each backend's 1,100–1,300 lines survives as leaves.

Known risk, stated with its fallback: Python's tail shaping is the
hardest unification. If a faithful shared `StatementOriented` mode
degrades either backend's output readability (goldens make any drift
visible immediately), the pre-agreed fallback is sharing Part 1 and
Part 2's dispatch skeleton while keeping per-backend tail shaping —
that still removes the scanner/dispatch duplication and ALL of the
sim-coverage drift risk, which are the correctness-bearing parts.
The fallback decision, if taken, is recorded in this file's milestone
notes exactly as v0.17's M11 recorded its narrowed slice.

**Amendment procedure, frozen now** (plans 24/25 reference it): a
`HostSyntax` contract change lands as its own commit containing (1)
the trait change, (2) the identity-syntax golden update showing
exactly what changed, (3) byte-identical goldens for every existing
target, and only THEN (4) the new target's consumption in a
subsequent commit. An amendment that cannot keep existing targets
byte-identical is a redesign, not an amendment, and goes back
through this plan's Pillar 3 with a milestone note.

## Pillar 4 — The conformance harness: parity as a test, not a claim

One new integration suite, `tests/tests/conformance.rs`, that plans
23–25 inherit as their definition of done. Five assertions per
(example × registered target that `check_support` accepts):

1. **Matrix generation:** generate in-memory; any panic or error is a
   conformance failure with the example/target named.
2. **Golden snapshots:** the existing per-target snapshots, unchanged
   in format, now enumerated by the registry so a new backend's
   goldens appear without editing the test.
3. **Cross-target OpenAPI equality:** every target's `openapi.json`
   for the same program must be byte-identical. Routes, methods,
   payload schemas, scopes — one `assert_eq!` catches drift in any
   of them, in any backend, forever.
4. **Topology equality:** subjects, queue groups, worker
   concurrency/retry bounds, cron schedules, table names, and
   migration SQL extracted from each generated tree must match across
   targets. Migration SQL is shared code so this is nearly free — the
   assertion exists to catch a backend accidentally post-processing
   shared artifacts (the Java/Flyway renaming in 25UpdatePlan.md is
   exactly the kind of transformation this keeps honest: the mapping
   is allowed, silent content drift is not).
5. **Validation:** each generated project runs its `TargetInfo`
   validators — locally where toolchains exist, CI otherwise, with
   the repo's standing Docker-delegation honesty (the harness prints
   which legs ran where; a leg that can't run locally is reported as
   delegated, never as passed).

Plus one CLI artifact: `ciac targets --json` renders the registry —
snapshot-tested and consumed by the docs build. The shape, fixed
now so downstream consumers (docs build, plans 23–25's checklists,
any agent calling MCP `describe`) can code against it before the
implementation exists:

```json
{
  "targets_version": 1,
  "targets": [
    {
      "id": "rust",
      "description": "Rust project using Axum, SQLx, redis, and async-nats/rdkafka",
      "kind": "internal",
      "project_marker": "Cargo.toml",
      "validate": [
        {"program": "cargo", "purpose": "type-checks (deny warnings)"},
        {"program": "cargo", "purpose": "unit and generated tests pass"}
      ],
      "sim": {"level": "narrow"},
      "capabilities": {
        "db": ["Postgres", "MySQL", "SQLite"],
        "queue": ["NATS", "Kafka"],
        "...": "derived from supports(), never hand-written"
      }
    },
    {"id": "go", "kind": "external", "...": "external-protocol targets appear with kind external and generic validation"}
  ]
}
```

The assertion inventory, numbered so milestone checklists and CI
failures can cite them:

| # | Assertion | Catches |
| --- | --- | --- |
| C1 | generate() succeeds for every supported (example, target) | emission regressions, context panics |
| C2 | goldens match | any output drift, reviewed not blind |
| C3 | openapi.json byte-equal across targets per example | route/method/schema/scope drift |
| C4 | topology extract equal (subjects, groups, retries, schedules, tables, migration content) | broker/schedule/schema drift; post-processing of shared artifacts |
| C5 | validators pass (local or delegated-with-report) | generated code that doesn't compile/lint/test |
| C6 | support matrix rows have their ratchet proofs | overclaimed capability tables |
| C7 | boundary decode/encode suite (added by plan 24, run everywhere) | absent/null/zero and empty-list wire divergence | The fidelity-ratchet rule from 17UpdatePlan.md extends
per target: a backend's capability row is not "done" in the support
matrix until its `--system` proof exists in CI and (where
`SimSupport` is not `None`) its sim-vs-real comparison row exists.
The harness is where that graduation is enforced mechanically —
a row without its proofs fails the matrix test, which is what
"the table cannot overclaim" means in practice.

## Pillar 5 — The template contract and the author path

**Context schema published.** `SystemModel`/`Ctx` already derive
`JsonSchema`; the `ciac codegen-schema` output becomes the documented
template contract, cross-linked from a rewritten
`docs/backend-authoring.md` covering internal-crate authoring (the
current doc covers only the external protocol). The doc's spine is a
table: every context struct, every field, which templates consume it
in the two reference backends — generated from source annotations
where cheap, hand-maintained with a drift test where not.

**Emission-plan helper.** The standard file set every backend emits
(build file, Dockerfile, README, config, state, observability,
per-api routes, per-worker/job/consumer modules, logic/services
split, schemas/models, OpenAPI embed, compose/k8s handoff, gitignore/
dockerignore) becomes a declarative table in the backend crate:

```rust
const EMIT: &[Emit] = &[
    Emit::always("Cargo.toml", "Cargo.toml.j2"),
    Emit::when(Cond::HasQueue, "src/queue.rs", "queue.rs.j2"),
    Emit::per_api("src/routes/{snake}.rs", "route_api.rs.j2"),
    // ...
];
```

driving a shared loop — replacing ~150 lines of hand-rolled
`project.add_file` sequencing per backend and making "which files
exist under which condition" diffable data. Non-template emissions
(vendored sim sources via `include_str!`, the OpenAPI serialization,
compose handoff) get `Emit::custom(fn)` so nothing is forced through
a template that isn't one. Both backends port under goldens.

**Skeleton.** `backends/skeleton-internal/` — a compilable, gated
do-nothing backend crate demonstrating the trait, `TargetInfo`,
filters, `HostSyntax` (identity impl), the emission table, and the
conformance hookup, kept green by the workspace build and exercised
by one registry test. Plans 23–25 start by copying it; its README is
the quick-start half of the authoring guide.

## Pillar 6 — What backend number six costs: the authoring walkthrough

The factory's deliverable is a *path*, and the honest way to specify
a path is to walk it. This is the documented sequence a new internal
backend follows after this plan ships — it is also the table of
contents of the rewritten `docs/backend-authoring.md`, and plans
23–25's M1–M4 are instances of it:

1. **Copy the skeleton** (`backends/skeleton-internal` →
   `crates/ciac-backend-<lang>`); rename; add the crate to the
   workspace; add the one registry line in
   `commands.rs::backends()`. Everything compiles; `ciac targets`
   lists the new id with its gated (empty) support set. Elapsed
   cost: minutes.
2. **Fill in `TargetInfo`**: marker, migrations dir + filename
   mapping, validate steps, CI steps, compose values, dev commands,
   `SimSupport::None { reason }`. Every CLI surface — verify's
   validators, sim's refusal, dev's loop, the generated CI, the
   compose assembler, the docs matrix — now handles the target with
   zero further edits. Cost: one struct literal.
3. **Write the build-file, Dockerfile, README, config, state,
   observability, health templates** and list them in the emission
   table. `supports()` admits `Api` + core components. The
   conformance harness picks the target up automatically (registry-
   driven) and starts asserting OpenAPI equality from the first
   route. Cost: the essential template work, now the *first* real
   work instead of the fourth.
4. **Register the type filters** (`<lang>_type` over
   `FieldTypeKind`) and write schemas/models/routes templates.
5. **Implement `HostSyntax`** — ~30 leaf methods against the frozen,
   golden-snapshotted contract, with the identity impl as the
   reference and both real backends as worked examples. The walker,
   scanner, precedence, enum resolution, orientation shaping, and
   `unguarded_verbs` all arrive for free.
6. **Fill the capability templates** (queue/cache/auth/ontology
   wrappers, worker/job/channel modules) in whatever milestone order
   the language plan sets, un-gating `supports()` as each lands.
7. **Optionally implement the sim slice**: a world restatement + a
   generated runner + flipping `SimSupport` to `Narrow` — with the
   coverage function supplied by the shared scanner, so the refusal
   list is correct by construction.

What the author never does after this plan: touch `model.rs`, touch
`commands.rs`/`ci.rs`/`dev.rs`/`vocab.rs` beyond the registry line,
re-implement a walker, re-derive SQL or bind order, write compose/
k8s/terraform/OpenAPI/system-test/migration logic, or hand-maintain
a docs support row. Each "never" above is one of the audit's
accidental-cost items, and the walkthrough is the proof shape the
M6 retrospective fills with measured numbers.

## Implementation map

The change inventory, file by file, so execution starts from a map
rather than a search (line counts are the audit's, for scale):

| File | Change |
| --- | --- |
| `ciac-codegen/src/lib.rs` | `TargetInfo`, `ValidateStep`, `DevCommands`, `SimSupport`, `NameForms` definitions; `Backend::target_info()` |
| `ciac-codegen/src/model.rs` (2,377) | `NameForms` adoption; host-field deletion per the disposition table; no semantic changes |
| `ciac-codegen/src/lower.rs` (new) | scanner + dispatch + orientation shaping + `HostSyntax` trait + identity impl |
| `ciac-codegen/src/template.rs` | filter-registration hook for backend-owned filters |
| `ciac-codegen/src/emit.rs` (new) | the declarative emission-plan loop |
| `ciac-codegen/src/compose.rs` | `BackendComposeOpts` sourced from `TargetInfo` (struct unchanged) |
| `ciac-codegen/src/ci.rs` | per-target consts → `target_info.ci_test_steps`; GENERIC fallback kept for external backends |
| `ciac-codegen/src/protocol.rs` | `protocol_version` bump; schema regen |
| `ciac-backend-python/src/lower.rs` (1,305) | reduced to `HostSyntax` leaves + filters (`py_type`, `py_args`, session composition) |
| `ciac-backend-python/src/lib.rs` (374) | emission table; filter registration; `TargetInfo` |
| `ciac-backend-rust/src/lower.rs` (1,088) | reduced to leaves + clone discipline + filters; `unsupported_sim_capabilities` re-based on the shared scanner |
| `ciac-backend-rust/src/lib.rs` (509) | emission table (including the `Emit::custom` vendored-sim writes); `TargetInfo` with `SimSupport::Narrow` |
| `ciac/src/commands.rs` | markers/validators/migrations-mapping/sim-dispatch via registry; grep-fence exceptions annotated |
| `ciac/src/vocab.rs` | provider support derived from registry; docs drift test |
| `ciac/src/dev.rs` | `DevCommands` consumption |
| `ciac/src/main.rs` | `ciac targets --json` |
| `tests/tests/conformance.rs` (new) | the Pillar 4 harness |
| `backends/skeleton-internal/` (new) | the Pillar 5 skeleton |
| `docs/backend-authoring.md` | rewritten around Pillar 6's walkthrough |
| `docs/external-backends.md` | protocol v-bump migration note |
| templates (both backends) | host-field → filter call sites; zero output change |

## Milestones

1. **M1 — Audit freeze + `TargetInfo` + registry consumption.** Land
   the struct and both backends' instances; port `commands.rs`
   (markers, validators, migrations mapping and rename-replay path
   resolution, sim dispatch), `ci.rs`, `dev.rs`, `vocab.rs` (derived
   support tables + docs drift test) to read the registry. Proof:
   zero behavior change — full workspace suite, all goldens, and a
   scripted session byte-comparing `ciac verify` (both targets, one
   example each), `ciac dev --no-docker` transcript, `ciac describe`,
   and the generated CI YAML for one example against pre-refactor
   output. The v0.17 sim CLI proofs re-run (dispatch moved).
2. **M2 — `NameForms` + backend filters; delete `py_*`/`rust_*`.**
   The field-disposition table (every host field: filter / composed
   filter / stays-neutral, with reasons) commits first; then
   three-phase under goldens (add filters → migrate templates in
   reviewable chunks → delete fields + aliases). `protocol_version`
   bump, schema regeneration, `docs/external-backends.md` migration
   note, reference Go backend updated and its `ciac build --target
   go` live proof re-run. This is the one milestone that changes any
   observable surface (the wire schema), and it is versioned.
3. **M3 — `lower_core` + `HostSyntax`; port both backends.**
   Scanner first (both backends consume it; sim proofs re-run since
   `unguarded_verbs` moved), then dispatch + leaves per backend,
   golden-byte-identical or the disclosed fallback scope with the
   decision recorded. The identity-syntax snapshot lands as the
   contract's own golden. Acceptance metric recorded: leaf-only LOC
   for the identity target, and before/after LOC for both real
   backends.
4. **M4 — Conformance harness + `ciac targets --json`.** Matrix,
   OpenAPI/topology equality (run against python×rust immediately —
   any existing divergence found becomes a fix-or-disclose item in
   this milestone, not a surprise in plan 23), validator hookup with
   delegation reporting, ratchet enforcement wired to the matrix
   test, CI job added.
5. **M5 — Emission-plan helper + skeleton backend + authoring
   guide.** Both backends ported to the declarative emission table
   (golden-identical); `backends/skeleton-internal` compiles and is
   exercised; `docs/backend-authoring.md` rewritten with the measured
   what-you-must-write inventory (the cost-model table, updated with
   M1–M5 actuals).
6. **M6 — Version, docs reconciliation, retrospective.** Workspace
   version bump (number assigned at execution); README/docs support
   tables switched to the generated source of truth; whole-version
   analysis: the before/after cost model with real numbers, every
   deviation from this plan disclosed in place, and the explicit
   handoff sentence to 23UpdatePlan.md — whose M5 checkpoint grades
   these numbers against a real third backend.

### Per-milestone exit checklists

Because this plan is a refactor whose only observable deliverable is
"nothing changed," each milestone's exit is a mechanical checklist
rather than a judgment call — recorded here so execution can't
soften them:

- **M1 exits when:** all six integration files compile with zero
  target-name matches outside annotated exceptions; the grep-fence
  test passes; the byte-comparison session (verify ×2 targets, dev
  --no-docker transcript, describe output, generated CI YAML) is
  diff-empty against pre-refactor captures; both sim CLI proofs and
  the refusal case reproduce; full workspace suite green.
- **M2 exits when:** the disposition table is committed; zero
  `py_*`/`rust_*` fields remain in `model.rs`; every golden is
  byte-identical; `protocol_version` bumped with schema + docs +
  reference-backend proof; `cargo test --workspace` green.
- **M3 exits when:** both backends' `lower.rs` contain no
  scan/dispatch logic (or the fallback scope is recorded with its
  reason); goldens byte-identical; the identity-syntax golden
  exists; the sim proofs reproduce (scanner moved); the LOC metrics
  are recorded in this file.
- **M4 exits when:** the harness runs in CI; python×rust OpenAPI and
  topology equality pass on all 26 examples (or divergences found
  are fixed/disclosed as M4 items); `ciac targets --json` snapshot
  exists; the docs matrix drift test passes.
- **M5 exits when:** both backends emit through the table; goldens
  byte-identical; the skeleton compiles, registers under a test
  flag, and its gated no-op passes the harness; the authoring guide
  matches the walkthrough pillar section by section.
- **M6 exits when:** version bumped everywhere the release checklist
  names; the retrospective's cost table has a measured number in
  every cell; push green.

## Execution order and dependencies

The milestones are sequential but their internals parallelize
differently, recorded so execution doesn't serialize what needn't
be: M1's six file-ports are independent of each other once
`TargetInfo` lands (each is its own commit with its own byte-compare
leg). M2's filter migration parallelizes per template family
(schemas, routes, workers) because goldens isolate each. M3 is the
one strictly serial milestone (scanner → dispatch → Python port →
Rust port — each step's goldens gate the next). M4 depends on M1
(registry) but not M2/M3, and may run early if a divergence check
between the existing backends is wanted sooner; its C3/C4
assertions are meaningful the moment two targets exist. M5 depends
on all prior. Nothing in this plan blocks concurrent work elsewhere
in the repo except edits to the six integration files and the two
`lower.rs` files during their respective milestones.

## Decision log (questions this plan already answered)

Recorded in FAQ form because these are the questions review will
ask, and the answers are load-bearing:

- *Why internal crates for new backends instead of growing the
  external protocol?* Typed-handler lowering, validators, scope
  tests, dev loop, and simulation have no wire surface; specifying
  them is unbounded. The protocol remains for third parties at its
  honest level.
- *Why not generate ASTs instead of templates?* Per-language ASTs
  triple the per-target cost for output-formatting fidelity CIaC
  gets from goldens already. Templates are reviewable by language
  experts who don't know the compiler.
- *Why minijinja filters instead of a `TypeRenderer` trait?* The
  templates are where types are spelled; filters put the rendering
  at the use site with zero indirection, and the mechanism already
  exists (`sqlph`).
- *Why is the emission plan data, not code?* So "which files exist
  under which condition" is diffable and the standard set is
  enforced structurally — a new backend physically cannot forget
  AGENTS.md or the OpenAPI embed without deleting a table row that
  review will see.
- *Why does `SimSupport` live on `TargetInfo` instead of a separate
  trait?* One integration surface. Splitting trait surfaces per
  concern re-creates seam 3 at the trait level.
- *Why byte-identical instead of semantically-equivalent goldens?*
  Because "semantically equivalent" requires a judge, and judges
  drift. Bytes don't.

## Verification strategy

- Byte-identical goldens are the invariant for M2/M3/M5 — any diff is
  reviewed as a deliberate output change or reverted, never accepted
  as incidental churn. `cargo insta` review discipline as practiced
  all arc: diffs read before acceptance, acceptance reasons in the
  commit message.
- `cargo fmt --check`, `clippy -D warnings`, `cargo test --workspace`
  per milestone; full example sweep (`ciac verify`, both targets, all
  26 examples) at M3 and M5; the v0.17 sim CLI proofs (both
  scenarios, both targets, plus the order-system refusal case) re-run
  at M1 and M3.
- External protocol: the Go reference backend's live proof re-runs at
  M2 — the one intentional observable change, versioned and
  documented, never silent.
- No Docker requirement anywhere in this plan: every proof is
  static/golden/local-toolchain, which is precisely why this
  refactor-heavy plan is safe to execute in this environment.

## Diagnostics and docs impact

No new user-facing error codes: this plan's surface is compiler-
internal, and every user-visible behavior is pinned identical. Two
diagnostic-adjacent improvements ride along because the registry
makes them one-line-cheap: `ciac verify`'s validator failures name
the step's `purpose` ("type-checks failed" rather than only "npm
exited 1"), and `ciac sim`'s `SimSupport::None` refusals cite the
target's own stated reason uniformly instead of per-call-site prose.
Docs: `backend-authoring.md` rewritten (Pillar 6);
`external-backends.md` versioned migration note (Pillar 2);
`language.md` provider table becomes generated-with-drift-test
(Pillar 1); `backends.md` gains a short "how backends are built"
section pointing at the authoring guide; README's target list reads
from `ciac targets`. AGENTS.md (repo-level) gains one paragraph:
where backend code lives, what the conformance harness asserts, and
that `model.rs` is target-neutral by test, not convention.

## Relationship to the forecast documents

Explicitly repeated from the preamble because scheduling questions
land here: v0.19 (outbox/idempotency), v0.20 (provenance), v0.21
(breadth) remain open forecasts. This plan neither blocks on them
nor blocks them — but it changes v0.19's cost calculus favorably
(outbox machinery designed once in the shared model, rendered per
target through the factory) and it consumes v0.21's TypeScript
candidate via 23UpdatePlan.md. If a forecast track executes between
this plan and the language plans, the language plans' M1
reconciliation steps absorb the drift — the same
reconcile-against-reality discipline v0.17 M1 established.

## Explicit cuts

No new language ships in this plan (that's 23–25). No
template-language change (minijinja stays; its filter mechanism is
load-bearing here). No cross-language template unification beyond
the existing shared compose/k8s/terraform/ci layer — a shared
"service template" rendered five ways is the tar pit this plan
names and forbids. No plugin/dynamic-loading backend discovery; the
registry stays compiled-in for bundled targets. No external-protocol
support for typed-handler lowering (would require shipping HIR plus
a lowering spec across the wire — real, unbounded future work). No
attempt to make `system_tests.rs` emit non-Python system suites —
the system suite is deliberately one language regardless of target,
as v0.8 M4 decided. No changes to generated-code content at all:
this plan's entire observable output surface is "identical bytes,
one wire-schema version bump."

## Risks

- **Unification pressure produces worse generated code.** Mitigated
  by the golden-identical rule: shared machinery must reproduce
  today's output exactly, or it doesn't land. The generated code is
  the constitution; the compiler internals adapt to it, never the
  reverse.
- **`HostSyntax` becomes a lowest-common-denominator that fights the
  next language.** Mitigated by scoping it to leaves, keeping
  orientation an explicit mode, and pre-authorizing contract
  amendments through the goldens-first procedure plans 24 (error
  idiom) and 25 (none expected) already schedule. The contract has
  its own snapshot, so amendments are visible diffs.
- **Protocol break annoys external-backend authors.** There is
  exactly one known external backend (the in-repo Go reference). The
  break is versioned, documented, migration-noted, and makes the
  payload smaller and permanently more stable.
- **The refactor never ends.** Every milestone has a hard,
  mechanical acceptance test (byte-identical goldens, a measured LOC
  number, a compiling skeleton, a passing matrix). No milestone's
  exit is a judgment call, and the fallback for the one uncertain
  unification is pre-agreed and bounded.
- **Hidden coupling surfaces late** (something in the six files
  secretly depends on target-name strings beyond the audited sites).
  Mitigated by M1's byte-comparison session across every CLI surface
  that touches targets, and by grep-audit (`"python"`/`"rust"`
  string literals outside backend crates) as an M1 exit checklist
  item with each surviving site justified in a comment.

## Confidence and handoff

High confidence: this is consolidation of proven code under the
strongest possible regression oracle (byte-identical output), with
direct in-repo precedent for each pillar (v0.9 compose dedup for
emission sharing, v0.10 FieldTypeKind for neutral types, v0.13 M1
placeholder discipline for the shared-SQL contract the harness
asserts, v0.17 M11's `unsupported_sim_capabilities` for `SimSupport`).
The handoff artifact is M6's measured cost model. 23UpdatePlan.md
(TypeScript) begins by validating its estimates against it and
doubles as this plan's live acceptance test: if the TS backend's
non-template cost lands materially above the M6 numbers, that is a
defect in this plan's deliverables, tracked and fixed as such before
Go and Java consume the same factory.
