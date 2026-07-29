# Operations (v0.15 — tracing, generated CI, dev identity)

*Reader: a builder running a generated system as a team, not just
generating it once.*

By v0.14 a generated system can be expressed, run, and verified in
minutes. This page covers the three v0.15 pillars that make it
something a team actually *operates*: following a request across
services, shipping a CI definition alongside the code, and running an
OAuth2-shaped system without hand-rolling tokens. `docs/deployment.md`
covers compose/k8s/Terraform/`ciac verify`; this page assumes that
context and focuses on what changes once a real incident, a real CI
run, or a real login flow enters the picture.

## Distributed tracing (`use { tracing OpenTelemetry; }`)

The gap this closes: generated systems exist to have cross-service
edges (`call`, streams, channels) — and gave the operator no way to
follow one request across them. Declaring `tracing OpenTelemetry` on
a service turns that on:

- **Python**: OTel SDK plus FastAPI/HTTPX auto-instrumentation; broker
  publishes carry `traceparent` in NATS headers / Kafka record
  headers; workers extract it and continue the trace.
- **Rust**: `tracing` + `opentelemetry-otlp` layers; the generated
  call clients inject `traceparent`; queue consumers extract it.
- **Compose**: an `otel-collector` container plus a Jaeger UI at
  `http://localhost:16686` — the same "real dev container" convention
  every capability follows. `OTEL_EXPORTER_OTLP_ENDPOINT` is wired
  into generated config automatically.

The property this buys, proven by the generated system suite (see
`docs/deployment.md`'s `ciac verify --system`): **one trace id spans
an api → `call` → downstream handler → `publish` → worker chain.**
Cut line for v0.15: auto-instrumented spans only (HTTP server/client,
broker produce/consume, db calls the instrumentation library gives
for free) — no custom span verbs in handler bodies yet.

Declare it per service; a multi-service system's compose gets one
shared collector every declared service points at. It's compose-only
— see `docs/deployment.md`'s k8s section for what a traced service's
`ConfigMap` looks like without a collector Service in the cluster.

## Generated CI (`ciac build --deploy ci`)

See `docs/deployment.md`'s "Generated CI" section for the full shape
(`test`/`build-image`/`compose-smoke` jobs). The operational framing:
this is deliberately small, by design — a `test` job that mirrors
`ciac verify` exactly (so "CI is green" and "`ciac verify` passes
locally" never drift apart), an image build/push job gated on a
version tag, and a compose smoke job proving the whole dev stack still
boots. GitHub Actions only; GitLab CI is a template away once real
usage says it's worth the second target.

## `users Keycloak`: a dev identity provider

The gap this closes: both auth providers (`JWT`, `OAuth2`) validate
tokens someone else issues — for the dev loop, and for tests that need
"a user with scope X", there was no story but hand-rolled scripts.
`use { users Keycloak; }` alongside `auth OAuth2` makes the system
runnable without an external IdP:

- compose gains a `keycloak` container seeded (via `--import-realm`)
  with a `dev` realm: a public, password-grant-enabled client, one
  client scope per distinct scope string declared anywhere in the
  system (`scope`/`read_scope`/`write_scope`), and two dev users
  (`dev-admin`, `dev-user`, password `dev-password`);
- `auth OAuth2`'s `issuer` defaults to this container's URL when the
  program's own `use { .. }` block omits one — an explicit `issuer`
  always wins, so pointing at a real IdP in prod is just "set
  `issuer` and don't declare `users`" (`users` is deploy-target-aware:
  it never emits a k8s/Terraform resource, so prod simply doesn't run
  the container — see `docs/deployment.md`'s k8s notes on what an
  OAuth2 service without `users` at runtime should point at instead);
- a generated `scripts/token.sh` mints real access tokens (the
  resource-owner-password-credentials grant) for humans (`bash
  scripts/token.sh dev-admin "orders:read orders:write"`) and for the
  generated `tests/system/` suite, which upgrades its scoped-route
  403/200 assertions from a locally-signed JWT to a token issued by a
  real IdP once `users` is present.

**Disclosed simplification**: Keycloak authorizes *optional* client
scopes per client, not per user — there's no realm-level mechanism
making `dev-admin` "have" a scope and `dev-user` "not have" it.
`scripts/token.sh` is what encodes that distinction, by choosing which
scopes to request at token time. That's an intentional fit for a
dev/test identity provider, not a production authorization model —
the resource-server stance from v0.11's `auth OAuth2` stands: no user
CRUD in the model, no registration/login UI, no session management.
`users` mints tokens; it does not become an identity product.

## A production checklist

None of the above is a substitute for review before a generated
system goes anywhere near production traffic. At minimum:

1. **Secrets**: `JWT_SECRET` and any `--deploy k8s --secrets`
   placeholder values are exactly that — placeholders. Override them
   with real secrets (or an external-secrets operator) before
   `kubectl apply`.
2. **`users Keycloak`**: confirm it isn't declared on any service a
   production deploy target actually builds from, or confirm your k8s
   manifests were generated with an explicit real `issuer` instead of
   relying on the dev default. The `REPLACE-ME` placeholder
   `--deploy k8s` emits for a `users`-backed OAuth2 service exists
   precisely so this can't be missed silently.
3. **Tracing collector endpoint**: `OTEL_EXPORTER_OTLP_ENDPOINT`
   points at the dev `otel-collector` hostname by default; point it at
   your real collector (or override per-environment) before deploying.
4. **CI credentials**: `.github/workflows/ci.yml`'s `secrets.
   REGISTRY_*` references need real GitHub Actions secrets configured
   in the repository — `ciac` never generates or stores credentials
   itself.
5. **Everything `docs/deployment.md` already says**: no Ingress/TLS/
   autoscaling generated, stateful capabilities get no k8s resources,
   `ciac` proves the deployment *shape*, not a full production
   posture.
