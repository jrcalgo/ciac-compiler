# Documentation index

*29UpdatePlan.md M6's coherence pass: every doc below states its own
reader in its first lines; this page just tells you where to start.
Not part of the guide series' own numbering — it's the map, not a
guide.*

## Start here

| Doc | Reader | What it covers |
|---|---|---|
| [../README.md](../README.md) | Anyone new to CIaC | The 15-minute pitch: claim, a working example, the five-target map |
| [positioning.md](positioning.md) | An evaluator deciding whether to adopt CIaC | When to reach for it, when not to, vs frameworks/generators/BaaS |
| [guide/01-first-service.md](guide/01-first-service.md) | A builder, first hour | Series entry point — start here to learn by building |

## The guide series

One continuous example (`Ping`), one guide per topic, each ending at
a real, harness-verified checkpoint:

| Guide | Covers |
|---|---|
| [01-first-service.md](guide/01-first-service.md) | Install, `ciac new`, project anatomy, the check/build/verify loop |
| [02-records-and-crud.md](guide/02-records-and-crud.md) | Records, `crud`, schema evolution, `ciac diff` |
| [03-handlers-and-logic.md](guide/03-handlers-and-logic.md) | Typed handlers, `transaction`, streams, workers |
| [04-streams-and-workers.md](guide/04-streams-and-workers.md) | Channels, scheduled jobs, multiple consumers of one stream |
| [05-simulation.md](guide/05-simulation.md) | Scenarios, failure injection, `ciac sim` — no database, no Docker |
| [06-multi-service.md](guide/06-multi-service.md) | `project`, cross-service `call`, multi-service simulation |
| [07-deployment-and-day-two.md](guide/07-deployment-and-day-two.md) | k8s/Terraform/CI artifacts, `--system`, rename, the backfill ladder |

## Reference

| Doc | Reader | Covers |
|---|---|---|
| [language.md](language.md) | Anyone wanting the exact language rule | Full declaration/provider reference, per-target support |
| [expressions.md](expressions.md) | A builder writing handler-body logic | The expression language, verbs, `transaction` |
| [blueprints.md](blueprints.md) | A builder reusing patterns across files/projects | Modules, blueprints, `registry:` imports |
| [authoring.md](authoring.md) | A builder setting up editor support | `ciac new`, `ciac lsp`, editor setup, `registry:` |
| [dev-loop.md](dev-loop.md) | A builder iterating locally | `ciac dev`'s watch loop |
| [regeneration.md](regeneration.md) | A builder regenerating an edited project | The manifest/drift discipline |
| [evolution.md](evolution.md) | A builder changing a shipped system | Semantic diff, rename, the backfill ladder |
| [simulation.md](simulation.md) | A builder writing/running scenarios | Full scenario schema, capability matrix per target |
| [deployment.md](deployment.md) | A builder taking a system to production | Compose, Kubernetes, Terraform, `--system` |
| [operations.md](operations.md) | A builder running a system as a team | Tracing, generated CI, dev identity provider |
| [errors.md](errors.md) | Anyone who hit a `CIAC####` code | The full error code index |
| [backends.md](backends.md#divergence-ledger) | A contributor adding a target, or an evaluator checking parity | Writing a backend; the two-table divergence ledger |
| [external-backends.md](external-backends.md) | Someone implementing a target outside the compiler | The external-backend wire protocol |
| [agents.md](agents.md) | An agent (or its human) | `ciac describe`, `ciac mcp`, generated `AGENTS.md` |

## For contributors to the compiler itself

| Doc | Covers |
|---|---|
| [architecture.md](architecture.md) | The compiler's own pipeline (syntax → sema → codegen) |
| [ir.md](ir.md) | The `SystemGraph` intermediate representation |
| [perf/README.md](perf/README.md) | Generation-speed baseline, the budget gate, how to benchmark a backend change |

## History

| Doc | Covers |
|---|---|
| [history/history.md](history/history.md) | The version-by-version story — how the language and its five targets grew |
| [history/backend-spike-report.md](history/backend-spike-report.md) | The v0.8 M6 third-backend spike (historical interest only) |

Every `NNUpdatePlan.md` file at the repository root is this
project's own detailed build log — one file per arc, milestone by
milestone, with a "Shipped" note appended to each milestone as it
actually happened. Not indexed here (there are many, and they're
addressed by number in cross-references throughout the docs above),
but the real source if you want the full story behind any decision.
