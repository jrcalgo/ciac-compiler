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

## `ciac verify --system` (v0.8 M4)

Compose and k8s both answer "can this be deployed"; `ciac verify
--system` answers "does the deployed system actually work" — it boots
the program's `docker-compose.yml` and runs a generated `tests/system/`
pytest suite (always Python, regardless of build target, since it's
exercising HTTP/NATS/WebSocket wire contracts rather than
target-language ones) proving whole-system edges: every cross-service
`call` is reachable, every single-hop publish→consume stream and
channel actually delivers across the real broker.

```sh
ciac verify video-platform.ciac --target python --out ./video-platform --system
```

Requires Docker; a no-op success when the program has no whole-system
edges to test (single-service, no cross-service calls or streams).
Plain `ciac verify` (no `--system`) never runs this suite and needs no
infra — `tests/system/` is excluded from the per-service project walk
`ciac verify` otherwise does. See `crates/ciac-codegen/src/system_tests.rs`
for exactly which edges qualify and why (single-hop, non-auth-gated
calls only — a disclosed, deliberate scoping, not an oversight).
