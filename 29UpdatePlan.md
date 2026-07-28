# CIaC v0.29-file — The Front Door: Onboarding, Editor Polish, and the Pitch (implementation plan)

> Implementation plan. Document number ≠ release number (standing
> precedent; expected to ship as **0.27.0**). Assumes
> 26UpdatePlan.md–28UpdatePlan.md shipped — deliberately, because
> this arc's entire product is *description*, and the previous
> three arcs finished the system worth describing: gaps closed or
> classified, simulation at full depth over any topology,
> releases real, `install.sh` truthful. This is the first arc in
> the project's history whose primary deliverables are prose,
> and the plan treats prose with the same discipline the code
> arcs treated code: contracts, oracles, measured baselines, and
> an explicit statement of what "done" means for a document.
>
> **Reader contract** (this arc's parity contract): a stranger
> with a terminal and fifteen minutes can go from the README's
> first line to a generated, simulated service and know what they
> just did; a builder can follow a numbered guide series from
> first service to multi-service system to deployment without
> reading a single milestone log; an evaluator can read one
> positioning document and know when to reach for CIaC and — 
> stated with equal care — when not to; an editor user gets
> snippets, rich hovers, quick-fixes, and go-to-definition from
> the same single vocabulary source the compiler itself speaks;
> and the maintainer holds a `DOGFOODING.md` script ready to run
> with a real outside human, with the top cold-start frictions
> already fixed because the arc measured them three times. Every
> command block in every guide is executed against the real
> binary before the guide ships — documentation that lies is the
> one bug class this arc can introduce, so it gets an oracle.
>
> **Confidence:** high on mechanics — docs, snippets, hovers,
> quick-fixes, and go-to-definition are additive work on stable
> machinery (`vocab.rs`, the rename index, structured fixes, the
> TextMate/LSP plumbing from v0.12–v0.13). Medium on the one
> risk no milestone structure can engineer away: **author
> empathy** — every line of this project has been written by its
> own builder, and onboarding written from the inside
> reliably explains the wrong things. The mitigations are
> structural rather than aspirational: cold-start transcripts on
> clean containers (three of them, measured), the guide-veracity
> harness, an explicit audience model each document is checked
> against, and the honest framing of the dogfooding item — this
> arc *prepares* for an outside user; it does not pretend to
> manufacture one. The actual session with a real stranger is
> the user's to run, and everything this arc ships is shaped to
> make that session cheap to run and impossible to waste.

## The gap this version closes

The punch-list's Tier 3 read, verbatim: nothing reads like
"here's CIaC in 15 minutes" for someone who's never seen it. The
survey behind this plan confirmed it quantitatively: the README
is 366 lines of which the first 194 are a version-by-version
history — accurate, complete, and shaped exactly backwards for a
first encounter (a stranger must read about v0.3's additions
before learning what the tool does today); docs/ holds 21 files
of genuinely excellent *reference* — language spec, per-feature
guides, error index, schemas — and zero tutorials; there is no
document anywhere that says when you'd choose this over Rails,
NestJS, or a Prisma-driven generator, and when you wouldn't; and
the editor experience is functional but bare — completion items
with no snippets, hovers from a vocabulary table with no
per-target support data, no go-to-definition at all, and
quick-fixes only where v0.15's structured fixes happened to
reach.

None of this was neglect — it was sequencing. Reference docs and
engineering logs were the right docs while the system was being
built, and writing a front door for a half-finished building
would have meant rewriting it every arc. But the building is no
longer half-finished: after 26–28, the honest sentence about
CIaC is short and strong — one declarative source, five
production targets at parity, deterministic simulation of whole
systems with zero infrastructure, releases you can install with
one command — and nothing in the repository says that sentence
to a newcomer. This arc builds the front door: the narrative
README, the guide series, the positioning doc, the polished
editor, and the dogfooding kit. Its end state is not "users
arrive" — no document can promise that — it is "when a user
arrives, nothing about the first hour is accidentally hostile,
and when the maintainer recruits a test user, the session is
scripted, instrumented, and cheap."

## Pillar 1 — The audience model

Every document this arc writes or rewrites is checked against
one of three named readers, and mixing readers in one document
is the failure mode the model exists to prevent:

- **The evaluator** (15 minutes, deciding whether to care):
  served by the README's narrative and the positioning doc.
  Wants the claim, one honest demonstration, the boundaries,
  and zero history.
- **The builder** (hours, building something real): served by
  the guide series and the existing reference docs. Wants
  numbered steps that work, runnable checkpoints, and links
  *into* reference — never a wall of concepts before the first
  success.
- **The integrator/agent** (a tool or a person wiring CIaC into
  something): already served — `describe`, MCP, AGENTS.md,
  `--json` envelopes, the schemas — and this arc's only job for
  them is not to break the existing surface and to link it from
  the new index so it is findable.

The existing docs keep their reader assignments (language.md,
expressions.md, errors.md, ir.md, backends.md, targets.json:
reference; the per-feature docs: builder-depth); the arc's new
documents fill the two empty seats (evaluator, builder-tutorial)
rather than duplicating the occupied ones. The coherence pass
(M6) enforces exactly this: each doc states its reader in its
opening lines, and content that serves a different reader moves
or links instead of squatting.

## Pillar 2 — The README, rewritten as a narrative

### The shape

The new README is the 15-minute arc, in order:

1. **The claim** (a paragraph): one `.ciac` source; production
   backends in Python, Rust, TypeScript, Go, or Java at parity;
   deterministic simulation of whole systems with no Docker;
   generated deploys; a compiler that owns the seams. One
   sentence of anti-claim (what CIaC is not) linking the
   positioning doc — honesty above the fold.
2. **The demonstration** (the core): a compact but real program
   — a service with a record, a CRUD surface, a stream, a
   worker, a `transaction`, a failure-injected simulation
   scenario — walked from `curl | sh` install through
   `ciac new`/`check`/`build`, a look at what was generated (a
   *short* look — the point is that it is real code the reader
   can open), `ciac sim` with the scenario's exact outcomes,
   `ciac verify`, and `ciac dev`. Every command block executed
   by the veracity harness; the demonstration program checked in
   as an example so it can never rot apart from the corpus.
3. **The map** (brief): five targets and the parity claim
   (linking targets.json's derived truth and backends.md's
   ledger — the honest-disclosure culture is itself a selling
   point and gets shown, not buried); simulation in one
   paragraph (linking the guide); deployment in one paragraph;
   evolution/rename in one; the agent front door in one.
4. **The pointers**: guide series for builders, positioning for
   evaluators still deciding, reference index, contributing/
   building from source (kept from today), license.

### The demonstration program, sketched

The README's core example (final at M3; the sketch fixes the
scope — every feature here earns its place by appearing in the
walkthrough, and nothing else does):

```text
service notes {
  use { db Postgres; queue NATS; }

  record Note { id: Id; title: String; body: String; }
  table notes: Note;
  crud notes;

  stream NoteCreated: Note;

  api CreateNote(input: Note) -> Note {
    transaction { db.insert(notes, input); }
    publish NoteCreated(input);
    return input;
  }

  worker IndexNote on NoteCreated { /* count it, cheaply */ }
}
```

