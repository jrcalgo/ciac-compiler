"""v0.17 M10: the bounded child protocol's Python side.

`ciac sim` (Rust) writes this runner's source (embedded into the
`ciac` binary via `include_str!`, so it works regardless of the
current directory a user invokes `ciac` from) next to the generated
project, then invokes this script once per scenario with the `SimPlan`
JSON and the scenario JSON as file arguments, reading back one JSON
result document from stdout. This is what "bounded" means here: one
process, one scenario, one JSON reply, then exit -- not a persistent
session or a streaming request/response protocol. Building the richer
kind (individual `advance`/`drain` steps round-tripped live to Rust)
remains future work; this bounded shape is what M10 ships.

Auto-discovers workers and jobs from `SimPlan` using the generated
project's own naming convention -- `app.workers.<snake_name>`, reading
`SUBJECT`/`QUEUE_GROUP`/`MAX_RETRIES` (workers) or `SCHEDULE`/
`CATCH_UP` (jobs) as the module-level constants every worker/job
template already emits, never guessed. `SimPlan` does not (M2's own
disclosed scope) enumerate APIs, so APIs are instead discovered
directly from the scenario's own `request` steps: each `api` name is
snake_cased (matching `ciac-codegen`'s own file-naming exactly) and
imported from `app.api.<snake>`. A route is auto-called only in the
common case -- its only parameters besides `payload` are named
`session` and/or `claims`; anything else (a second capability
instance) is refused with a clear, disclosed error naming the exact
route and its extra parameters, not a silent skip or an opaque crash.

27UpdatePlan.md M3: a `claims` parameter (`Depends(require_auth)` or
`Depends(require_scope(...))`, the shape every generated auth-gated
route carries -- see `app/auth.py.j2`) is resolved by walking that
`Depends` chain directly (`_resolve_claims`) and synthesizing a bearer
token from the scenario step's own `"as": {"sub": ..., "scopes": [...]}`
principal via `world.auth.issue`, rather than replicating FastAPI's
request-scoped dependency resolution wholesale. A request step naming
an auth-gated api with no `as` principal is refused, not silently
treated as anonymous.
"""

from __future__ import annotations

import argparse
import asyncio
import base64
import importlib
import inspect
import json
from pathlib import Path
from typing import Any

from pydantic import BaseModel

from cron import CronSchedule, parse_duration_ms
from replay import ReplayError, build_replay, check_compatible
from scenario_runner import ApiEntry, JobEntry, ScenarioRunner, WorkerEntry
from world import SEARCH_INDEX_NAME, Schema, SimWorld


class RegistryError(Exception):
    """A scenario named an api/worker/job this driver can't auto-wire
    -- reported clearly, not silently skipped or crashed on."""


def to_snake_case(name: str) -> str:
    """Matches `ciac_sim::plan::snake_case` (Rust) and `world.py`'s own
    copy exactly -- see either's docstring for why this matches
    `ciac-codegen`'s generated file/function names."""
    out: list[str] = []
    for i, c in enumerate(name):
        if c.isupper():
            if i != 0:
                out.append("_")
            out.append(c.lower())
        else:
            out.append(c)
    return "".join(out)


def _payload_constructor(fn, param_name: str = "payload"):
    """The real class a typed parameter's annotation names, if it's a
    pydantic model (generated worker/job/api functions never use
    `from __future__ import annotations`, so annotations are live class
    objects, not strings) -- `None` for an untyped `dict[str, Any]`
    payload, in which case the raw dict is passed through unchanged."""
    annotation = inspect.signature(fn).parameters[param_name].annotation
    if inspect.isclass(annotation) and issubclass(annotation, BaseModel):
        return annotation
    return None


