# Deployment (compose, Kubernetes, and system verification)

Every `ciac build` already emits a `docker-compose.yml` at the output
root (per-service `docker-compose.yml.j2` for a single deployable,
`system-compose.yml.j2` at the system root for a multi-service one) —
that stays the development default, requires nothing extra, and is
what `ciac verify --system` (below) runs against. v0.8 M6 adds a second,
opt-in production posture: Kubernetes manifests, generated from the
same validated system, alongside compose rather than instead of it.

## Compose (the default)

```sh
ciac build video-platform.ciac --target python --out ./video-platform
cd video-platform && docker compose up
```

Every capability (`db Postgres`, `cache Redis`, `queue NATS`, ...) gets
a real dev container wired in by name (`db`, `cache`, `queue`, or
`db-main`/`cache-hot`/... for named instances) — the same hostnames
every generated app's `app/config.py`/`config.rs` already points at by
default. Nothing to configure before `docker compose up`.

## OpenAPI and the TypeScript client (v0.15 M1/M2)

Every `ciac build` already emits `openapi.json` at the project root
(and `openapi.json` at the system root too for a multi-service
program, indexing every service) — every `api` and every crud-expanded
route, request/response records as component schemas (enums included),
scoped routes carrying an `x-ciac-scope` extension and a `bearerAuth`
security requirement, `/health` included. It's derived from the same
IR every backend generates from, one serializer, so both targets serve
one truth: the Python backend mounts it as FastAPI's own OpenAPI
source (replacing FastAPI's auto-generated one) rather than shipping
two near-duplicate specs.

`ciac build --client ts` additionally emits a dependency-free,
typed TypeScript `fetch` client under `clients/ts/` — one interface
per record, one function per route, scope-bearing routes typed to
require a token (`AuthenticatedClientOptions` vs. `ClientOptions`, so
a missing token is a compile error, not a runtime surprise). Generated
straight from the IR, not by shelling out to an external OpenAPI
generator, so it's deterministic and golden-covered like everything
else `ciac` emits:

```sh
ciac build video-platform.ciac --target python --out ./video-platform --client ts
```

## Kubernetes (`ciac build --deploy k8s`)

```sh
ciac build video-platform.ciac --target python --out ./video-platform \
    --deploy k8s --image-prefix registry.example.com/video-platform --image-tag v1.2.0
```

Emits `k8s/<service>.yaml` per declared service (a `Deployment` +
`ClusterIP` `Service`, both named after the service; a `ConfigMap`
holding the same environment variables compose already sets) plus one
`k8s/queue.yaml` (a `StatefulSet` + headless `Service` named `queue`)
when the program uses a `queue` capability. Readiness and liveness
probes point at the `/health` route every generated app already
serves — nothing new to add on the application side.

**Before `kubectl apply -f k8s/`, build and push the image** the
manifests reference (`{image-prefix}[-<service>]:{image-tag}`, single
image for a single-deployable program, one per service in a
multi-service system) from the `Dockerfile` `ciac build` already
generated:

```sh
docker build -t registry.example.com/video-platform:v1.2.0 .
docker push registry.example.com/video-platform:v1.2.0
```

`ciac` proves the deployment *shape* — it does not run a CI/registry
pipeline for you; wiring the build/push step to your own CI is your
call to make, the same way choosing a real image registry is.

### What's intentionally not generated

- **Stateful infra capabilities** (`db`, `cache`, `object_store`,
  `email`, `search`) get no k8s resources — no in-cluster Postgres
  StatefulSet, no Redis Deployment. Compose's dev-container convenience
  doesn't translate to "provision production infra automatically";
  point the `ConfigMap`'s connection strings at whatever you actually
  run (a managed database, a Helm chart, a hand-written manifest) —
  either name that Service to match the hostname already in the
  `ConfigMap` (`db`, `cache`, ...) or edit the `ConfigMap` directly.
- **Secrets stay in the `ConfigMap`, unencrypted**, with an obvious
  placeholder value (`JWT_SECRET: change-me-override-with-a-real-secret`).
  Kubernetes `Secret` generation is out of scope for now — override the
  placeholder with a real `Secret` (or an external-secrets operator)
  before this goes anywhere near production. This is disclosed, not
  silently unsafe: don't apply the generated `ConfigMap`'s secret-shaped
  values as-is.
- **No Ingress, no TLS, no autoscaling** — `ciac` emits the minimum
  shape the roadmap asks for (Deployments/Services/ConfigMaps per
  service, one broker StatefulSet); front it with whatever ingress
  controller and autoscaling policy your cluster already uses.
- **`tracing OpenTelemetry` and `users Keycloak` are compose-only**
  (v0.15 M3/M4/M6) — neither gets a k8s resource. A traced service
  still emits `OTEL_EXPORTER_OTLP_ENDPOINT` pointed at `otel-collector`
  in its `ConfigMap` (point a real collector Service at that hostname,
  or edit the value); a `users`-backed OAuth2 service gets an explicit
  `OAUTH_ISSUER: https://REPLACE-ME.example.com/realms/prod`
  placeholder instead of the dev Keycloak URL, since there's no
  Keycloak Service in the cluster to resolve it against — a
  misconfigured deploy fails loudly at startup rather than silently
  pointing at a container that was never provisioned.

## Generated CI (`ciac build --deploy ci`, v0.15 M5)

```sh
ciac build video-platform.ciac --target python --out ./video-platform --deploy ci
```

Emits `.github/workflows/ci.yml`: a `test` job running exactly what
`ciac verify` runs locally (`uv sync && uv run ruff check . && uv run
pytest -q` for Python, `cargo check` with `RUSTFLAGS=-D warnings` and
`cargo test -q --lib` for Rust), a `build-image` job per declared
service (matrixed for multi-service systems) that builds the
Dockerfile on every push and pushes only on a version tag, and a
`compose-smoke` job that boots the full dev compose stack and polls
every service's `/health` — the same acceptance bar `ciac verify
--system --live` already proves locally, running in CI too.

Registry credentials are never emitted, only referenced —
`secrets.REGISTRY_URL`/`REGISTRY_USERNAME`/`REGISTRY_PASSWORD` are
GitHub Actions repository secrets you configure yourself; `ciac`
emits the shape, not a credentials pipeline. `--deploy` is repeatable
alongside `k8s`/`terraform`: `--deploy k8s --deploy ci` emits both.
GitHub Actions only for now — the shape is a template away from
GitLab CI once real usage says that's worth it. `.github/workflows/
ci.yml` is an owned file like any other: a hand-edited copy survives
regeneration as a sidecar, never a silent overwrite.

## Terraform (`ciac build --deploy terraform`, v0.11)

k8s manifests deliberately stop at "point the ConfigMap at whatever
you actually run" for stateful infrastructure. `--deploy terraform`
reaches past that boundary: `terraform/main.tf` gets one AWS module
per stateful capability instance — RDS per `db` (postgres or mysql),
ElastiCache per `cache`, MSK when the program uses `queue Kafka`, an
S3 bucket per `object_store` — with outputs named after the same env
vars compose and the ConfigMap already use. `--deploy` is repeatable:
`--deploy k8s --deploy terraform` emits both. Same discipline as k8s:
ciac emits the shape; review networking/sizing and run
`terraform apply` yourself.

## `--profile` and `--secrets` (v0.11)

`--profile {dev,staging,prod}` sizes the deploy artifacts: k8s
replicas 1/2/3 (resource requests/limits beyond dev), Terraform
instance classes/storage/broker counts per tier. Compose is the
dev-only path and ignores it; generated application code is
profile-independent by design.

`--secrets` (with `--deploy k8s`) moves secret-shaped values
(`JWT_SECRET`) out of the ConfigMap into a generated
`k8s/<service>-secrets.yaml` `Secret` wired via `secretRef` —
placeholder values, override before applying.

## `ciac verify --system` (v0.8 M4, extended v0.9)

Compose and k8s both answer "can this be deployed"; `ciac verify
--system` answers "does the deployed system actually work" — it boots
the program's `docker-compose.yml` and runs a generated `tests/system/`
pytest suite (always Python, regardless of build target, since it's
exercising HTTP/NATS/WebSocket/SQL wire contracts rather than
target-language ones) proving whole-system behavior:

- every cross-service `call` is reachable at the URL/path the caller
  is configured to use;
- every single-hop publish→consume stream and channel actually
  delivers across the real broker;
- **capability round-trips (v0.9 M2)**: every auth-less typed CRUD
  resource is created through the real HTTP api, then read back
  through a *second, independent* connection — asyncpg straight into
  Postgres, and a direct Redis client for the cache entry after a
  cached read — proving the write persisted to the infrastructure,
  not just to app-process state. Compose maps each db/cache instance
  to a unique host port (5432+, 6379+) so these direct connections
  are possible from outside the compose network.
- **trace continuity (v0.15 M3/M4)**: on a `tracing OpenTelemetry`
  service whose pipeline crosses a `call` and/or a `publish`→worker
  hop, hits the entry route and polls Jaeger's query API for a trace
  under that service's name, asserting the span count is consistent
  with one continuous trace spanning every hop — not a dangling root
  span per service.
- **scope enforcement against a live IdP (v0.15 M6)**: on a
  `users Keycloak`-backed OAuth2 service, every scoped CRUD resource
  gets the same 403-without/200-with assertions the no-infra suite
  already proves for the `jwt` scheme, but with real tokens minted
  from the live dev realm via `scripts/token.sh` — the case the
  no-infra suite can't cover, since OAuth2 needs a live JWKS issuer.

```sh
ciac verify inventory-system.ciac --target python --out ./inventory --system
```

Requires Docker; a no-op success when the program has nothing
system-level to test (no cross-service edges and no verifiable
capabilities). Plain `ciac verify` (no `--system`) never runs this
suite and needs no infra — `tests/system/` is excluded from the
per-service project walk `ciac verify` otherwise does. See
`crates/ciac-codegen/src/system_tests.rs` for exactly which edges and
resources qualify and why (single-hop, non-auth-gated only — a
disclosed, deliberate scoping, not an oversight).

**This runs in CI on every push** (v0.9 M5): the `generated-system`
job system-verifies the multi-service examples for real — a green
checkmark means containers actually booted and the generated suite
passed against them, not just that text snapshots matched.

## `ciac verify --live` and `--keep` (v0.9 M3+M4)

`--live` boots the generated compose stack and polls every service's
`/health` route (bounded backoff, 60s budget), reporting per-service
up/down — a fast "does it even start" smoke check without running the
full system suite:

```sh
ciac verify app.ciac --target python --out ./app --live
```

`--keep` (with `--system` or `--live`) leaves a **green** stack
running instead of tearing it down, and prints the
`docker compose down` invocation to stop it — for poking at the live
system after verification passes. A failing run always tears down.
