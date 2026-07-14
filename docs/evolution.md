# Evolution: semantic diff, rename, and the backfill ladder

v0.18 answers a question the compiler couldn't answer before: *did this
change break anything?* Every earlier version diffed **generated
files** (`ciac diff`, since v0.6) — useful for regeneration hygiene, but
blind to architecture. Renaming a field and deleting-then-recreating an
identically-named one produce the same file diff; a route that dropped
a required request field and one that only reordered its declaration
produce different file diffs. Neither comparison is what a team
actually wants to know before merging: *is this contract change safe
for whoever already depends on it?*

This document covers the four things v0.18 adds, in the order a real
change moves through them:

1. [**Semantic diff**](#semantic-diff) — a target-independent
   architecture model, and a differ that classifies each change as
   `Breaking`, `Additive`, or `Internal`.
2. [**Baselines and the CI gate**](#baselines-and-the-ci-gate) — a
   checked-in snapshot `ciac diff --semantic` compares against, and a
   generated CI job that blocks a breaking merge.
3. [**Rename**](#rename) — a whole-program, multi-file symbol rename
   that also replays affected `--out` trees' regeneration, transactionally.
4. [**The backfill ladder**](#the-backfill-ladder) — the expand →
   backfill → contract sequence for turning a breaking storage change
   into a safe one, with a human doing the one step CIaC can't: writing
   the per-row conversion.

Everything below is a real, live-run transcript against
[`examples/order-system.ciac`](../examples/order-system.ciac) (with a
required `priority: Int;` field added to `Order` and its constructing
handler updated to set it, and later a handler renamed) — not
hypothetical output.

## The gap this closes

`ciac diff` (v0.6) answers "what would regenerating this output tree
change" — a Python/Rust file-tree question. It says nothing about
whether a route's request shape changed, a table column disappeared,
or a stream's payload type changed, because it never looks past the
generated files at the architecture that produced them. `ciac diff
--semantic` is a second, independent comparison over a canonical model
of the *architecture itself* — routes, payload shapes, storage columns,
stream/channel topology, capabilities — id-keyed so that reordering
declarations or reflowing whitespace never registers as a change.

## Semantic diff

### The canonical model

`ciac_codegen::semantic_model::SemanticModel::from_ir` projects
`NormalizedIr` into logical, insertion-order-independent keys instead
of the compiler's internal `NodeId`/`ServiceId`/`RecordId`: `record/
Order`, `record/Order/field/priority`, `service/OrderSystem/api/
MarkShippedApi`, `table/orders`, and so on. Reordering declarations or
reformatting a program provably produces the same model and the same
`semantic_hash` — this is a dedicated invariance test, not an assumed
property.

### The classification matrix

`ciac_codegen::semantic_diff::diff_models` compares two `SemanticModel`s
and emits a `Change` per difference, each carrying a stable `kind`
(e.g. `table.column.added_required`, `record.field.added`,
`handler.body_changed`), a `Classification` (`Breaking` / `Additive` /
`Internal`), the affected symbol's key, and a human message. The matrix
covers routes, method/path, auth/scope, request/response fields, enums,
streams/subjects, channels, relations/validation, constraints, tables/
columns, capabilities/providers, retries/schedules, handlers, and
pipeline edges — reviewable as a vocabulary of change kinds, not a
hidden heuristic.

```sh
$ ciac diff order-system.ciac --semantic
Breaking  table.column.added_required    Order.priority
          record `Order` (table-backed) gained field `priority` with no universal default for existing rows
          note: `ciac backfill plan` is available for this change
Additive  record.field.added             OrderUpdate.priority
          record `OrderUpdate` gained field `priority`
Internal  handler.body_changed           service/OrderSystem/handler/MarkShipped
          handler body changed (structural digest only, not behavioral)
```

One source edit — adding a field to two records and touching a handler
body — produced all three classifications in one run: a table-backed
field addition with no safe default is `Breaking` (existing rows can't
retroactively acquire a value CIaC has no way to invent); the same
shape added to a plain (non-table) record is `Additive`, since nothing
already serialized depends on the field being absent; and the handler
body's structural digest changing is `Internal` — CIaC diffs the body's
*shape*, never its behavior (see [Explicit cuts](#explicit-cuts)
below), so this classification means "something in this handler
changed," not "this handler now behaves differently."

`--deny-breaking` turns a `Breaking` change into a non-zero exit —
report-only by default, a hard gate on request:

```sh
$ ciac diff order-system.ciac --semantic --deny-breaking; echo "exit: $?"
Breaking  table.column.added_required    Order.priority
...
exit: 1
```

`--json` carries the identical changelist as one structured document
(`semantic.changes[]`, each with `id`/`kind`/`classification`/`symbol`/
`before`/`after`/`consumers`/`message`) — the same data `ciac mcp`'s
`diff_semantic` tool returns, never a second copy that can drift from
the human rendering.

### What "Internal" does *not* claim

A change classified `Internal` is not proven safe to deploy — it means
the differ found no *contract* change (no field, route, or stream
shape moved). A `fail`-vs-early-return refactor inside a handler body,
a changed SQL predicate's semantics, or a timing change are all real
behavior changes this differ cannot and does not see. Treat `Internal`
as "no known-shape break," not "safe."

## Baselines and the CI gate

`ciac baseline <file>` creates or replaces the checked-in snapshot
`ciac diff --semantic` compares the current program against by default
(`<entry-dir>/.ciac/baselines/<entry-stem>.semantic.json`):

```sh
$ ciac baseline order-system.ciac
.ciac/baselines/order-system.semantic.json: created (semantic_hash sha256:c9d327d1...)
```

Replacing an existing baseline whose architecture actually changed
needs `--update --accept-breaking` (plus an optional `--reason`, which
is appended to a source-owned `CHANGELOG.ciac.md` along with the
before/after hash) — this is a human decision CIaC never makes for you;
there is no `--auto-accept` and no code path that silently rewrites a
baseline that changed. A byte-for-byte-unchanged recreation, or a first
creation, needs neither flag.

`--against <file>` compares two source files directly instead of a
baseline (no `--baseline` write involved) — useful for a quick two-
branch comparison without touching `.ciac/`.

`ciac build --deploy ci` (v0.15) gains a `semantic-compat` job
(18UpdatePlan.md Pillar 4) whenever a checked-in baseline exists: it
runs `ciac diff --semantic --deny-breaking --baseline <path> --json`,
uploads the changelist as a build artifact, writes it to the job
summary, and gates the `test` job behind it — a breaking change never
reaches the rest of CI un-flagged. The job pins the same `ciac` version
the rest of the workflow uses (no floating "latest compiler" in CI) and
never writes the baseline itself; a commit trailer like
`ciac-breaking: reviewed` is surfaced in the summary as an
*informational* note only — it is never accepted in place of the real
gate result.

## Rename

`ciac rename` is a whole-program, multi-file symbol rename — not a
text search-and-replace. It resolves through
`ciac_syntax::rename_index`, a whole-program index of every
definition and reference built from the module-merged AST **before**
blueprint expansion (expansion mangles body-declared names into
synthetic identifiers while preserving the original source spans, so
indexing has to happen first). Two forms locate the symbol:

```sh
# position-based: --file/--line/--column identify the exact site
ciac rename entry.ciac --file entry.ciac --line 49 --column 8 --to PurchaseOrder

# qualified convenience form: an unambiguous name (or `Record.field`)
ciac rename entry.ciac Order PurchaseOrder
```

### Dry run

Dry run is the default — nothing is written until `--apply`:

```sh
$ ciac rename order-system.ciac Order PurchaseOrder
rename record `Order` -> `PurchaseOrder` (1 file, 5 sites)
  order-system.ciac
    49:8: `Order` -> `PurchaseOrder`
    57:15: `Order` -> `PurchaseOrder`
    86:46: `Order` -> `PurchaseOrder`
    87:19: `Order` -> `PurchaseOrder`
    136:23: `Order` -> `PurchaseOrder`
(dry run — pass --apply to write these files)
```

Every occurrence resolves through the same symbol table: the
declaration, the `table Orders: Order;` reference, two type positions
inside a handler signature, and — line 136 — a **blueprint type
argument**, `expand RateLimitedApi<Order>`. Blueprint provenance is
indexed before expansion specifically so a case like this is caught;
a heuristic text-based rename would have no way to know that occurrence
is a renamable reference rather than a comment or an unrelated string.

### Applying: staging, verification, and rollback

`--apply` doesn't write files in place. It stages every affected file
(a same-directory backup, then an atomic tmp-write-then-rename), writes
a recovery journal *before* touching any real file, then re-parses and
re-analyzes the edited program. If that fails, every staged file is
rolled back from its backup and the journal is removed — the rename
never leaves a program that used to compile in a state that doesn't.

A stale journal from an interrupted prior run is refused, not silently
overwritten or silently ignored, so a genuinely interrupted rename
always gets a human's attention rather than a guess.

### `--out`: regeneration replay, and why it can refuse the whole rename

Passing one or more `--out <dir>` (repeatable, requires `--apply`)
replays that tree's checked-in build recipe (recorded on the manifest
by every `ciac build` since v0.18 M5) against the renamed source —
*before* committing anything. If any listed tree can't regenerate
safely, the entire rename is refused, **including the source edit**:

```sh
$ ciac rename order-system.ciac Order PurchaseOrder --apply --out ./out
rename record `Order` -> `PurchaseOrder` (1 file, 5 sites)
  ...
error[CIAC0046]: table `tickets_order` was removed; drop it with a manual migration
error: unsupported schema change
```

This is a real result from this document's own walkthrough, not a
contrived one: `RateLimitedApi<Order>` (line 136) is a blueprint whose
expansion derives a table name from its type parameter — renaming
`Order` changes that derived name too (`tickets_order` →
`tickets_purchase_order`), which the regeneration replay sees as an
unsupported table removal and refuses to apply automatically. Because
the check runs *before* the transaction commits, refusing it also rolls
the source edit back — `order-system.ciac` is byte-identical to before
the command ran. A rename that only touches non-storage symbols (a
handler name, for instance) has no such conflict and commits cleanly,
regenerating the affected files in the same command:

```sh
$ ciac rename order-system.ciac MarkShipped ShipOrderHandler --apply --out ./out
rename handler `MarkShipped` -> `ShipOrderHandler` (1 file, 2 sites)
  ...
applied: 1 file(s) written, 1 output tree(s) regenerated
$ ciac verify order-system.ciac -t python -o ./out
...
17 passed, 16 warnings in 0.84s
```

`--out` refuses (rather than guesses) against a **legacy manifest**
with no recorded build recipe — run `ciac build` once with this `ciac`
to upgrade it first.

### Seeded-file scanning

Once a rename commits, every manifest-tracked *seeded* file (handler
stubs, migrations — code CIaC generated once and now leaves to a human)
is grepped for the literal old name, and any hit is reported as
`possible_reference` — informational only, never rewritten and never a
blocking condition. CIaC does not parse arbitrary Python/Rust/
TypeScript/SQL, so this is a heuristic surfaced for review, not a
compile-time guarantee.

### What rename does not do

- **No heuristic inference.** A rename is only ever a rename if you
  invoke `ciac rename`; the identical textual edit made by hand (delete
  a declaration, add a differently-named one) is indistinguishable from
  a real remove-then-add to every other part of the compiler, including
  the semantic differ and the migration differ.
- **No import/file/directory rename**, no cross-repository/registry
  rename, no editing `std/` or the registry cache.
- **No selective rename of one blueprint expansion** with no source
  identity to rename.
- Read-only source (a symbol declared in an embedded `std/` file or a
  `registry:` import) is refused, not silently skipped.

## The backfill ladder

A `Breaking` `table.column.added_required` change can't be resolved by
a single migration: existing rows have no value for a new required
column, and CIaC cannot invent one — that depends on the domain.
`18UpdatePlan.md` Pillar 5 splits this into three steps, only the first
of which was already automatic.

### 1. Expand (already automatic)

An ordinary `ciac build`/`ciac verify` against `--out` already writes
and applies the safely nullable `ALTER TABLE ... ADD COLUMN` the moment
a required field is added to a table-backed record — no new command.
This is `ciac-codegen::migrations::diff_schema`'s existing additive-only
behavior; `ciac backfill plan` starts *after* this step, refusing to
plan until it's landed in the target tree.

### 2. Backfill (a seeded, target-native script)

```sh
$ ciac backfill plan order-system.ciac --out ./out
plan d8438d3390ff3d43: orders.priority (record `Order` (table-backed) gained field `priority` with no universal default for existing rows)
  backfill script: out/app/migrations/backfill_d8438d3390ff3d43.py (written)
  plan record: out/.ciac/backfills/d8438d3390ff3d43.json
  contract migration: withheld — re-run with --allow-destructive d8438d3390ff3d43 once the backfill script above has been run against the target database
```

The seeded script (placed per-target: `app/migrations/` for Python,
`migrations/` for Rust — the same convention the expand migration
itself already uses) is real, runnable code with the mechanical parts
filled in (a `SELECT ... WHERE priority IS NULL`, an `UPDATE ...
WHERE id = :id` loop, a final ledger `INSERT`) and exactly one `TODO`:
the per-row value. CIaC never runs this script and never decides a plan
is complete — that's a human, once, against the real database.

### 3. Contract (guarded, plan-scoped)

`--allow-destructive <plan-id>` materializes the contract migration —
but only for the *exact* plan id already on record, and only once:

```sh
$ ciac backfill plan order-system.ciac --out ./out --allow-destructive d8438d3390ff3d43
...
  contract migration: out/app/migrations/0003_contract_d8438d3390ff3d43.sql (seq 0003, guarded on plan d8438d3390ff3d43)
```

```sql
CREATE TABLE IF NOT EXISTS _ciac_backfills ( ... );

-- Refuses to apply unless plan d8438d3390ff3d43 is recorded complete in
-- _ciac_backfills. A completed row is written by the seeded backfill
-- script, never by CIaC itself.
SELECT CASE WHEN NOT EXISTS (
    SELECT 1 FROM _ciac_backfills WHERE plan_id = 'd8438d3390ff3d43'
) THEN 1/0 ELSE 0 END;

ALTER TABLE orders ALTER COLUMN priority SET NOT NULL
```

The guard is a portable SQL "assert" (a division by zero, which every
supported SQL engine turns into a runtime error) rather than an engine-
specific stored procedure, consistent with `migrations.rs`'s existing
one-portable-dialect design. `_ciac_backfills` is created idempotently
and only ever written to by the seeded script's own final `INSERT` —
CIaC itself never marks a plan complete. A second
`--allow-destructive` for an already-materialized plan id is refused
rather than writing a duplicate contract migration.

## MCP and LSP surfaces

Both new features are available to an agent client or an editor, not
only the terminal — `ciac mcp` and `ciac lsp` reuse the exact same
resolution/diff logic the CLI does, never a second implementation.

### `ciac mcp`

Two new tools alongside the existing `check`/`build`/`diff`/`verify`/
`graph`/`explain`/`describe`/`fix` (see [docs/agents.md](agents.md)):

| Tool | Mirrors |
|------|---------|
| `diff_semantic` | `ciac diff --semantic --json` |
| `rename` | `ciac rename` — dry-run preview by default, `apply: true` writes the files. Deliberately **source-only**: it never replays a `--out` tree's regeneration, since that needs a human reviewing the regenerated diff, not an agent committing it unattended. |

### `ciac lsp`

`textDocument/prepareRename` and `textDocument/rename` are backed by
the same `rename_index` the CLI uses: `prepareRename` highlights the
exact token under the cursor and seeds the rename box with its current
name; `rename` returns a `WorkspaceEdit` spanning every affected file
in the project, which the editor applies as one multi-file edit. Like
the rest of `ciac lsp`, both read from disk (no unsaved-buffer overlay
— continuing the same disclosed scope `ciac lsp`'s diagnostics have had
since v0.12) and never write anything themselves; the editor applies
the returned edit.

## Explicit cuts

Carried over verbatim from `18UpdatePlan.md`, since they shape what the
sections above do and don't claim:

- No behavioral equivalence of handler bodies — `Internal`/
  `handler.body_changed` is a structural digest, not a proof of
  unchanged behavior.
- No heuristic rename inference, no automatic baseline updates, no
  automatic data conversion.
- No first-class API versioning language (`/v1`, `/v2`) — semantic diff
  tells a team when a contract broke, not how that team versions it.
- No claim that `Internal` means operationally safe to deploy.
- No explicit `ALTER ... RENAME` migration-differ identity yet: a
  rename's storage-shape consequences (a renamed column, a blueprint-
  derived table name change) still surface to the regeneration differ
  as a plain add/remove, which is why the `--out` replay in the
  walkthrough above refuses a rename that removes a table rather than
  silently recognizing it as "the same table, renamed." This is a
  deliberate, disclosed gap (18UpdatePlan.md's own "explicit cuts"
  list) rather than a bug: teaching the migration differ a rename
  identity is deep enough surgery on `ciac-codegen::migrations` to risk
  v0.16's relational-migration work, and is left for a future version.

## Command reference

```sh
ciac diff file.ciac --semantic [--against FILE | --baseline FILE] [--deny-breaking] [--json] [--format text|markdown]
ciac baseline file.ciac [--out FILE] [--update --accept-breaking [--reason TEXT]]
ciac rename entry.ciac [Old New | --file FILE --line N --column N --to NAME] [--apply] [--out DIR]...
ciac backfill plan file.ciac --out DIR [--baseline FILE] [--allow-destructive PLAN_ID]
```

`docs/semantic-baseline-schema.json` is the checked-in JSON Schema for
the baseline document (`ciac semantic-baseline-schema` regenerates it
from the same types that serialize the real file).