(Exact syntax deferred to the real grammar at M3 — the sketch's
job is scope: one record, CRUD, one handler with a transaction,
one stream, one worker. Small enough to read in a minute, real
enough that `ciac sim` has something worth asserting.) The
walkthrough runs: install → `ciac new` → paste → `ciac check` →
`ciac build -t python` (with one sentence: swap `python` for
rust/typescript/go/java and everything below still holds — the
five-target claim made *demonstrable* instead of asserted) → a
six-line look at the generated handler → `ciac sim` with a
scenario asserting a created row, a delivered event, and one
injected `db.commit` failure leaving zero rows → `ciac verify` →
`ciac dev`. Fifteen minutes, timed by the M5 transcript, and
every block harness-executed.

### What leaves

The version-by-version narrative (lines 1–194 today) moves to
`docs/history.md` essentially verbatim — it is a good document
about how the system came to be, mislabeled as an introduction.
The concept→target tables survive where the map section needs
them; the rest of the current README's content maps into the
new shape or into history.md; nothing is deleted, everything is
re-seated. The README's length budget: **under 250 lines** —
a budget, stated like every budget in this repo, so the
narrative cannot silently re-accrete history (future arcs add a
line to history.md, not a section to the README; the rule goes
in the coherence pass's doc-reader notes).

## Pillar 3 — The guide series

### The series

`docs/guide/`, numbered, each ending at a runnable checkpoint
the veracity harness executes:

| Guide | Covers | Checkpoint |
| --- | --- | --- |
| 01-first-service.md | install, `ciac new`, project anatomy, check/build/verify loop, reading generated code without fear | ping-class service green on `verify`, any target the reader picked |
| 02-records-and-crud.md | records, tables, typed CRUD, keyed store, migrations, `ciac diff`/regeneration discipline | CRUD round-trip via generated tests |
| 03-handlers-and-logic.md | typed handlers, the expression language, verbs (db/cache/http/email/store/search), transactions | a handler using three capability families, verified |
| 04-streams-and-workers.md | streams, publish, workers, jobs, channels, retry semantics | worker consuming a stream, job on a schedule |
| 05-simulation.md | scenarios, given/expect, failure injection, virtual time, `verify --sim` — the 27-era full surface | a failure-injected scenario with exact outcomes, any target |
| 06-multi-service.md | `project`, call clients, cross-service streams, system simulation (28's surface), evolution across consumers | the three-service example simulated + system-verified |
| 07-deployment-and-day-two.md | compose/k8s/terraform emission, `verify --system`, generated CI, tracing, rename/backfill ladder | the reader's system compose-verified (Docker required and said so) |

Rules the series holds: guides teach by building one continuous
example across 01–07 (each guide opens from the previous
checkpoint — a stranger who stops after 03 still has a working
thing); reference material is *linked*, never restated (the
guide says "the full verb table lives in expressions.md" —
duplication is how docs rot in stereo); each guide states its
reader and its time budget in the opening lines; and prose is
plain — the milestone-log voice this repository's internal docs
speak (this file included) is the wrong voice for a stranger's
first hour, and the coherence pass reads for it explicitly.

### Guide 01, outlined as the series template

The first guide's skeleton, drafted because it sets the shape
every later guide follows (and the shape is most of what the M5
checkpoint validates):

```text
# Guide 1 — Your first service
*Reader: builder, first hour. Time: ~20 minutes.
 You need: a terminal, nothing else.*

1. Install            (one block, verified; prerequisites
                       stated HERE, not discovered later)
2. Create a project   (`ciac new` — and read what it printed,
                       because M2 made it print the next step)
3. The anatomy        (what's in the directory; which files are
                       yours, which are owned — the manifest
                       discipline in three sentences, linking
                       regeneration.md for depth)
4. Make it yours      (edit the scaffold: rename the service,
                       add a field; `ciac check` after each —
                       teaching the loop, not the language)
5. Build and read     (generate one target; open TWO generated
                       files with guided commentary — the
                       "nothing here is magic" beat)
6. Verify             (`ciac verify` — what it proves, what it
                       doesn't; the honesty culture, taught
                       early)
7. Checkpoint         (the green output the reader should see,
                       verbatim; where to go next: 02, or
                       simulation right away via 05 if that's
                       what hooked you)
```

Beats 3 and 5 are the deliberate personality of the series:
CIaC's differentiation is *owned, readable* generated code, so
the guides put reading generated code on the golden path from
the first hour instead of treating it as advanced material.

### The veracity harness

`scripts/check-guides.sh` (name final at M4): extracts fenced
command blocks marked runnable from README + guides, executes
them in order in a clean workspace per document, and fails on
any non-zero exit — wired into CI as a docs job. Blocks that
cannot run in CI (Docker-needing steps in 07) are marked with an
explicit annotation the harness counts and reports, so the
untested surface is enumerated rather than invisible — the
Docker-delegation honesty, applied to documentation. The
harness is deliberately dumb (no output assertions beyond exit
codes in v1 — output-matching guides are a maintenance trap;
the *checkpoints* assert through `verify`/`sim`, which is what
they are for).

## Pillar 4 — The positioning document

`docs/positioning.md`, for the evaluator, structured as the
honest comparison the punch-list asked for:

- **The one-paragraph thesis**: CIaC occupies the gap between
  frameworks (which give you a language-locked skeleton you
  fill with imperative code) and generators (which scaffold once
  and abandon you): a compiler that permanently owns the
  infrastructure seams of a system it can regenerate, verify,
  simulate, and evolve, in five languages, from one source.
- **vs Rails/NestJS/Spring-alone** (frameworks): you write the
  seams yourself, forever, in one language; CIaC generates and
  owns them, and the language is a choice not a commitment.
  What frameworks have that CIaC does not: ecosystems of
  middleware, a decade of Stack Overflow, escape hatches
  everywhere. Honest.
- **vs Prisma-style / OpenAPI codegen** (generators): they
  scaffold or type one layer once; regeneration is destructive
  or absent; there is no semantic diff, no cross-service
  evolution, no simulation. What generators have: narrower
  claims, shallower learning curve, no new language to adopt.
  Honest.
- **vs low-code/BaaS**: adjacent pitch, opposite mechanics —
  CIaC's output is ordinary code in your repo with no runtime
  platform dependency; the exit cost is zero by construction
  (delete the compiler, keep the code). The trade: no visual
  anything, a DSL to learn.
- **When not to use CIaC**, its own section, not a footnote:
  systems whose core is what the language doesn't model (heavy
  computation, ML pipelines, bespoke protocols); teams unwilling
  to adopt a DSL; projects needing the framework ecosystems'
  depth on day one; anything needing capabilities the ledger
  lists as absent — with a link to the ledger, whose two-table
  honesty is exactly what makes this section credible.
- **The maturity statement**: version, test/verification
  posture, the scanning/release machinery, the disclosed-gaps
  culture — the evaluator's due-diligence paragraph, written
  for them instead of making them assemble it from milestone
  logs.

Drafted by the implementer per the user's decision, reviewed
against one rule: every comparative claim must be checkable
(cite the mechanism, not the vibe), and every "CIaC lacks X"
that is true stays in — the document's credibility *is* its
value, and one oversold paragraph spends it all.

## Pillar 5 — Editor polish: snippets, hovers, quick-fixes, definition

All four features ride the same principle that built `describe`
and the LSP in v0.12–v0.13: **one vocabulary source**
(`crates/ciac/src/vocab.rs`), consumed everywhere, so the editor
can never disagree with the compiler.

### Snippets

Two surfaces, one source: LSP completion items gain
`insert_text` + `InsertTextFormat::Snippet` (tab-stopped bodies
for declaration forms — service/record/table/stream/worker/job/
blueprint/handler skeletons, capability `use` blocks with
provider choices as placeholder alternatives), generated from a
new snippet table in vocab.rs; and `editors/vscode` gains a
`contributes.snippets` file **generated from the same table**
(a small build step in the extension packaging, or a checked-in
file with a test asserting it matches vocab — decided at M7,
recorded; the test either way, because two snippet sources that
can drift is the exact disease vocab.rs exists to cure).
Snippet bodies are checked by the veracity principle too: each
snippet's fully-expanded default form must parse (`ciac check`
on a scratch file per snippet — a unit test, not a manual
promise).

