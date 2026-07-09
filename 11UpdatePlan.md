# CIaC v0.11 — Breadth: Capability Providers & Real Deployment (roadmap forecast)

> Forecast document. Assumes v0.9 (verification automated) and v0.10
> (backend legibility, agent-facing CLI) have landed. Direction-setting;
> the v0.11 planning pass finalizes exact provider set and IaC tool
> choice.

## The gap this version closes

Every capability in CIaC has exactly one provider: `db` is Postgres,
`cache` is Redis, `auth` is JWT, `queue` is NATS (with `Kafka` accepted
by the grammar and gated at build since v0.3 — the sole exception,
and even that is a second *provider* of an existing capability, not a
new one). Real teams do not show up with a blank slate; they show up
with an existing MySQL instance, an existing OAuth2 identity provider,
an existing RabbitMQ cluster. For that audience, the single-provider
ontology is the actual adoption blocker, more than any missing
language construct.

Deployment has the same shape of gap one layer up: `--deploy k8s`
(v0.8) explicitly and correctly refuses to provision stateful infra —
Postgres, Redis, S3 all stay "point this at something real yourself."
That's the right call for a Deployment/ClusterIP/ConfigMap generator,
but nothing in CIaC today helps with the "something real" half — no
Terraform, no secrets-manager integration beyond a default password
sitting in a compose environment variable. The gap between "generates
a working dev stack" and "deployed by a team" is entirely unaddressed
past the k8s manifest boundary.

**v0.11 theme: breadth, spent deliberately — new providers where teams
actually show up with existing infra, and a deployment story that
reaches past the point compose/k8s already stop.**

## Pillar 1 — A real provider abstraction, not a one-off per capability

`QueueEngine::{Nats, Kafka}` (`ciac-ir/src/component.rs`) is already
the shape every other capability needs: an enum naming the provider,
carried through IR into codegen, gating unimplemented providers at
`ciac build` (`CIAC0011`) while still passing `ciac check` — v0.3's
approach to Kafka is, unmodified, the template for this whole pillar.

- Extend the same enum-per-capability pattern to `db` (`Postgres`,
  `MySQL`, `Sqlite`) and `auth` (`JWT`, `OAuth2`, `Session`) first —
  chosen as the two highest-leverage gaps from the "existing infra"
  angle, not the full capability matrix at once.
- Each new provider needs: an IR variant, a codegen path per host
  (Python: SQLAlchemy dialect swap for db providers, an
  authlib-based flow for OAuth2; Rust: SQLx driver feature swap,
  `oauth2` crate), a docker-compose dev container, and a shared
  conformance test run against every provider of a capability (same
  CRUD/auth assertions, parameterized by provider) — this last piece
  is what keeps the provider matrix from becoming an untested
  combinatorial surface as it grows.
- `object_store`/`search`/`email` provider expansion (GCS/Azure Blob,
  a second search engine, more email providers) explicitly deferred
  past this pillar — v0.11 proves the pattern on two capabilities
  deep enough to be real, rather than one shallow pass across all of
  them.

## Pillar 2 — Kafka, finally

With Pillar 1's provider-abstraction pattern proven on `db`/`auth`,
`queue Kafka` — gated since before v0.4 — has no remaining excuse:
the IR variant already exists, only the codegen path and dev-compose
container are missing.

- Codegen path per host (Python: `aiokafka` or `confluent-kafka`;
  Rust: `rdkafka`), matching the existing NATS wrapper shape closely
  enough that pipeline/stream/worker semantics (subjects, fan-out,
  consumer groups) need no language-level changes — Kafka topics map
  onto the same `<service>.<stream>` naming NATS subjects already use,
  consumer groups map onto the same per-worker queue-group naming.
- Dev-compose: single-node KRaft (no ZooKeeper) container, keeping the
  "nothing to configure before `docker compose up`" promise
  `docs/deployment.md` already makes for every other capability.
