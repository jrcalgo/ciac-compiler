# Positioning: when to reach for CIaC, and when not to

*Reader: the evaluator — someone deciding whether to adopt this tool,
not yet using it. Every comparative claim below cites the mechanism
that backs it; where CIaC genuinely lacks something a competitor has,
that stays in this document rather than getting quietly omitted —
the [divergence ledger](backends.md#divergence-ledger) this document
links to is what makes that promise checkable, not just stated.*

## The one-paragraph thesis

CIaC occupies the gap between **frameworks** (which give you a
language-locked skeleton you fill with imperative code, forever) and
**generators** (which scaffold once, at the start, and then abandon
you to hand-maintenance). It's a compiler you keep running against a
single source file for the system's whole life: it owns the
infrastructure seams — auth, database access, queues, HTTP routing,
compose/k8s/Terraform — and regenerates, verifies, simulates, and
evolves them, in five languages, from that one source, for as long as
the system exists.

## vs Rails / NestJS / Spring-alone (frameworks)

With a framework, you write the seams yourself — auth middleware,
ORM wiring, queue consumers, the compose file — by hand, in one
language, and you own that code forever, including every time a
seam's shape needs to change everywhere it's used. CIaC generates and
owns those seams instead: change `use { auth JWT; }` to
`use { auth OAuth2; }` and every generated route, test, and dev-realm
config updates from the one line — see the provider table in
[docs/language.md](language.md). The language becomes a choice, not
a commitment: the same `.ciac` source targets Python, Rust,
TypeScript, Go, or Java (`ciac build --target <name>`), at parity.

**What frameworks have that CIaC does not, honestly:** a decade of
Stack Overflow answers for exactly your error message; ecosystems of
drop-in middleware for anything not in CIaC's own ontology; escape
hatches everywhere, because the whole codebase is already yours to
edit, not just the handler stubs.

## vs Prisma-style / OpenAPI codegen (generators)

A schema-to-client generator scaffolds one layer — the ORM models, or
the API client — once, from a spec, and stops: there is no semantic
diff against a previous version, no whole-program rename, no
cross-service evolution ladder, and regenerating after a hand-edit is
either destructive (your edits are gone) or simply not offered (you
maintain the generated layer by hand from then on). CIaC's
regeneration is manifest-aware: a hand-edited generated file is
detected as edited and never silently overwritten
([docs/regeneration.md](regeneration.md)); `ciac diff --semantic`
classifies every architecture change as `Breaking`/`Additive`/
`Internal` against a checked-in baseline ([docs/evolution.md](evolution.md));
`ciac rename` is a whole-program, resolution-based rename, not a
find-and-replace.

**What generators have that CIaC does not, honestly:** a narrower,
easier-to-explain claim (one layer, not a whole system); a shallower
learning curve, since there's no DSL to adopt at all, just a config
format for a tool you likely already use; faster adoption for a team
that only needs that one layer generated.

## vs low-code / BaaS platforms

An adjacent pitch — "describe your backend, get it built" — with
opposite mechanics. A low-code platform's output is a black box that
runs *inside its own platform*: leaving means rebuilding from
scratch, because there's no ordinary source code to take with you.
CIaC's output is ordinary code in your own repository, checked in,
readable, and yours — the exit cost is zero by construction: delete
the compiler, and the last generated code stays exactly as runnable
as it was the moment before. No hosted runtime, no platform lock-in,
no vendor billing tied to your traffic.

**What low-code/BaaS platforms have that CIaC does not, honestly:**
a visual builder — CIaC has no GUI, only a text DSL with editor
support (`ciac lsp`); a managed runtime, meaning no deploy step of
your own to run at all, where CIaC's own output still needs somewhere
to run.

## When *not* to use CIaC

This section is not a footnote — an honest boundary is the thing that
makes the rest of this document worth reading.

- **Your system's core is what the language doesn't model.** Heavy
  numerical computation, ML training/inference pipelines, bespoke
  binary protocols, anything where the interesting logic isn't
  "typed API in front of a database/queue/cache" — CIaC models
  architecture, not arbitrary computation, and a handler's *body* is
  always yours to write regardless of target, but the ontology itself
  (records, apis, pipelines, streams, workers) has no opinion about
  domains outside it.
- **Your team won't adopt a DSL.** Every seam CIaC owns is a seam
  your team stops hand-maintaining, but that trade requires learning
  `.ciac` syntax and its ontology first — [the guide series](guide/01-first-service.md)
  is the on-ramp, but it is still a new thing to learn, not zero
  onboarding cost.
- **You need a specific framework ecosystem's depth on day one.**
  If your system's real requirement is "whatever this one Rails gem
  or this one Spring Boot starter already solved," CIaC's own
  ontology may not cover that gem's exact behavior yet, and writing
  it as a handler-body escape hatch is a workaround, not the same
  thing as the ecosystem depth itself.
- **You need a capability the divergence ledger lists as absent or
  target-narrow.** [docs/backends.md](backends.md#divergence-ledger)'s
  two tables are the actual, current, checkable answer to "does CIaC
  do X on target Y" — read them before assuming; the "Permanent by
  design" table names gaps that will never close (each with the
  reason), and the "Open (tracked)" table names gaps a specific
  future plan is scoped to close.

## The maturity statement

As of this document: language version 1.0.0 (frozen syntax, its own
semver and deprecation policy — [docs/language.md](language.md)),
compiler version 0.28.0. Every generated project passes lint and its
own test suite before `ciac build`/`verify` reports success; the
workspace runs `cargo deny`/`cargo audit` plus per-ecosystem
dependency scanning (pip-audit, npm audit, govulncheck, and a Java
scanner) in CI against representative generated projects, not just
the compiler's own dependency tree. The disclosed-gaps culture named
above is not aspirational: the divergence ledger, the simulation
capability matrix in [docs/simulation.md](simulation.md), and every
arc's own retrospective (this repository's `NNUpdatePlan.md` files)
exist specifically so a claim about this tool is checkable against
its own history rather than taken on faith.