Two snippet bodies, drafted to fix the register (final set at
M7, one per declaration form and capability):

```text
"service": {
  "prefix": "service",
  "body": [
    "service ${1:name} {",
    "  use { ${2|db Postgres,db MySQL,db SQLite|}; }",
    "  $0",
    "}"
  ]
}
"worker": {
  "prefix": "worker",
  "body": [ "worker ${1:Name} on ${2:Stream} {", "  $0", "}" ]
}
```

— tab stops at every choice, provider alternatives as
placeholder options where the vocabulary enumerates them, and
each body's default expansion (`service name { use { db
Postgres; } }` etc.) parse-tested against the real grammar.

### Richer hovers

`doc_for` today returns a sentence; after M7 it returns
structured markdown assembled from the registry: for a
capability — its providers with **per-target support** (derived
from the same source as targets.json, so hover truth is
machine truth), a one-line example, and a deep link to the
reference doc section; for a provider — its config surface
summary; for a builtin verb — signature, behavior line from the
expressions table, and simulation-fake note (the 27 world
contract making it into the editor: "faked under `ciac sim`" is
exactly what a user hovering `email.send` wants to know); for a
declaration keyword — the snippet's skeleton as preview. Hover
content is generated-at-compile-time data, tested like data
(a unit test per vocabulary class asserting shape, not prose).

A hover, as it will render (the `cache` capability, hovered):

```text
**cache** — capability: keyed cache with TTL

Providers: Redis
Targets:   python ✓  rust ✓  typescript ✓  go ✓  java ✓
Verbs:     cache.get, cache.set(ttl?), cache.delete
Simulation: fully faked (TTL against the virtual clock)

    use { cache Redis; }

→ docs/language.md#cache · docs/expressions.md#cache-verbs
```

Every line of that box is derived: providers and target support
from the registry, verbs from the expressions table, the
simulation note from the 27-era world contract, links from a
doc-anchor table added to vocab. Nothing is hand-written per
capability except the one-line description — which already
exists in `doc_for` today.

### Quick-fixes

The structured-fix machinery (v0.15 M7) already turns
diagnostics with `fixes` into code actions; the gap is
coverage. M8 inventories every diagnostic the compiler can emit
(docs/errors.md is the enumeration) and extends structured
fixes to the mechanically fixable class: unknown
capability/provider names (nearest-match suggestion — the
vocabulary is right there), missing required fields with known
shapes, deprecated forms once 26's deprecation ladder has
customers, import path typos against the module loader's known
set, and the scope/name mismatches the sema pass can already
name precisely. The bar for each: the fix must be *the* fix
(no guessing — a quick-fix that might be wrong is worse than
none), and each lands with a fixture test in the existing
structured-fix suite. The inventory itself (which codes have
fixes, which could, which can't and why) is recorded in the M8
Shipped note — the ledger discipline at diagnostic scale.

### Go-to-definition

The flagged addition beyond the user's named three, included
because the machinery makes it nearly free: the rename engine
(v0.18) already resolves an identifier at a position to its
declaration across files (`rename_index`; the LSP already ships
`prepare_rename`/`rename` on it). `definition_provider` becomes
a thin projection of the same index — identifier at cursor →
declaration site location. Streams, records, tables, services,
handlers, blueprints, imports: whatever the rename index
resolves, definition serves. Same-file and cross-file (module
imports) both work because the index already does.

### The small polish that rounds it out

Diagnostics on change (debounced `didChange` reparse — today
diagnostics land on open/save only, which reads as lag in a
modern editor), completion `detail`/`documentation` fields
upgraded alongside hovers, and the VS Code extension version/
packaging refreshed with the arc's version bump. Explicitly not
included: formatting, semantic tokens, workspace symbols,
signature help — candidates for a future polish pass, listed in
Explicit cuts so their absence is a decision.

## Pillar 6 — Prepare for dogfooding

### The reframing, stated honestly

The punch-list item was "dogfooding by someone who isn't the
author"; the discussion resolved it to "prepare for dogfooding"
because a plan cannot manufacture a human. What a plan can do:
make the eventual session cheap, instrumented, and hard to
waste — and fix in advance the frictions a session would
predictably surface, so the real stranger's hour is spent
finding the problems the author *can't* predict, which is the
entire value of an outside user.

### The cold-start transcripts

Three times in the arc (M1 baseline, M5 checkpoint, M9 final),
the same scripted run on a clean container: `curl | sh` install
(real, against the 26-era releases) → README demonstration
end-to-end → guide 01 → guide 05's simulation checkpoint —
executed exactly as written, with every deviation, confusion
candidate, wall-clock, and error message transcribed into a
friction log (`docs/dogfooding/transcripts/` — kept in-repo;
the transcripts are the arc's measurement instrument and their
diffs are its progress metric). The author running a script is
not a stranger — the transcripts don't pretend otherwise; what
they measure is *mechanical* friction (broken commands, missing
prerequisites, misleading output, slow steps) which is the
class an author can find, leaving *conceptual* friction to the
real session.

### The fixes

M2 (and continuing opportunistically): the top of the M1
friction log, fixed. Predictable candidates from the survey
(verified, not assumed, by M1): first-run experience of
`ciac new` (does the scaffold say what to do next?), error
output for the beginner's classic mistakes (the structured-fix
work feeds this), `ciac dev --no-docker`'s messaging, install
prerequisites stated where they're needed. Each fix is ordinary
work in the relevant crate with the friction-log line as its
requirement.

### The kit

`DOGFOODING.md` (repo root): the session script for the
maintainer — recruitment note (who makes a good first tester),
setup (what to give them: a machine with nothing pre-installed,
the README, one hour, no help), the task list (mirroring the
transcript script so results compare), observation prompts
(where did they stop reading; what did they type that failed;
what did they ask), a feedback capture template, and the
triage rule for what they find (friction → issue with the
`dogfooding` label; conceptual confusion → docs issue; bug →
bug). Plus `.github/ISSUE_TEMPLATE/` (bug report, docs
friction, feature request — the repo has none today) so what
the session finds has somewhere structured to land. The kit's
exit criterion is blunt: the user could hand DOGFOODING.md and
a laptop to a colleague tomorrow and run the session without
preparing anything else.

### DOGFOODING.md, outlined

```text
# Running a dogfooding session

## Who to recruit
A backend developer who has never seen CIaC. Not a friend of
the project. Comfort with a terminal; no Rust knowledge needed.