- Un-gate `CIAC0011` for `queue Kafka` once the conformance suite
  (Pillar 1's harness, parameterized to cover queue providers too)
  passes for it identically to NATS.

## Pillar 3 — Deployment past the k8s manifest boundary

- **Terraform module generation** (opt-in, alongside `--deploy k8s`,
  not replacing it): for each stateful capability instance, emit a
  module provisioning the managed equivalent (RDS/CloudSQL for `db`,
  ElastiCache/MemoryStore for `cache`, MSK for `queue Kafka`) —
  generated from the same `Ctx`/capability model the k8s manifests and
  compose file already derive from, so the three deployment artifacts
  never describe three different systems.
- **Secrets-manager integration**: replace the default-password
  environment variable story for at least one real backend (Vault or
  AWS Secrets Manager, picked in the v0.11 planning pass) — the
  generated k8s `ConfigMap` gains a secrets-reference variant pointing
  at the chosen backend instead of a plaintext value, opt-in via the
  same `--deploy k8s` flag family.
- Scope discipline: this is not a CI/CD pipeline generator and not a
  registry-provisioning tool — `docs/deployment.md`'s existing
  disclaimer ("`ciac` proves the deployment shape — it does not run a
  CI/registry pipeline for you") extends unchanged to Terraform; v0.11
  generates the module, it does not run `terraform apply`.

## Pillar 4 — Environment/profile story

- `ciac build --profile {dev,staging,prod}` (name and exact semantics
  finalized in planning): today exactly one docker-compose shape
  exists per build, sized for local development. A profile flag
  selects resource sizing/replica hints threaded into the k8s
  manifests and Terraform modules from Pillar 3 — dev stays the
  zero-config compose default; staging/prod are where the new
  deployment artifacts actually matter.

## Secondary items

- `ciac targets`/provider listing extended to show provider support
  per capability (today it only lists backend targets, not providers).
- Conformance-suite results surfaced via v0.10's `--json` output, so
  "which provider combinations are actually proven" is queryable data,
  not something read off a test file.

## Milestones

1. Provider-abstraction refactor: `db` (Postgres + MySQL) and `auth`
   (JWT + OAuth2) as the two proof cases, shared conformance harness.
2. Kafka codegen (Python + Rust) + dev-compose container; `CIAC0011`
   un-gated for `queue Kafka`.
3. Terraform module generation for stateful capability instances,
   opt-in alongside `--deploy k8s`.
4. Secrets-manager integration for one backend (Vault or AWS Secrets
   Manager).
5. `--profile` flag threading resource sizing into k8s/Terraform
   output.
6. Docs (`docs/language.md` capability table rewritten for multi-
   provider reality, `docs/deployment.md` Terraform section), version
   0.11.0.

## Risks

- **Provider matrix combinatorial growth.** N capabilities × M
  providers is a real testing burden — mitigated structurally by the
  shared conformance harness from Pillar 1 (one assertion set per
  capability, parameterized over providers) rather than hand-written
  per-provider test suites that drift apart.
- **Terraform/k8s/compose drift.** Three deployment artifacts
  generated from three independent code paths will disagree eventually
  — mitigated by deriving all three from the same `Ctx` capability
  model (the same discipline that already keeps compose and k8s
  manifests honest against each other).
- **Scope discipline.** `object_store`/`search`/`email` provider
  breadth, a full CI/CD pipeline generator, and automatic infra
  provisioning (`terraform apply` on ciac's behalf) are all explicitly
  out of scope — each is a plausible v0.12+ item, not a v0.11
  commitment.

## After v0.11

CIaC now serves teams with real, existing infrastructure footprints
and a deployment story that reaches past a manifest into what actually
provisions the infrastructure it names. The remaining gap is upstream
of all of this: the authoring experience for the `.ciac` language
itself, and the ecosystem for sharing patterns across teams — v0.12's
subject.