def build_workers_and_jobs(
    plan: dict[str, Any],
    service_key: str,
) -> tuple[dict[str, WorkerEntry], dict[str, JobEntry]]:
    """`service_key` (`SimPlan`'s own `"service/<Name>"` scheme) filters
    `plan["workers"]`/`plan["jobs"]` down to this driver's own service --
    for a single-service program every entry already belongs to the one
    known service, so this filter is a no-op there (28UpdatePlan.md M3:
    for a multi-service program, `plan["workers"]`/`plan["jobs"]` list
    *every* service's own entries, tagged by `service_key`, and
    importing another service's `app.workers.<name>` while a different
    service's tree is active would resolve to the wrong module)."""
    workers: dict[str, WorkerEntry] = {}
    for worker in plan.get("workers", []):
        if worker.get("service_key") != service_key:
            continue
        snake = to_snake_case(worker["name"])
        module = importlib.import_module(f"app.workers.{snake}")
        handle_once = module.handle_message_once
        payload_cls = _payload_constructor(handle_once)

        async def handle(payload: dict, _once=handle_once, _cls=payload_cls) -> None:
            await _once(_cls(**payload) if _cls is not None else payload)

        workers[worker["name"]] = WorkerEntry(
            service=worker.get("service_key") or "",
            subject=module.SUBJECT,
            queue_group=module.QUEUE_GROUP,
            handle_once=handle,
            max_retries=module.MAX_RETRIES,
        )

    jobs: dict[str, JobEntry] = {}
    for job in plan.get("jobs", []):
        if job.get("service_key") != service_key:
            continue
        snake = to_snake_case(job["name"])
        module = importlib.import_module(f"app.workers.{snake}")
        jobs[job["name"]] = JobEntry(
            service=job.get("service_key") or "",
            schedule=CronSchedule.parse(module.SCHEDULE),
            handle_tick=module.handle_tick_once,
        )
    return workers, jobs


def build_failure_rules(scenario: dict[str, Any]) -> list[tuple[str, str | None, int]]:
    """Reads `given.failures` (v0.17 M10: added to `ciac_sim::scenario
    ::Given` so a checked-in scenario is fully self-describing, not
    dependent on out-of-band per-fixture configuration a hand-written
    script used to supply). `world.py`'s `FailureEngine` only supports
    the `error` action (a disclosed, narrower subset of the full
    `FailureAction` vocabulary `ciac-sim` owns); any other action kind
    is refused clearly rather than silently ignored."""
    rules: list[tuple[str, str | None, int]] = []
    for rule in scenario.get("given", {}).get("failures", []):
        if rule["action"]["kind"] != "error":
            raise RegistryError(
                f"failure rule for effect {rule['at']['effect']!r} uses action "
                f"{rule['action']['kind']!r}, which this driver's FailureEngine "
                f"restatement does not support (only 'error' is implemented)"
            )
        at = rule["at"]
        rules.append((at["effect"], at.get("subject"), at["occurrence"]))
    return rules


async def apply_given(world: SimWorld, scenario: dict[str, Any]) -> None:
    """27UpdatePlan.md M3: seeds `given.db`/`given.cache`/`given.store`/
    `given.search`/`given.external_http` into `world` before the runner
    executes any step. Until this fix, `build_failure_rules` was the
    *only* `given.*` list this driver ever consumed -- every other
    `given.*` list was parsed by `ciac_sim::scenario` and then silently
    dropped on the floor, discovered while authoring this milestone's
    corpus scenarios (all of which lean on seeding to set up state
    without a bespoke handler call for every fixture)."""
    given = scenario.get("given", {})
    for table_rows in given.get("db", []):
        table = table_rows["table"]
        for row in table_rows["rows"]:
            world.db.insert(table, str(row["id"]), row)
    for entry in given.get("cache", []):
        ex_seconds = parse_duration_ms(entry["ttl"]) // 1000 if entry.get("ttl") else None
        await world.fake_cache(entry["instance"]).set(
            entry["key"], json.dumps(entry["value"]), ex=ex_seconds
        )
    for obj in given.get("store", []):
        body = base64.b64decode(obj["value_base64"])
        await world.fake_object_store(obj["instance"]).put(obj["key"], body)
    for doc in given.get("search", []):
        await world.fake_search(doc["instance"]).index(SEARCH_INDEX_NAME, doc["id"], doc["doc"])
    # `given.external_http` is read lazily by `fake_http_client` on its
    # first access per instance (see `SimWorld.fake_http_client`), so
    # seeding `world.http_fixtures` directly -- before any request step
    # can trigger that first access -- is enough; no constructor plumbing
    # needed.
    for fixture in given.get("external_http", []):
        world.http_fixtures[fixture["instance"]] = fixture["responses"]