## Setup (before they arrive)
- A machine or fresh VM with nothing pre-installed beyond a
  shell and a browser. Do NOT pre-install toolchains.
- Give them: the repository URL. Nothing else.
- Set aside: 60–90 minutes. You observe; you do not help.

## The session
Phase 1 (0:00–0:20) — cold start: "Get this installed and make
  it do something." No further instruction. OBSERVE: where do
  they land first, what do they read, what do they skip.
Phase 2 (0:20–0:50) — guided build: hand them Guide 01, then
  02. OBSERVE: every place they stop, re-read, or type
  something the guide didn't say.
Phase 3 (0:50–1:10) — the hook: "Make a request fail on its
  third attempt and prove what happened." (Simulation, guide
  05.) OBSERVE: do they find sim; does the scenario format
  make sense unprompted.
Debrief (1:10–) — the five questions:
  1. What is this tool, in your words?
  2. What almost made you quit?
  3. What surprised you, good or bad?
  4. Would you use it? For what? Why not?
  5. What did you want that wasn't there?

## Capturing
One observation per line in feedback-log.md (template below),
tagged {friction|concept|bug|want}. File issues with the
matching template; label `dogfooding`.

## Rules for the observer
Silence except for phase transitions. Every time you feel the
urge to explain something, write down WHAT needed explaining —
that urge is the data.
```

The skeleton is the deliverable's spine; M9 completes it (the
feedback-log template, the issue-template links, the recruit-
thank-you note) against the exit criterion already stated:
runnable tomorrow, zero additional prep.

## Pillar 7 — Coherence, index, and the docs that already exist

The arc's changes touch every entry point, so M6 runs a
coherence pass over the whole docs surface: `docs/README.md` (or
an index section in the main README — decided by where GitHub
renders best, recorded) listing every doc with its reader and
one line; each existing doc gains its reader statement;
terminology unified (the pass greps for the drift candidates:
"backend"/"target", "capability"/"component", "example"/
"program" — one term wins per concept, the glossary in guide 01
records the winners); stale cross-references from three arcs of
churn fixed (the ledger's two tables, simulation.md's rewritten
status, deployment.md's OAuth2 rewording all landed recently —
the pass verifies the links that point at them); and
backend-spike-report.md — a v0.8 artifact of purely historical
interest — moves under a `docs/history/` grouping with
history.md rather than sitting beside living reference docs.
Nothing is deleted; everything is findable from one index;
every doc knows its reader.

## Implementation map

| Area | Changes |
| --- | --- |
| `README.md` | full rewrite per Pillar 2; <250-line budget |
| `docs/history.md` (+ `docs/history/`) | the version narrative, relocated; spike report grouped |
| `docs/guide/01..07-*.md` | the series, new |
| `docs/positioning.md` | new |
| `docs/README.md` (or index section) | the docs index, new |
| `scripts/check-guides.sh` + CI docs job | the veracity harness, new |
| `crates/ciac/src/vocab.rs` | snippet table; structured hover data; per-target support derivation |
| `crates/ciac/src/lsp.rs` | snippet completions; rich hovers; `definition_provider` off the rename index; debounced didChange diagnostics |
| `crates/ciac/src/describe.rs` | consumes the enriched vocab (additive fields; DESCRIBE_VERSION unchanged if additive holds) |
| structured fixes (ciac-sema/diagnostics) | quick-fix coverage extension + fixture tests |
| `editors/vscode/` | contributes.snippets (generated-or-tested against vocab); extension refresh |
| `docs/dogfooding/transcripts/` | three transcripts + friction logs |
| `DOGFOODING.md` | the session kit, new |
| `.github/ISSUE_TEMPLATE/` | bug / docs-friction / feature templates, new |
| friction fixes | wherever M1's log points (ciac new output, error messages, dev messaging) |

## Reader-contract checklist

- Evaluator: README narrative <250 lines, demonstration
  harness-verified; positioning doc with checkable claims and a
  real when-not-to section.
- Builder: guides 01–07, one continuous example, every runnable
  block harness-executed, checkpoints green on the documented
  commands.
- Editor user: snippets (parse-tested), hovers carrying
  machine-derived support truth, quick-fix inventory recorded
  with fixture tests, go-to-definition on everything the rename
  index resolves.
- Maintainer: three transcripts with measured friction deltas;
  DOGFOODING.md complete per its exit criterion; issue templates
  live.
- Integrator: describe/MCP/AGENTS.md surfaces unchanged or
  additively enriched; the index makes them findable.

## Determinism and supply chain

No new runtime dependencies anywhere. The veracity harness is
shell + the real binary; the snippet/hover work is data in
vocab.rs; the extension's packaging pins hold. The docs CI job
consumes the same toolchain setup existing jobs pay for. The
one supply-chain-adjacent note: the README's `curl | sh` line
is now load-bearing marketing — the M1/M5/M9 transcripts
exercise it against the real release each time, which makes
install rot a measured quantity three times per arc.

## Diagnostics, gating, and docs impact

No new error codes; the quick-fix work adds `fixes` to existing
codes (docs/errors.md rows gain a "quick-fix: yes" marker — the
inventory made visible). The LSP capability announcement grows
(`definition_provider`, snippet support in completion) —
additive, no client breakage expected, verified against the VS
Code client in M7/M8. Docs impact *is* the arc; the map above
enumerates it. One meta-note recorded here for future arcs: the
README's budget and the docs index's reader labels become part
of the standing docs discipline — every future arc's docs
milestone updates the index and respects the budget, the same
way every arc already updates the ledger.

## Relationship to the forecast documents

The punch-list's Tier 3, executed per the discussion's
decisions: rewritten-README-plus-guides (chosen over a hosted
site — revisitable later; the guide content is the hard part
and hosting is a rendering decision), dogfooding reframed as
preparation with the real session explicitly the user's,
editor polish at full scope (all three named items, plus
definition flagged as the plan's addition), positioning drafted
by the implementer. Sequenced last of the four plans so every
sentence the front door speaks describes the finished system —
the README this arc writes would have been a lie in v0.23 and
is merely the truth in v0.27. This is also the arc that ends
the planned sequence: its handoff is not to a fifth plan but to
the dogfooding session itself, whose findings — not another
armchair punch list — should shape whatever is planned next.

## What this arc is predicted to cost

Predictions, reconciled in M9's retrospective:

| Workstream | Predicted size |
| --- | --- |
| README + history extraction (M3) | the writing is the work; ~250 new lines + relocation; one new example program ×5 |
| Guide series (M4, M6) | the arc's bulk: seven guides, one continuous example; the harness (~100 lines of shell) amortizes across all of them |
| Positioning (M6) | one document; the review rule is the cost |
| Transcripts + fixes (M1, M2, M5, M9) | three scripted runs; the fix queue is unknowable until M1 — the arc's honest variable |
| Snippets + hovers (M7) | vocab data + LSP wiring; the drift/parse/shape tests are half the line count |
| Fixes + definition + rounding (M8) | inventory-driven; definition is thin over the rename index; the didChange debounce is the only new LSP mechanics |
| Kit (M9) | documents with exit criteria |

### Predicted golden churn

| Milestone | Expected churn |
| --- | --- |
| M2 | `ciac new` scaffold goldens + any diagnostic-text goldens the friction fixes touch |
| M3 | the demonstration example's goldens ×5 (new example) |
| M7–M8 | none in generated projects (compiler-side LSP/vocab; describe JSON snapshot updates only) |
| others | none |

### The config/env surface

None. The arc adds no environment variables, no config rows, no
generated-project surface of any kind — the one arc in the
sequence where that sentence is trivially true, recorded anyway
for the pattern's sake.

## Milestones

Nine milestones: measure, fix, then build the front door
outside-in (README → guides → positioning), re-measure at the
checkpoint, polish the editor, and close with the kit and the
final measurement. Standing discipline throughout (full
verification, golden review where generation is touched — the
LSP/vocab work is compiler-side and golden-neutral; commit +
push; in-place Shipped notes).

1. **M1 — Baseline cold-start transcript.** The scripted run on
   a clean container against the current (v0.26-era) front
   door: install → current README's quick start → the closest
   thing to the guide-01 path that exists today. Full friction
   log committed (`docs/dogfooding/transcripts/01-baseline.md`)
   with wall-clocks and verbatim confusing output. The log is
   triaged into: fix-now (M2's queue), fix-via-rewrite (Pillars
   2–3 absorb it), defer-with-reason. No fixes in this
   milestone — measurement first, so the arc's deltas mean
   something.

   **Shipped (v0.29 M1) — six findings, zero fixes, as designed.**
   `docs/dogfooding/transcripts/01-baseline.md` records the script
   run for real against the live binary in this session's sandbox
   (with an honest caveat: a warm `cargo`/`uv` cache, not a bare
   machine — called out per-step where it matters). Six findings,
   F1-F6. The two fix-now items (M2's queue): **F4**, the highest-
   priority finding in the transcript — the scaffolded README's own
   documented third step, `ciac verify`, fails immediately on a
   freshly generated, untouched project with 18 ruff lint errors
   (`B008`/`I001`/`UP037`) inside generated code the reader never
   touched. Root cause confirmed by inspection, not guessed:
   `crates/ciac-backend-python/templates/pyproject.toml.j2` pins
   `"ruff>=0.6"` with no upper bound, no lockfile, and a
   `[tool.ruff]` block that sets only `target-version` — no explicit
   `select`. `uv run ruff --version` resolves 0.16.0 today; ruff's
   own default/implied rule set picked up findings between whenever
   the templates were last hand-verified and now. Confirmed not
   scaffold-specific: the same 18 errors at the same lines reproduce
   against the checked-in `examples/crud-notes.ciac` directly (the
   exact program `ciac new --template crud` embeds verbatim, per
   `docs/authoring.md`), meaning this breaks `ciac verify` on every
   fresh Python project today, not a scaffold edge case — and no
   existing test catches it, since `crates/ciac/tests/scaffold_cli.rs`
   asserts scaffolds pass `ciac check` only, never `ciac verify`,
   and CI's example sweep verifies `examples/*.ciac` without pinning
   ruff any tighter than the template does. **F5**: `ciac dev`, run
   exactly as the top-level README documents (no flags), produces
   zero output for 8-20s when Docker's daemon is unreachable — a
   realistic "clean container" state this very sandbox is in (the
   `docker` CLI exists, no daemon) — before its own clear failure
   message finally surfaces; `--no-docker` reports instantly but
   isn't mentioned in the quick start. Three fix-via-rewrite items,
   each already homed in a later milestone rather than reopened
   here: **F1** (the `curl \| sh` 404 — expected, already disclosed
   in 26 M8/M9, the real fix is M9's actual release cut), **F2**
   (the `cargo install` fallback is a silent >100s wait with no
   framing that it needs a Rust toolchain or how long it takes —
   Pillar 2's job), **F6** (`docs/authoring.md`, read cold as
   today's nearest guide-01 substitute, is stale — still titled
   `v0.13` and still claims rename/code-actions are "deliberately
   out of scope" for `ciac lsp`, false since v0.15 M7 and v0.18 —
   Pillar 3's guide-01 supersedes it and Pillar 7's coherence pass
   catches the staleness generally). Zero defer-with-reason items —
   every finding had a home already. F3 is recorded as a positive
   baseline (scaffold/check/build messaging is already good,
   sub-10ms) so M5/M9 have something to *not* regress, not just
   things to fix. No code changed this milestone, per M1's own rule.

2. **M2 — Friction fixes, round one.** The fix-now queue
   executed: the predictable candidates (scaffold next-steps
   output, beginner error messages, dev-loop messaging,
   prerequisite statements) plus whatever M1 actually found —
   each fix carrying its friction-log line as the requirement
   and landing with the normal test discipline (golden-visible
   where output text changes; the `ciac new` scaffold is
   golden-snapshotted already). Exit is the queue empty or
   explicitly deferred-with-reason, not vibes.

   **Shipped (v0.29 M2) — F4 and F5 fixed, plus a third bug the
   fix's own verification caught.** **F4** (the highest-priority
   finding — `ciac verify` failing on every fresh Python project):
   `crates/ciac-backend-python/templates/pyproject.toml.j2` gained
   an explicit `[tool.ruff.lint] select = ["E4", "E7", "E9", "F"]`
   — ruff's own long-documented default selection, pinned rather
   than left to whatever ruff resolves at generation time. Verified
   directly: `ruff==0.6.9` (near the old `>=0.6` floor) already
   passed the generated `crud` project clean with no explicit
   select, confirming the regression was ruff's own default
   widening between 0.6 and today's 0.16.0, not a template defect;
   adding the explicit select made 0.16.0 pass the same project
   clean too, without touching a single line of generated code.
   **F5** (the silent multi-second gap before `ciac dev` reports
   anything when Docker's daemon is unreachable): `crates/ciac/src/
   dev.rs` gained one `eprintln!("dev: starting the compose
   stack...")` immediately before the `docker compose up`
   invocation — the only signal a reader gets between "regenerated"
   and whatever Docker reports next. Verifying F4 across the full
   corpus (a sweep script run against all 28 `examples/*.ciac`
   under `--target python`) caught a third, unpredicted bug:
   `traced-checkout.ciac` failed with 7 `E402` (module-level import
   not at top of file) errors — `observability.py.j2` interleaved
   each capability's imports with that capability's function body
   (`{% if has_logging %}` imports + `configure_logging()`, then
   `{% if has_tracing %}` imports + `configure_tracing()`), so
   whenever two of logging/metrics/tracing were both present, the
   second capability's imports landed textually after the first
   capability's function definition. Not caught by F4's own crud
   template test (which has no tracing) or by the original M1
   transcript (which never generated a tracing-enabled project).
   Fixed by restructuring the template into two passes — all
   capability-gated imports first, then all capability-gated
   function/statement bodies — rather than one pass per capability.
   The sweep before this third fix was 27/28 green (only
   `traced-checkout` failing); after, 28/28. All three fixes are
   golden-visible (28 `golden__gen__python__*.snap` files
   regenerated via `cargo insta test`, reviewed diff-by-diff:
   every diff was exactly the `[tool.ruff.lint]` block and, for the
   tracing-bearing examples, the import/body reordering — nothing
   unexpected). Full verification green: `cargo fmt --check`,
   `cargo clippy --workspace --all-targets -- -D warnings` (zero
   warnings), `cargo test --workspace` (14/14 test binaries `ok`,
   zero failures). Fix-now queue is empty — F4 and F5 were the only
   two items M1 triaged there, and both are closed, with F4's own
   fix disclosing and closing a third bug along the way rather than
   leaving it for a later milestone to rediscover.

3. **M3 — The README rewrite.** Pillar 2 executed: narrative
   shape, demonstration program checked in as an example
   (verifying ×5 like any example), history extracted to
   docs/history.md, <250-line budget met, every runnable block
   annotated for the harness (which lands next milestone — the
   annotations are designed here). Reviewed against the
   evaluator reader-model: someone who reads only this file
   knows what CIaC is, saw it work, and knows where the
   boundaries live.

   **Shipped (v0.29 M3) — the rewrite, a real demonstration
   example, and one incidental fix.** `README.md` went from 366
   lines (194 of them a version-by-version history) to 203 —
   well inside the <250 budget — following Pillar 2's own shape:
   claim + anti-claim paragraph, a fifteen-minute walkthrough,
   the map (five-target table plus one paragraph each for
   simulation/deployment/evolution/the agent front door), and a
   "where to go next" pointer section. The demonstration is a
   new checked-in example, `examples/quickstart.ciac` — one
   record, free `crud`, one custom handler wrapped in a
   `transaction`, one stream, one worker — plus
   `sim/quickstart.ciac-sim.json`, a failure-injection scenario
   (fail the archive's own `db.commit` once, assert the audit row
   absent, retry, assert present). Both verified live, not just
   golden-snapshotted: `ciac check`, `ciac build`, and `ciac sim`
   with the checked-in scenario all pass on **all five targets**
   (python, rust, typescript, go, java) — the README's own "swap
   `--target` and everything below still holds" claim is
   demonstrable, not asserted, because this session ran it on
   each target and got `[PASS] 29-m3-quickstart` every time.
   Wired into the standing regression surface the same way every
   prior flagship example was: added to `scripts/sim-corpus-x5.sh`
   and to `.github/workflows/ci.yml`'s `generated-sim` job. One
   genuinely surprising design snag, resolved and worth recording:
   the sketch in this plan's own Pillar 2 combined an unbound
   `crud Note;` with a hand-written `api CreateNote`, which the
   real grammar can't do — `crud <Name>: <Record>;` owns its
   bound record's table privately (confirmed by building
   `examples/sqlite-notes.ciac` and inspecting the generated
   model), so a second declaration of the same table collides.
   The shipped example resolves this the way a real author would:
   `crud Note: Note;` for the free CRUD surface, and a separate
   `ArchiveEvent`/`ArchiveEvents` table for the one piece of
   custom logic — arguably a better demonstration than the
   sketch's own version, since it shows generated CRUD and custom
   business logic coexisting rather than colliding. `docs/
   history.md` now carries the old narrative essentially
   verbatim (retitled, the version list's tail extended through
   v0.26 to close the gap the old README's own last entry left).
   The runnable-block annotation format is designed here (not
   built — M4's job): an `<!-- ciac-verify:start id=NAME -->` /
   `<!-- ciac-verify:end -->` HTML-comment pair around each
   fenced command block, invisible in rendered Markdown and
   trivially greppable by id — used on all four command blocks
   in the walkthrough. Deliberately **not** linked from the new
   README: `docs/positioning.md` (Pillar 4, lands M6) and a guide
   series (Pillar 3, lands M4/M6) — both would be dead links
   today, so the "where to go next" section links only what
   exists, and the coherence pass (M6) is where those references
   get added as the files land. One incidental fix, found while
   writing the simulation paragraph and cross-checked against
   `ciac sim --help`: `crates/ciac/src/main.rs`'s `Sim` subcommand
   doc comment still claimed only `python`/`rust` fake simulation
   capabilities (stale since 27UpdatePlan.md brought TypeScript,
   Go, and Java to full parity) — corrected to name all five.
   Evaluator reader-model review, stated explicitly per this
   milestone's own bar: a reader of README.md alone now gets what
   CIaC is (paragraph one's claim), sees it work (the walkthrough
   ends in real, checked `[PASS]` output, not a promise), and
   knows the boundary (the anti-claim paragraph, stated before
   any install command). Full verification green: `cargo fmt
   --check`, `cargo clippy --workspace --all-targets -- -D
   warnings` (zero warnings), `cargo test --workspace` (14/14
   test binaries `ok`, zero failures) — including 12 new golden
   snapshots for `quickstart` (ir/dot/five gen/four host-syntax-
   identity/one ts-client) with zero existing snapshots
   perturbed, confirmed by diff before accepting.

4. **M4 — Guides 01–03 + the veracity harness.** The first
   three guides (install/anatomy, records/CRUD, handlers/
   logic), building the continuous example; the harness
   (`scripts/check-guides.sh`) landed and wired into CI running
   README + guides 01–03; the runnable-block annotation format
   frozen and documented in a contributor note. The
   harness-can-fail proof (a deliberately broken block in a
   scratch branch) demonstrated, per the 26 M6 tradition.

   **Shipped (v0.29 M4) — the harness landed, and its very first
   real run against the README caught a bug real enough to have
   broken the arc's own centerpiece.** Three guides —
   `docs/guide/01-first-service.md` (install/anatomy/first field,
   `--template minimal`'s `Ping`/`Message`), `02-records-and-crud.md`
   (`crud Message: Message;` for free persistence, then a schema
   change previewed with `ciac diff` before rebuilding), and
   `03-handlers-and-logic.md` (a `transaction`-wrapped handler, a
   `ReadReceipt` table, a stream, a worker) — one continuous
   example, each guide independently self-contained (a full,
   current `main.ciac` written fresh, not a diff from the previous
   guide) per the plan's own "clean workspace per document" rule.
   `scripts/check-guides.sh`: builds `ciac` once, then per document
   creates a temp workspace with `examples/`/`sim/` symlinked in
   from the real repo (so a block that says `examples/quickstart.
   ciac` resolves exactly as it would for someone who cloned this
   repository) and executes every annotated block in order. Final
   annotation format, frozen here as M4's own text promised:
   `<!-- ciac-verify:file id=NAME path=REL/PATH -->` (write the
   fenced block's content to a file), `:start id=NAME` (run it as
   shell, fail the harness on nonzero exit), `:skip id=NAME
   reason="..."` (counted and reported by name, never silently
   dropped — used for the install curl line, which needs a cut
   release the harness doesn't have yet, and for `ciac dev`, a
   watch loop with no exit code an exit-code-only harness can
   check). Harness-can-fail proof: a `/tmp` scratch copy of guide
   01 (not a git branch — same isolation, no git state touched)
   with `ciac check main.ciac` swapped for a nonexistent
   subcommand; the harness reported `[FAIL]` with the real
   `unrecognized subcommand` text and exited 1, confirmed live in
   this session.

   The headline finding, though, wasn't a guide bug — it was in the
   README the harness ran first. `ciac sim examples/quickstart.ciac
   --target python --out ./build --scenario ...` (the README's own
   walkthrough, relative `--out` matching its own `build`/`verify`
   lines) failed with `ModuleNotFoundError: No module named 'app'`.
   Root cause, traced in `crates/ciac/src/commands.rs`: all five
   per-target sim drivers (`sim_drive_python`/`_rust`/`_typescript`/
   `_go`/`_java`) call `find_project_dirs(out, marker)` with the
   raw, possibly-relative `out` path; the returned `project_dir`
   then crosses into a subprocess with its *own*, different cwd
   (concretely, Python's driver builds its `PYTHONPATH` from that
   relative string), so a relative `--out` silently re-resolved
   against the wrong directory once inside the child process. Fixed
   by wrapping all five `find_project_dirs` call sites in
   `resolve_path(out)?` — a helper that already existed in this
   same file for exactly this "crossing a subprocess-cwd boundary"
   class of problem, just not yet applied here. Verified live for
   all five targets this session with a relative `--out ./build`
   (python/typescript/go fast; java clean past its own proxy
   startup noise; rust needed a longer timeout for its own cargo
   compile, not a retry — same command, same result once given
   time). This bug would have broken the literal walkthrough this
   arc's own M3 just finished writing, for any real reader who
   cloned the repo and typed the commands as documented — the
   single most consequential finding of the milestone, and one M1's
   own transcript had no way to predict (its own sim step used an
   absolute scratch path, the one difference between "measuring
   friction" and "running the exact block a reader would run").
   CI wiring: a new `check-guides` job in `.github/workflows/
   ci.yml`, positioned right after `generated-sim`. Full
   verification green: `cargo fmt --check`, `cargo clippy
   --workspace --all-targets -- -D warnings` (zero warnings),
   `cargo test --workspace` (14/14 test binaries `ok`, zero
   failures), and `scripts/check-guides.sh` itself green against
   all four documents (README + guides 01-03: 15 blocks run, 3
   skipped and disclosed, 0 failed).

5. **M5 — CHECKPOINT: transcript two.** The same scripted run,
   now against the new README + guides 01–03: measured friction
   delta against M1 (steps failed, confusions hit, wall-clock),
   committed as transcript 02. The checkpoint decision: go
   (guides 04–07 proceed on the validated voice/shape),
   adjust (the transcript says the shape is wrong — the
   remaining guides change shape before being written, cheaper
   now than after), or re-fix (new fix-now queue from the new
   findings, absorbed before M6). This is the arc's
   empathy-risk gate: it cannot prove a stranger succeeds, but
   it can prove the author's best stranger-simulation does.

   **Shipped (v0.29 M5) — go, with one small re-fix caught and
   closed on the spot.** `docs/dogfooding/transcripts/
   02-checkpoint.md` re-ran the same script for real (install →
   README walkthrough → guides 01–03, the last of which didn't
   exist at M1) against a fresh `cargo install` of the current
   tree. F1 (no release) and F6 (`authoring.md` staleness) are
   unchanged, as expected — neither was scoped to close before M9/
   M6. F2, F4, and F5 are confirmed fixed under fresh measurement,
   not just a harness re-run: the README's install block now states
   the toolchain requirement and a rough time inline (F2); `ciac
   verify` on both the quickstart example and a fresh guide-01
   service passes clean, no ruff errors (F4); `ciac dev`'s
   previously-silent gap now shows `dev: starting the compose
   stack...` immediately (F5). One real number worth recording
   honestly rather than smoothing over: the `cargo install`
   fallback measured 2m41s this run against 1m40s at M1 — both
   warm-cache readings, the difference almost certainly this
   session's own source churn forcing more recompilation, not a
   regression, and well inside the README's own "~2 minutes"
   framing.

   One new finding, **F7**, caught by re-reading the guide series
   with fresh eyes rather than by the harness (which checks command
   blocks, not prose links): `docs/guide/01-first-service.md` and
   `03-handlers-and-logic.md` linked forward to `05-simulation.md`
   and `04-streams-and-workers.md` — files that don't exist until
   M6. The exact mistake M3's own README rewrite had deliberately
   avoided (no links to `docs/positioning.md` or the guide series
   before they exist) hadn't been carried into the guides' own
   cross-references to each other's future installments. Fixed live
   during this milestone — replaced with plain, unlinked mentions
   ("a later guide in this series...") — re-verified by grepping
   `docs/guide/*.md` and `README.md` for `0[4-7]-`: zero matches.
   Re-ran `scripts/check-guides.sh` after the fix: still 15 blocks
   run, 3 skipped (disclosed), 0 failed — the text-only fix changed
   no command behavior.

   **Checkpoint decision: go.** Pillar 2/3's narrative shape and
   voice hold up under a second, independent read; every M1 fix-now
   item is closed and re-confirmed; the one new finding was minor,
   caught by the checkpoint's own discipline, and closed without
   needing to reopen an earlier milestone. Guides 04–07 proceed at
   M6 on the validated shape. No code changed this milestone — docs
   only (the two guide files' cross-reference text, and the new
   transcript); `cargo fmt`/`clippy`/`cargo test --workspace` were
   not re-run since nothing they check was touched, and
   `scripts/check-guides.sh`'s own green run is the milestone's real
   verification.

6. **M6 — Guides 04–07, positioning, coherence.** The remaining
   guides (streams/workers, simulation at 27 depth,
   multi-service at 28 scope, deployment/day-two) under the
   harness; docs/positioning.md per Pillar 4's structure and
   honesty rule; the coherence pass (index, reader statements,
   terminology unification with the glossary, cross-reference
   verification, history grouping). The docs surface is
   complete at this milestone's exit; what follows is editor
   and kit.

7. **M7 — Snippets and rich hovers.** The vocab.rs snippet
   table + structured hover data (per-target support derived
   from the registry — the machine-truth rule); LSP snippet
   completions + rich hovers; contributes.snippets with the
   drift test; every snippet's expansion parse-tested; hover
   shape unit tests; describe's additive enrichment verified
   against DESCRIBE_VERSION compatibility. Manual verification
   in real VS Code recorded (screenshot-level sanity — the LSP
   tests prove protocol, a human proves rendering).

8. **M8 — Quick-fixes, go-to-definition, and LSP rounding.**
   The diagnostic inventory (which codes have/could/can't have
   fixes, recorded); structured-fix extension over the
   mechanically-fixable class with fixture tests per fix;
   `definition_provider` off the rename index (unit tests over
   the same fixtures rename uses; cross-file via imports
   proven); debounced didChange diagnostics; extension refresh.
   The LSP capability set after this milestone is the arc's
   final editor claim — recorded in authoring.md's LSP section,
   which this milestone rewrites to match reality.

9. **M9 — The kit, the final transcript, version, and the arc
   close.** DOGFOODING.md to its exit criterion; issue
   templates; transcript three (the full path: install → README
   → guides through 05, on the release the arc is about to
   cut... run against the release candidate, with the delta
   table 01→02→03 as the arc's headline metric); version
   **0.26.0 → 0.27.0** (workspace + pins, vscode manifest —
   which this arc actually changed — language.md compiler
   parenthetical; language still 1.0.0); full verification;
   retrospective appended after a rule — the friction-delta
   table, the docs-surface before/after (files, readers,
   harness coverage), what the transcripts could and could not
   measure, and the handoff: **the next plan file should not be
   written until a real outside human has run the DOGFOODING.md
   session**, because every armchair-derivable improvement now
   has either shipped or been explicitly cut, and the marginal
   value of planning without new evidence is negative. That
   sentence is the arc's, and the sequence's, deliberate last
   word.

### Per-milestone exit checklists

- **M1 exits when:** transcript 01 committed with wall-clocks
  and verbatim output; triage recorded into the three queues.
- **M2 exits when:** the fix-now queue is empty or
  deferred-with-reason; each fix cites its friction line; tests
  green (goldens where output changed).
- **M3 exits when:** README <250 lines, demonstration example
  verifies ×5, history.md carries the narrative, annotations in
  place, evaluator-model review recorded.
- **M4 exits when:** guides 01–03 harness-green in CI; the
  harness failure-proof demonstrated; annotation format
  documented.
- **M5 exits when:** transcript 02 committed; the delta table
  vs M1 computed; the go/adjust/re-fix decision recorded in
  this file.
- **M6 exits when:** guides 04–07 harness-green (Docker blocks
  annotated and counted); positioning.md live with every
  comparative claim checkable; index + reader statements +
  terminology pass complete; no dangling cross-references
  (link check in the harness run).
- **M7 exits when:** snippets parse-tested and drift-tested;
  hovers carry registry-derived support data with shape tests;
  VS Code manual verification recorded.
- **M8 exits when:** the fix inventory is recorded; new fixes
  fixture-tested; definition green on rename's fixtures incl.
  cross-file; didChange diagnostics debounced and tested;
  authoring.md matches reality.
- **M9 exits when:** DOGFOODING.md meets its exit criterion;
  templates live; transcript 03 + the delta table committed;
  version bumped; retrospective appended ending with the
  no-plan-before-dogfooding handoff.

## Open questions resolved at implementation (pre-registered)

1. **contributes.snippets: generated at package time vs
   checked-in with a drift test** — decided at M7 by the
   extension build's realities; the drift test exists in either
   case.
2. **Docs index placement** — `docs/README.md` vs a section of
   the main README; decided at M6 by GitHub rendering behavior,
   with the <250-line budget as a constraint.
3. **Demonstration program** — a new compact example vs an
   existing one; decided at M3 by whether any existing example
   is genuinely README-sized (bias: new, small, purpose-built —
   the corpus gains one more ×5-verified program).
4. **Harness annotation syntax** — HTML comment markers vs
   fence-info strings on code blocks; decided at M4 for
   renderer-invisibility and grep-ability.
5. **didChange debounce interval and reparse cost** — measured
   at M8 against the real parser on the largest example;
   interval chosen from data.
6. **Quick-fix scope line** — the inventory decides which codes
   get fixes this arc; the pre-registered bar ("must be *the*
   fix") is the criterion, and the recorded inventory is the
   deliverable either way.

## Verification strategy

The arc's novelty is oracles for prose: the veracity harness
(every runnable block, CI-executed, annotated exceptions
counted), the three transcripts (measured friction, deltas
computed, committed as data), the budget checks (README line
count in the harness), the link check, and the
parse-test/drift-test/shape-test trio on the editor data. The
code-side work (LSP, fixes, vocab) carries the normal
discipline: unit and fixture tests, clippy/fmt/test workspace
green, no golden churn expected outside `ciac new` scaffold
output and any friction-fix message changes — both reviewed
normally.

The proof ledger by layer:

| Claim | Oracle |
| --- | --- |
| the README demonstration works | harness-executed on every CI run; demonstration example verifies ×5 |
| guides don't lie | harness per guide; Docker-block exceptions counted and reported |
| the front door improved, measurably | transcripts 01→02→03 delta table |
| snippets are valid language | per-snippet parse test |
| editor truth = compiler truth | hover/snippet data derived from vocab/registry; drift tests |
| quick-fixes are correct fixes | fixture test per fix; the "must be *the* fix" bar in review |
| definition resolves correctly | rename-index fixtures reused; cross-file cases |
| positioning claims are checkable | the per-claim mechanism-citation rule at review |
| the kit is complete | DOGFOODING.md exit criterion: runnable tomorrow with zero prep |
| docs stay coherent | index + reader statements + link check in CI |

## Milestone dependencies and parallelism

M1→M2 sequential (measure, then fix). M3 after M2 (the rewrite
absorbs fix-via-rewrite items). M4 after M3 (harness design
rides the README's annotations). M5 after M4, hard gate. M6
after M5. M7/M8 are independent of the docs track entirely and
may run parallel to M3–M6 (different files, different skills,
no shared state except vocab.rs which only they touch); listed
late so the docs track — the arc's core — gets first attention,
but a stalled docs review should not idle the editor work. M9
last, needing everything.

## Explicit cuts

No hosted docs site (revisitable; content first). No videos or
interactive tutorials. No formatting provider, semantic tokens,
workspace symbols, or signature help in the LSP (listed so
their absence is a decision; candidates for a future pass). No
community bootstrapping work (Discord/discussions/blog — 
distribution is not this arc). No translation/i18n. No
positioning A/B or marketing-channel work — one honest document,
not a campaign. No attempt to simulate a user the arc cannot
have: the transcripts measure mechanical friction and say so;
conceptual friction waits for the real session. No new language
or generation surface of any kind — the front door describes;
it does not add.

## Risks

- **Author empathy fails silently — the docs read well to their
  writer and opaquely to everyone else.** The structural
  mitigations (transcripts, harness, reader statements, the M5
  gate) catch the mechanical half; the conceptual half is
  explicitly deferred to the real dogfooding session, and the
  arc's last word (M9) blocks further planning until that
  session happens — the risk is bounded by refusing to pretend
  it's solved.
- **The guides rot as the system evolves.** The harness makes
  rot loud (a broken block fails CI the day it breaks), the
  continuous-example design concentrates maintenance in one
  program, and the standing-discipline note makes docs-index
  updates part of every future arc's docs milestone.
- **Snippet/hover data drifts from the compiler.** Single
  source (vocab/registry) + drift tests; the same cure the
  LSP already proved for keywords.
- **Quick-fix overreach** — a wrong fix teaches distrust
  faster than no fix teaches patience. The "must be *the* fix"
  bar, fixture tests, and the recorded inventory (including
  the deliberate can'ts) hold the line.
- **The positioning doc oversells.** The checkable-claim rule
  and the mandatory when-not-to section; the reviewer
  instruction is written in the pillar: one oversold paragraph
  spends the document's entire value.
- **The editor work distracts from the docs core.** The
  parallelism note makes the docs track first-priority
  explicitly; M7/M8 are structured to be pausable without
  blocking M9's kit (whose dependencies are docs-side only).

## Confidence and handoff

High on everything mechanical, which is most of the arc: the
docs have contracts and oracles, the editor work rides proven
machinery with drift tests, the kit is a document with an exit
criterion. Medium — honestly and irreducibly — on the empathy
question, which no amount of milestone discipline converts to
high: the arc's transcripts are the best stranger-simulation an
author can run, and the plan's whole shape concedes that the
real answer arrives only when a real stranger sits down. That
concession is the handoff: this arc ends the four-plan sequence
(26: trust; 27: depth; 28: topology; 29: the door), and its
final milestone deliberately refuses to name a successor plan.
The next plan file is written after — and from — the dogfooding
session DOGFOODING.md exists to make easy. What the project
needs next is no longer more of its own judgment; it is
evidence, and everything is now in place to collect it cheaply.
