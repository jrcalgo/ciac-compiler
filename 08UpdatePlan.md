# CIaC v0.8 — Composition & Proof: Modules, Blueprints, System Guarantees (roadmap forecast)

> Forecast document, two iterations out. Assumes v0.6 (living
> projects, jobs, channels) and v0.7 (expression language, tables,
> migrations, behavioral tests) have landed. Direction-setting; the
> v0.8 planning pass finalizes grammar and codes.

## The gap this version closes

After v0.7, one `.ciac` file can express a complete, behaving,
multi-service system — but only as a monolithic file with no reuse.
Real systems repeat themselves: every service wants the same auth
posture, the same webhook-receiver shape, the same audited-CRUD
pattern. Goal 3's "composable together having used DRY method" is,
at the language level, still unmet: there is no way to write a pattern
once and instantiate it twice. And goal 4's guarantee stops at each
service's edge: the compiler proves each piece, but "the *system*
functions" is asserted, not tested.

**v0.8 theme: compose programs from reusable, parameterized parts, and
make the compiler prove the composed system end-to-end.**

## Pillar 1 — Modules: multi-file programs

```ciac
// media-system.ciac
project MediaSystem;
import "records/video.ciac";        // records, tables, errors
import "services/billing.ciac";     // a service block per file
import "services/upload.ciac";
```

- `import` is textual-with-scoping: each file parses independently;
  names resolve project-wide after all imports load (cycle in imports:
  new error; duplicate names across files: existing CIAC0003 with
  cross-file spans — the diagnostics infrastructure already carries
  multi-file `SourceMap`).
- `ciac check/build` accept a directory or entry file; the manifest
  records the full source-set hash (regeneration correctness).
- Deterministic ordering: declaration order = import order, then file
  order — locking today's ordering guarantees across files.

## Pillar 2 — Blueprints: parameterized architecture (the DRY core)

```ciac
blueprint AuditedCrud<R: record> {
    params { prefix: String; }
    use { db main Postgres; }
    crud R;
    stream Audited: AuditEvent;
    handler AfterWrite(r: R) -> R { publish Audited(AuditEvent { … }); return r; }
}

service Catalog  { expand AuditedCrud<Video>  { prefix: "/v1"; } }
service Accounts { expand AuditedCrud<User>   { prefix: "/v1"; } }
```

- A blueprint is a checked template over records (and scalar params):
  type-checked **once generically** (R constrained to `record`, field
  requirements expressible as `where R has id: Uuid`), then expanded
  per instantiation with hygienic naming (instance-qualified node
  names, exactly the `scoped_key` machinery services already use).
- Expansion happens in sema *before* validation passes — identical to
  how `crud`/`events` already lower — so every existing pass validates
  expanded output with zero changes. Blueprints are macros with a type
  discipline, not a new IR concept.
- **Standard library**: `std/` blueprints shipped with the compiler
  (versioned with it): `std.Crud`, `std.EventPipeline`,
  `std.WebhookReceiver`, `std.OutboxPublish`, `std.RateLimitedApi` —
  each one a distillation of the patterns v0.1–v0.7 already generate,
  now expressed *in the language itself*. Dogfooding target: `crud X;`
  becomes sugar for `expand std.Crud<X>;` with byte-identical output.

## Pillar 3 — System-level guarantees

v0.7 proves each handler; v0.8 proves the composed graph:

1. **Generated system tests** (`tests/system/` at the output root):
   from the IR the compiler derives executable assertions per edge —
   every `call` gets a contract round-trip test against the target
   service booted in-process/fixture; every publish→consume edge gets a
   broker-backed delivery test; every channel a subscribe-and-receive
   test. `ciac verify --system` runs them over the compose stack.
2. **Record evolution checking**: records become versioned in the
   manifest; a rebuild that changes a record used across a service
   boundary (call payload or shared stream) must be
   backward-compatible (adding optional fields ok; removing/retyping →
   new error with the exact consumer list from the graph). The
   compiler's whole-system view makes breaking-change detection exact,
   not heuristic — this is a guarantee no per-service framework can
   give.
3. **Production posture**: `--deploy k8s` emits Kubernetes manifests
   (Deployments/Services/ConfigMaps per service, one broker
   StatefulSet) from the same SystemModel that emits compose today;
   readiness/liveness wired to `/health`; per-service `.env.example`.
   Compose remains the dev default; k8s output goes through the same
   golden/determinism suite.

## Secondary items

- **Third backend spike (Go or TypeScript)**: not to ship — to prove
  the `Backend` seam and the v0.7 emitter abstraction hold for a
  genuinely different host before the interface calcifies. Timebox;
  outcome is a report + seam fixes, shipping optional.
- **Performance/limits pass**: channel fan-out pooling (one broker
  subscription per channel, not per socket), generated pagination on
  `query`, compile-time budget checks (graph size, blueprint expansion
  depth cap with error).
- **`ciac fmt`**: canonical formatter for `.ciac` (multi-file projects
  make style drift real; determinism ethos extends to source).

## Milestones

1. Modules: multi-file parsing/resolution, manifest source-set,
   cross-file diagnostics, examples split into files.
2. Blueprints: grammar, generic checking, hygienic expansion,
   negative suite (arity, constraint violations, expansion collisions).
3. `std` blueprints + `crud` re-based on `std.Crud` (byte-identical
   golden proof).
4. System tests generation + `ciac verify --system` (compose-backed;
   the v0.5.1 live round-trip becomes a *generated artifact* every
   user gets).
5. Record evolution checks across rebuilds (manifest-versioned).
6. k8s emission target; docs (`docs/blueprints.md`,
   `docs/deployment.md`); backend spike report; version 0.8.0.

## Risks

- Blueprint hygiene is the hard correctness problem (name capture
  across expansions × services); mitigated by reusing service scoping
  and an aggressive collision fixture suite.
- Generic checking can rabbit-hole; v0.8 constraints are limited to
  `record` + `has field: Type` — no higher-order blueprints, no
  blueprint-importing-blueprint beyond one level.
- System tests must not require Docker in unit CI: broker-backed tests
  gate behind `ciac verify --system` (documented as requiring compose),
  while call-contract tests run in-process.

## After v0.8

The four project goals read as met for the supported ontology: whole
systems in `.ciac` (1), compiled to interchangeable hosts (2), fully
implemented and DRY via expressions + blueprints (3), guaranteed by
handler-level equivalence tests, generated system tests, evolution
checks, and regeneration discipline (4). The v0.9+ horizon is then
breadth, not architecture: Kafka semantics, more providers per
capability, the third backend graduating to supported, and a public
blueprint registry.