async def _resolve_claims(dependency: Any, credentials: Any) -> Any:
    """27UpdatePlan.md M3: walks a generated route's `claims` parameter's
    FastAPI `Depends` chain (`require_auth`, or `require_scope(...)`'s
    returned closure wrapping `require_auth`) without a real ASGI
    request -- every leaf in that chain only ever needs a bearer
    credentials object, which the caller already synthesized from the
    scenario step's `as` principal via `world.auth.issue`. This is not a
    general FastAPI dependency-injection resolver; it only recurses
    through `Depends`-typed parameters and fills a `credentials`-named
    leaf, which is all `app/auth.py.j2`'s own two functions ever need."""
    from fastapi import params as fastapi_params

    sig = inspect.signature(dependency)
    kwargs: dict[str, Any] = {}
    for name, param in sig.parameters.items():
        if name == "credentials":
            # The chain's leaf: `require_auth`'s own bearer-credentials
            # parameter, normally filled by `Depends(_bearer)` resolving
            # a real `Request`'s Authorization header -- supplied
            # directly here instead of recursing into `_bearer` itself.
            kwargs[name] = credentials
        elif isinstance(param.default, fastapi_params.Depends):
            kwargs[name] = await _resolve_claims(param.default.dependency, credentials)
        else:
            raise RegistryError(
                f"auth dependency {dependency!r} has an unrecognized parameter {name!r} "
                f"this driver's claims resolver does not know how to supply"
            )
    return await dependency(**kwargs)


def build_apis(scenario: dict[str, Any], service_name: str) -> dict[tuple[str, str], ApiEntry]:
    """`service_name` filters the scenario's own `request` steps down to
    this driver's own service (single-service: every step already
    names the one known service -- SIM0011 refuses anything else before
    this driver ever runs -- so the filter is a no-op there;
    28UpdatePlan.md M3, multi-service: importing another service's
    `app.api.<name>` while a different service's tree is active would
    resolve to the wrong module, so only this service's own api names
    may be imported while it's the one active). Registered under
    `(service, api)`, not `api` alone -- see `ApiEntry`'s own
    docstring, `scenario_runner.py`."""
    api_names = {
        step["request"]["api"]
        for step in scenario["steps"]
        if "request" in step and step["request"]["service"] == service_name
    }
    apis: dict[tuple[str, str], ApiEntry] = {}
    for api_name in api_names:
        snake = to_snake_case(api_name)
        module = importlib.import_module(f"app.api.{snake}")
        route = getattr(module, snake)
        sig = inspect.signature(route)
        params = [p for p in sig.parameters if p != "payload"]
        extra = set(params) - {"session", "claims"}
        if extra:
            raise RegistryError(
                f"api {api_name!r} (app.api.{snake}.{snake}) needs manual registration -- "
                f"auto-discovery only handles `session` and `claims` beyond `payload`, "
                f"found extra parameter(s) {sorted(extra)!r}"
            )
        payload_cls = _payload_constructor(route)
        needs_session = "session" in params
        claims_dependency = sig.parameters["claims"].default.dependency if "claims" in params else None

        async def call(
            payload: dict,
            principal: dict[str, Any] | None,
            _route=route,
            _cls=payload_cls,
            _needs_session=needs_session,
            _claims_dep=claims_dependency,
            _api_name=api_name,
        ) -> Any:
            value = _cls(**payload) if _cls is not None else payload
            kwargs: dict[str, Any] = {}
            if _claims_dep is not None:
                if principal is None:
                    raise RegistryError(
                        f"api {_api_name!r} requires auth claims but the scenario's "
                        f"request step supplied no `as` principal"
                    )
                from fastapi.security import HTTPAuthorizationCredentials

                from app.state import current

                token = f"sim:{principal['sub']}:{','.join(principal.get('scopes', []))}"
                current().world.auth.issue(
                    token, {"sub": principal["sub"], "scope": " ".join(principal.get("scopes", []))}
                )
                credentials = HTTPAuthorizationCredentials(scheme="Bearer", credentials=token)
                kwargs["claims"] = await _resolve_claims(_claims_dep, credentials)
            if _needs_session:
                from app.db import get_sessionmaker

                async with get_sessionmaker("default")() as session:
                    return await _route(value, **kwargs, session=session)
            return await _route(value, **kwargs)

        apis[(service_name, api_name)] = ApiEntry(service=service_name, call=call)
    return apis


