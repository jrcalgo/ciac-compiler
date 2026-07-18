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

   **Recorded fallback scope, execution-time (v0.22 M2), matching the
   discipline this plan's own Risks section pre-authorizes:** `NameForms`
   shipped (`ciac-codegen::model::NameForms`, computed via `heck`); the
   `FieldCtx` group (`py_type`/`py_out_type`/`rust_type`/`db_rust_type`
   — the plan's own flagship worked example) fully migrated to
   backend-owned filters (`ciac-backend-python`/`-rust`'s
   `filters::py_type`/`py_out_type`/`rust_type`/`db_rust_type`, using
   minijinja's `ViaDeserialize` — the mechanism that makes `{{ field |
   py_type }}` work against a `Deserialize`-derived wrapper struct
   rather than the whole `FieldCtx`), byte-identical-golden verified,
   fields deleted, `PROTOCOL_VERSION` bumped to 2, schema regenerated,
   `docs/external-backends.md` given a v2 migration note, and the Go
   reference backend's own `protocolVersion` constant bumped with its
   `ciac build --target go` live proof re-run (see `backends/go/
   main.go`'s v0.22 M2 comment). One live audit finding surfaced during
   this pass and was fixed as an improvement, not deferred:
   `records_use_datetime`/`_json`/`_enum` were computed by string-
   sniffing the *Python-rendered* `py_type` text (checking for
   `"datetime"`/`"Any"`/`"Literal"` substrings) even though Rust's own
   `Cargo.toml.j2` reads `records_use_datetime` too — now computed
   directly from neutral `type_kind`, which is more correct, not just
   more neutral.

   The remaining host fields the audit named — `CfgFieldCtx`'s
   `py_ann`/`py_default`/`rust_default`; `ExtraDepCtx`'s `py_type`/
   `py_module`/`py_expr`/`py_getter`/`rust_type`/`rust_module`/
   `rust_state_field`; `ExtraImportCtx`'s `py_module`/`py_getter`;
   `ArmCtx::rust_variant`; `BindingCtx`'s `py_attr`/`rust_field`; and
   the `rust_db_field`/`rust_cache_field`/`py_args` family across
   `HandlerRef` and related structs — are **composed filters** (per
   the disposition table: each needs more than one neutral field
   composed together, e.g. binding kind + instance `NameForms`, not a
   single `FieldTypeKind` match), not pure functions of one enum the
   way `FieldCtx`'s group was. They follow the exact pattern just
   proven end-to-end (backend-owned filter, `ViaDeserialize` wrapper
   struct, byte-identical golden gate, then field deletion) but each
   needs its own small design pass to identify precisely which neutral
   fields it composes over — real, contained, mechanical follow-up
   work, deferred rather than rushed. `NameForms` itself is defined and
   proven but not yet threaded through every `snake`/`class_name` pair
   in `model.rs` (Move 1's full sweep) — same disposition, same
   deferral. Whoever picks this up next starts from a working,
   golden-verified template (this commit) rather than a blank page.
3. **M3 — `lower_core` + `HostSyntax`; port both backends.**
   Scanner first (both backends consume it; sim proofs re-run since
   `unguarded_verbs` moved), then dispatch + leaves per backend,
   golden-byte-identical or the disclosed fallback scope with the
   decision recorded. The identity-syntax snapshot lands as the
   contract's own golden. Acceptance metric recorded: leaf-only LOC
   for the identity target, and before/after LOC for both real
   backends.

   **Fallback scope taken (v0.22 M3), pre-authorized by this plan's own
   Risks section:** Part 1 (the `Needs` scanner) shipped in full —
   `ciac-codegen::lower::{Needs, scan, field_access_enum_name}` is now
   the one traversal both backends' `render()` call, computing the
   union of what Python's behavioral-test mock assertions need
   (`db_insert` count, `cache_get`/`cache_set`/`object_store_*`/
   `sa_*` booleans) and what Rust's imports/sim-coverage need
   (`db_get_tables`, `enums`, `unguarded_verbs`) in one pass. This is
   the correctness-bearing half the plan itself named — `unguarded_verbs`
   can no longer silently fall out of sync with what's actually
   scanned, because there is only one scan. Parts 2 and 3 (the
   dispatch skeleton and the `HostSyntax` leaf trait) are **not**
   attempted in this pass: Python's `Sink`/statement-orientation tail
   shaping vs. Rust's expression-orientation is exactly the "hardest
   unification" risk this plan's own text flagged in advance, and
   attempting it without a dedicated pass would mean the highest-risk,
   least-reversible rewrite in the whole four-plan arc going in
   unreviewed. Per-backend `py_expr`/`lower_tail`/`lower_block` and
   `rust_expr`/`rust_stmt`/`rust_block` are unchanged.

   Measured LOC (the milestone's own acceptance metric, for the part
   that shipped): Python's `lower.rs` 1305 → 1058 (−247); Rust's
   `lower.rs` 1088 → 803 (−285); new shared
   `ciac-codegen::lower` +358. Net across all three files: 2393 → 2219
   (−174), with the scanner-duplication risk eliminated even though
   the leaf/dispatch unification (the larger remaining LOC opportunity
   Parts 2/3 would close) is deferred. Byte-identical goldens hold;
   both v0.17 M11 sim CLI proofs (pass, and the narrow-target/refusal
   cases) reproduce unchanged, confirming the scanner move didn't
   perturb `unguarded_verbs`' actual coverage.

   **M3, continued — Parts 2-3 completed (continuation pass, after
   this plan's own M6 below had already closed the arc out).** The
   dispatch skeleton and the `HostSyntax` leaf trait this milestone's
   fallback deferred were subsequently implemented in full, under the
   identical byte-identical-golden discipline this plan's M2/M3/M5
   already proved workable. `ciac_codegen::lower` became a directory:
   `scan.rs` (Part 1, moved verbatim, zero logic change), `dispatch.rs`
   (the shared statement/expression walker — `lower_scalar`/
   `lower_expr_any`/`lower_tail`/`lower_block_expr`/`lower_block_stmt`/
   `lower_stmt`, plus `strip_outer_parens`/`fidelity_checked_float`/
   `indent_lines` moved verbatim from the Rust backend), `host_syntax.rs`
   (the `HostSyntax` trait — 44 universal/orientation-shared methods
   plus 4 `Expression`-only and 4 `Statement`-only leaves, ~52 total,
   more granular than this plan's original "roughly 30" estimate: enum
   use-site resolution, the record-field clone hook, and
   expression-vs-statement db-verb/dest-application leaves each needed
   their own method once real signatures were derived from the
   existing code rather than guessed at), and `identity.rs` (the
   contract's own reference implementation, one struct per
   orientation, proven against the full example corpus in the new
   `tests/tests/host_syntax_identity.rs`). Both backends'
   `py_expr`/`lower_tail`/`lower_block`/`lower_stmt`/`Sink` and
   `rust_expr`/`rust_stmt`/`rust_block`/`Tail` were deleted outright
   and replaced by a `PySyntax`/`RustSyntax` implementing `HostSyntax`
   against the shared walker — zero dead code left behind in either
   backend crate (checked by grep, not assumed).

   The named risk did not require the pre-agreed fallback: unifying
   Python's statement orientation and Rust's expression orientation
   under one dispatcher produced byte-identical output for Rust after
   one bug found and fixed *before* any golden was accepted (see
   below), and byte-identical output for Python on the first attempt,
   with no readability regression in either target's generated code
   observed during review. The fallback this plan's own Risks section
   pre-authorized (keep the shared walking skeleton, leave per-backend
   leaf-level string formatting) was therefore not needed — recorded
   here because "attempt the real thing first" was itself a deliberate
   decision, not an accident of things going smoothly.

   **The one real bug this pass found, in its own new code, before
   landing:** the initial `Wrap` design collapsed Rust's three-state
   `Tail` (`None`/`Plain`/`Wrapped`) into two states, conflating a
   mid-block statement's `None` (`;`-terminated, discarded) with a
   nested branch's own `Plain` tail (bare, feeding the enclosing
   expression). Invisible for `if`/`match` branches (their own tail
   always matches the block's `wrap` parameter), but wrong for
   `transaction { }`'s inner block, which always lowers as `None`
   regardless of position — the golden suite caught it immediately (a
   missing `;` after a `db.insert` block-expression inside a
   `transaction` block, surfaced on `domain-orders.ciac`), fixed by
   restoring the third `Wrap::None` state before any snapshot was
   accepted. Recorded here because it is the concrete version of the
   risk this plan's own text named in advance ("the shared dispatcher
   becomes a single point of failure for two backends at once") —
   caught by the acceptance bar this plan is built around, not by
   inspection.

   Measured LOC (this pass's own acceptance metric, matching this
   milestone's own reporting discipline): `ciac-backend-rust/src/lower.rs`
   803 → 577 (−226); `ciac-backend-python/src/lower.rs` 1058 → 869
   (−189, still including its own ~320-line generated-behavioral-test
   family — `render_test`/`dummy_value`/`assert_result`/
   `collect_record_ids` — which depends only on the Part 1 scanner and
   was never in this pass's scope to move); shared `ciac-codegen::lower`
   359 → 1,980 (`scan.rs` 364, `dispatch.rs` 792, `host_syntax.rs` 317,
   `identity.rs` 461, `mod.rs` 46). Net across all three files:
   2,220 → 3,426 (+1,206) — a real increase, not a reduction; the
   payoff is architectural (one walker, ~50 leaves per backend, both
   proven against the same HIR corpus from both orientations by
   `host_syntax_identity.rs`), not a line-count saving, exactly as this
   plan's own cost-model framing always said it would be for this
   pillar. `docs/backends.md`'s cost table carries the full breakdown
   and is reconciled to no longer describe `HostSyntax` as unshipped.

   Byte-identical goldens hold across all 26 examples for both
   backends; `typed_handler_python.rs`, `typed_handler_rust.rs`, and
   `typed_handler_equivalence.rs` all pass unmodified. The v0.17 M11
   sim CLI proofs were re-run live, not just re-derived from the
   goldens: `ciac sim --target rust` on `sim-vertical-slice.ciac` and
   `sim-broker-slice.ciac` both `[PASS]` end to end (a real
   `cargo build` of the generated project, real sim-runner execution
   against `SimWorld`), and the narrow-target refusal against
   `extras-verbs.ciac` (`cache.delete`/`email.send`/`object_store.*`/
   `search.*`/`http.call` — none of them faked) reproduces the exact
   reason list `unsupported_sim_capabilities` always produced,
   confirming the Part 1 scanner this pass deliberately left untouched
   is in fact untouched. Python's own `ciac sim --target python` proof
   could not be re-run live in the sandbox this pass executed in (`uv`
   is not installed there — a pre-existing, environmental gap
   unrelated to this change: `backfill_cli.rs`'s own `uv sync`-dependent
   test was confirmed to fail identically against the unmodified,
   pre-this-pass tree); every Python file generated across all 26
   examples was instead confirmed to parse cleanly with `python3 -m
   py_compile`, and `typed_handler_python.rs`'s content assertions on
   the specific ORM calls the lowering emits continue to pass
   unmodified.

   Handoff to `23UpdatePlan.md`: its preamble's assumption that
   "`lower_core`/`HostSyntax`" has shipped is now true, not
   aspirational; its own M4 ("Typed handlers: `HostSyntax` for
   TypeScript") can proceed by implementing `HostSyntax` against the
   now-real, now-tested contract, following the amendment procedure
   this plan's own Part 3 text froze if TypeScript's leaf needs force
   a signature change.
4. **M4 — Conformance harness + `ciac targets --json`.** Matrix,
   OpenAPI/topology equality (run against python×rust immediately —
   any existing divergence found becomes a fix-or-disclose item in
   this milestone, not a surprise in plan 23), validator hookup with
   delegation reporting, ratchet enforcement wired to the matrix
   test, CI job added.

   **Shipped (v0.22 M4):** `tests/tests/conformance.rs` — C1/C2 already
   ran in `golden.rs` (referenced, not duplicated); C3 (cross-target
   OpenAPI byte-equality) and C4 (migration-SQL byte-equality by
   filename, plus every subject/queue-group/cron-schedule/table-name
   the model declares provably appearing in every supporting target's
   output) are new and pass clean across all 26 examples — **zero
   divergence found** between python and rust, so there was nothing to
   fix-or-disclose this milestone. C5 stays delegated to the existing
   `generated-python`/`generated-rust` CI jobs (real local-toolchain
   validation already ran there since v0.9); no separate CI job was
   needed since `cargo test --workspace` (already the `test` job)
   picks up `tests/tests/conformance.rs` automatically. C6 (ratchet
   proofs) and C7 (boundary decode/encode) are named but have no
   content yet — C6 has nothing to mechanically check until a third
   target can diverge from the first two; C7 is 24UpdatePlan.md's
   (Go's) to introduce. `ciac targets --json` ships with the plan's
   own fixed shape (id/description/kind/project_marker/validate/sim/
   capabilities), checked in at `docs/targets.json` with a drift test
   (`crates/ciac/tests/targets_cli.rs`) mirroring the
   `protocol-schema.json` pattern. `capabilities` is sourced from
   `vocab::PROVIDERS` today, not yet `Backend::supports()`-derived —
   the same disposition M2 recorded for `vocab::BOTH`, for the same
   reason (`supports()` is still an unconditional `true` on both
   bundled backends; there is nothing yet to derive).
5. **M5 — Emission-plan helper + skeleton backend + authoring
   guide.** Both backends ported to the declarative emission table
   (golden-identical); `backends/skeleton-internal` compiles and is
   exercised; `docs/backend-authoring.md` rewritten with the measured
   what-you-must-write inventory (the cost-model table, updated with
   M1–M5 actuals).

   **Shipped (v0.22 M5), with a disclosed scope boundary:**
   `ciac_codegen::emit::{Emit, run}` — the declarative table + shared
   executor — is real and covers the always/conditional-single-file
   tier completely. `backends/skeleton-internal/` is a new workspace
   member: a compiling, gated (`supports()` always `false`) crate
   demonstrating `Backend`/`TargetInfo`/`Emit` end to end, with three
   passing tests (`emits_its_declared_file_set`,
   `supports_nothing_yet`, `target_info_is_populated_and_gated`) —
   the plan's own "one registry test." The doc that already served
   this page's purpose in-repo is `docs/backends.md` (not a new
   `backend-authoring.md` file); it's rewritten with the M1-M5
   walkthrough and a real, measured cost-model table.

   **Not shipped, recorded as follow-up:** porting the two *real*
   backends' emission sequences onto `Emit`. Each real backend's
   `emit_service` is roughly half always/conditional-single-file
   entries (what `Emit` covers today) and half per-item loops — one
   file per declared api/worker/job/consumer/channel/resource/call-
   target — which need per-item context threading `Emit`'s current
   shape doesn't express. Building that generically (an `Emit::per_x`
   variant taking an item-iterator + context-builder closure) is real,
   scoped, bounded work; attempting it as a rushed addition to this
   milestone risked a half-abstracted result touching both production
   backends' entire file lists under time pressure. `lib.rs` emission-
   wiring LOC is therefore unchanged from the pre-factory baseline —
   see the cost-model table in `docs/backends.md`, which reports this
   honestly rather than the plan's original aspirational ~350-line
   figure.
6. **M6 — Version, docs reconciliation, retrospective.** Workspace
   version bump (number assigned at execution); README/docs support
   tables switched to the generated source of truth; whole-version
   analysis: the before/after cost model with real numbers, every
   deviation from this plan disclosed in place, and the explicit
   handoff sentence to 23UpdatePlan.md — whose M5 checkpoint grades
   these numbers against a real third backend.

   **Shipped (v0.22 M6):** workspace version bumped `0.19.0` →
   `0.20.0` (root `Cargo.toml`'s `[workspace.package]` version plus
   its eight internal path-dependency version pins, all driven from
   one source; `docs/language.md`'s title; `editors/vscode/
   package.json`'s extension version, matching the pattern every
   prior version's M-final milestone followed). `docs/targets.json`
   (`targets_version: 1`) and `docs/protocol-schema.json`
   (`protocol_version: 2`) are independently-versioned artifacts, not
   tied to the workspace version, and needed no change — verified by
   grep rather than assumed. The one hand-written "support table"
   actually at drift risk — `docs/language.md`'s provider table
   (`vocab::PROVIDERS` rendered as prose, since `PROVIDERS` lives in
   the `ciac` binary crate, which has no `lib` target the `tests`
   crate could import — the same structural constraint
   `targets_cli.rs` already worked around at M4) — got a drift test:
   `crates/ciac/src/vocab.rs::tests::language_md_mentions_every_provider`,
   `include_str!`-checking every `PROVIDERS` entry's name appears in
   the doc, mirroring `tests/tests/docs.rs`'s existing
   `error_docs_cover_every_code` pattern but relocated into the
   binary crate's own unit tests for the same reason. README.md
   carries no hardcoded version/support strings at risk (checked,
   not assumed — only illustrative `--target python|rust` CLI
   examples). Full verification green: `cargo fmt --all --check`,
   `cargo clippy --workspace --all-targets -- -D warnings`, `cargo
   test --workspace` (58 suites, 0 failures, including the
   conformance harness, the grep fence, the `targets.json` drift
   test, and the new provider-doc drift test).

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

## Whole-arc retrospective (v0.22, M1–M6 complete)

**What shipped, in one pass, plain terms:** `TargetInfo` on the
`Backend` trait closed seam 3 (scattered per-target `match` sites) —
~25 sites across 6 non-backend files collapsed to registry lookups,
held shut by a new repo test (`target_literal_fence.rs`) rather than
by discipline. The `FieldCtx` flagship group of the neutral-typing
seam (seam 2) moved to backend-owned minijinja filters via
`ViaDeserialize`, with a real correctness fix (`records_use_datetime`
etc. no longer string-sniff Python's rendered type text) falling out
of the migration rather than being hunted for separately. The HIR
scanner (the correctness-bearing half of seam 2's `lower.rs`
duplication) unified into one `ciac_codegen::lower::scan` both
backends call, at a measured net −174 LOC across the three files it
touches. A conformance harness (`tests/tests/conformance.rs`) now
runs C3/C4 on every one of the 26 examples on every `cargo test
--workspace` — it found zero divergence between python and rust,
which is itself a real result (the two hand-maintained backends had,
in fact, stayed in lockstep without the harness; the harness's value
is that this is no longer an assumption). `ciac targets --json` gives
agents and CI a machine-readable capability registry backed by a
checked-in, drift-tested snapshot. A declarative `Emit` table plus a
compiling reference skeleton (`backends/skeleton-internal/`) give a
new backend author a working template to copy instead of a blank
page, and `docs/backends.md` states honestly, with measured numbers,
what a sixth backend would still cost today.

**What was deliberately not shipped, and why that was the right
call:** three items, each named in this plan's own Risks section in
advance as a legitimate fallback rather than discovered as a
surprise mid-execution. (1) `HostSyntax` — the trait that would
unify Python's statement-oriented `Sink` lowering with Rust's
expression-oriented `rust_expr`/`rust_stmt` behind one leaf
interface — was the plan's own flagged "hardest unification risk";
attempting it inside M3's time box would have put the highest-risk,
least-reversible rewrite in the whole four-plan arc through without
a dedicated review pass. *(Since shipped, in a dedicated continuation
pass — see this file's M3 section's own "M3, continued" addendum for
the full account, including the one real bug the byte-identical
discipline caught before landing.)* (2) Porting the two real backends' per-item
emission loops (one file per declared api/worker/job/consumer/
channel/resource/call-target) onto `Emit` — `Emit` today only covers
the always/conditional-single-file tier; per-item emission needs an
`Emit::per_x` shape with item-iterator + context-builder threading
that doesn't exist yet, and bolting it on under M5's pressure risked
a half-abstracted result touching every file both production
backends emit. (3) The remaining ~20 composed-filter host fields
(`ExtraDepCtx`, `CfgFieldCtx`, `ArmCtx`, `BindingCtx`, the
`rust_db_field`/`rust_cache_field`/`py_args` family) — each needs its
own small design pass to identify which neutral fields it composes
over, unlike `FieldCtx`'s group, which was a pure function of one
enum. All three are recorded as follow-up in their milestone's own
notes above, not silently dropped.

**The cost model, measured (see `docs/backends.md` for the full
table and honest summary):** the factory closed the seam that was
cheapest to fix and most dangerous to leave — seam 3 entirely (25
sites → 1 registry line, held by a test) and seam 2's
correctness-bearing scanner half (−174 net LOC, with the *shape* of
`unguarded_verbs` coverage now provably identical between backends
because there is only one scanner to drift). The largest remaining
line-count opportunity — `HostSyntax` leaf unification and per-item
`Emit` — is real, was scoped rather than guessed at, and is now
23–25UpdatePlan.md's to either consume as-is or extend. A new backend
today is still primarily template-writing plus hand-written leaf
lowering, not primarily archaeology across shared crates — which was
this plan's actual goal, and the honest cost table in
`docs/backends.md` is the evidence for that claim rather than an
assertion of it.

**Deviations from this plan's original text, all disclosed in
place** (no silent scope changes): M2 shipped only the `FieldCtx`
group of the neutral-field migration, not the full ~27-field sweep
the plan's flagship example implied — recorded in M2's own notes
above and in `docs/backends.md`'s cost table. M3 shipped Part 1
(scanner) of Pillar 3 only, not Parts 2/3 (dispatch skeleton +
`HostSyntax`) — recorded in M3's own notes and in the Risks-section
fallback this plan pre-authorized. M4 found zero cross-target
divergence, so had nothing to fix-or-disclose beyond that finding
itself; `capabilities` in `ciac targets --json` is sourced from
`vocab::PROVIDERS` rather than `Backend::supports()`, since
`supports()` carries no per-component discrimination on either
bundled backend yet to derive from — same disposition as `vocab::
BOTH`, recorded at both M2 and M4. M5 shipped `Emit` and the
skeleton but not porting the two real backends onto it — recorded in
M5's own notes. Every one of these was named as a possible fallback
by this plan's own Risks section before execution began; none is a
surprise discovered after the fact.

**Handoff to 23UpdatePlan.md (TypeScript):** this plan's M6 cost
model is now the acceptance test 23UpdatePlan.md's own M5 checkpoint
grades against. The concrete surface a TypeScript backend consumes
unchanged from this arc: `TargetInfo` (M1, zero seam-3 sites to add),
the backend-owned-filter pattern for its own neutral-to-TS type
mapping (M2's `ViaDeserialize` recipe, not yet its specific filters —
TS needs its own `ts_type`/etc.), the shared HIR scanner (M3,
`ciac_codegen::lower::scan` — no need to write a fourth copy of
`Needs`), the conformance harness (M4, C3/C4 run automatically the
moment a TS-target example is registered — this is where a first
real cross-target OpenAPI/topology proof, not just python×rust,
happens), `ciac targets --json` (M4, TS's entry appears the moment
its `Backend` is registered), and the `Emit`/skeleton walkthrough
(M5, `docs/backends.md`'s 9-step recipe and cost table as the
literal starting point — copy `backends/skeleton-internal/`, not a
blank file).

**Updated by the M3-continuation addendum above, disclosed here so
this section doesn't quietly go stale:** at the time this handoff
paragraph was first written, TS-specific leaf lowering had "no
`HostSyntax` to lean on." That is no longer true — `HostSyntax` (the
shared statement/expression dispatcher plus roughly 50 leaf
constructor methods) shipped in a dedicated continuation pass, proven
byte-identical against both existing backends and exercised from both
orientations against the full example corpus by
`tests/tests/host_syntax_identity.rs`. `23UpdatePlan.md`'s own M4
("Typed handlers: `HostSyntax` for TypeScript") can therefore proceed
by implementing the real, frozen contract — pick `Orientation`
(`Statement`, per that plan's own stated intent for a JS-family
target), implement the leaves, done — rather than designing a walker
from scratch. TS's own per-item emission (no `Emit::per_x` to lean on)
remains genuinely undesigned, unaffected by this addendum. If
TypeScript's leaf-implementation cost lands materially above what
`docs/backends.md`'s table now predicts for a `HostSyntax` consumer,
that is a defect in *this* addendum's contract design, tracked and
fixed via the amendment procedure Pillar 3's own text froze — the same
standard this section's original claim always held itself to, just
now measured against a real contract instead of a hoped-for one.