async def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("plan_path", type=Path)
    parser.add_argument("scenario_path", type=Path)
    parser.add_argument("--source-hash")
    parser.add_argument("--plan-hash")
    parser.add_argument("--record", type=Path)
    parser.add_argument("--replay", type=Path)
    args = parser.parse_args()

    plan = json.loads(args.plan_path.read_text())
    scenario = json.loads(args.scenario_path.read_text())

    recorded = None
    if args.replay is not None:
        recorded = json.loads(args.replay.read_text())
        try:
            check_compatible(recorded, source_hash=args.source_hash, plan_hash=args.plan_hash)
        except ReplayError as exc:
            print(json.dumps({"scenario": scenario["name"], "passed": False, "error": str(exc)}))
            return

    try:
        # `plan["services"][0]` is this single-service driver's own,
        # only service -- SIM0011 already refuses a scenario naming any
        # other, per `build_apis`'s own docstring.
        service = plan["services"][0]
        workers, jobs = build_workers_and_jobs(plan, service["key"])
        apis = build_apis(scenario, service["name"])
        failure_rules = build_failure_rules(scenario)
    except RegistryError as exc:
        print(json.dumps({"scenario": scenario["name"], "passed": False, "error": str(exc)}))
        return

    # 27UpdatePlan.md M2: `Schema.from_plan_json(plan)` (proven since
    # v0.17 M6) was, until this fix, only ever wired by the bespoke
    # `inner_proof_domain_orders.py` dev script -- the real `ciac sim`
    # driver here built every `SimWorld` with `Schema.empty()` (the
    # `SimWorld.schema` dataclass field's own default), silently
    # disabling reference/unique/cascade/restrict checking for every
    # actual `ciac sim` invocation regardless of target program.
    # Discovered live while authoring this arc's `relational-depth`
    # corpus scenario: a cascade-delete request returned success but
    # left the dependent row in place, which traced back to this gap,
    # not to `FakeDatabase`/`Schema` themselves (both already correct).
    world = SimWorld(failure_rules=failure_rules, schema=Schema.from_plan_json(plan))
    from app.state import AppState, set_current

    set_current(AppState.simulation(world))
    await apply_given(world, scenario)
    runner = ScenarioRunner(world=world, apis=apis, workers=workers, jobs=jobs)

    error = None
    try:
        await runner.run(scenario)
    except Exception as exc:  # noqa: BLE001 -- reported, not swallowed
        error = str(exc)

    if recorded is not None and error is None and world.transcript != recorded["transcript"]:
        error = "replayed transcript does not match the recorded one"

    if args.record is not None and error is None:
        replay = build_replay(
            source_hash=args.source_hash,
            plan_hash=args.plan_hash,
            scenario=scenario,
            transcript=world.transcript,
        )
        args.record.write_text(json.dumps(replay, indent=2))

    print(
        json.dumps(
            {
                "scenario": scenario["name"],
                "passed": error is None,
                "error": error,
                "worker_attempts": runner._worker_attempts,
                "job_runs": runner._job_runs,
            }
        )
    )


if __name__ == "__main__":
    asyncio.run(main())
