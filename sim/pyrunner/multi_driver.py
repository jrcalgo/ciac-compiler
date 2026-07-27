"""28UpdatePlan.md M3c: the bounded child protocol's Python side, for a
multi-service system -- `auto_driver.py`'s counterpart, not a
replacement (a single-service `ciac sim` invocation still uses
`auto_driver.py` unchanged; `ciac sim`'s own dispatch in `commands.rs`
picks whichever of the two matches the generated output on disk).

Same one-process-one-scenario-one-JSON-reply contract as `auto_driver.
py`, but loads N generated projects' own `app` packages into this one
process against one shared `world.SimWorld`, registering each
service's routes/workers/jobs in declaration order and routing
`call <Service>.<Api>` steps through `world.call_checked` instead of
real HTTP (28's M3b: `client.py.j2`'s own world-guard branch).

The package-aliasing seam (every generated project's top-level package
is literally named `app`) is `multi_service.ServiceModules` -- see its
own module docstring for why the capture-once/swap-before-invoke
discipline it implements is sound for this driver's single-threaded
asyncio event loop.

Disclosed, not yet done: table namespacing. `world.py`'s M3a
`namespaced_table_key` exists, but nothing in this driver (or in
`ciac-backend-python`'s `db.py.j2`) yet threads a per-service key into
`world.db`'s calls -- two services sharing an identical table name
would collide in the single shared `FakeDatabase` today. This
milestone's own live-proof fixture (`multi-service-media.ciac`)
declares no `table` at all in any of its five services, so the gap is
real but unexercised by that proof; closing it is deferred to the
system-corpus milestone (28's M4) once a table-bearing multi-service
example actually needs it.
"""

from __future__ import annotations

import argparse
import asyncio
import importlib
import inspect
import json
import sys
from pathlib import Path
from typing import Any

from auto_driver import (
    RegistryError,
    _payload_constructor,
    _resolve_claims,
    apply_given,
    build_failure_rules,
    to_snake_case,
)
from cron import CronSchedule
from multi_service import ServiceModules
from replay import ReplayError, build_replay, check_compatible
from scenario_runner import ApiEntry, JobEntry, ScenarioRunner, WorkerEntry
from world import Schema, SimWorld


def _snapshot_active_app_modules() -> dict[str, Any]:
    return {name: mod for name, mod in sys.modules.items() if name == "app" or name.startswith("app.")}


def _service_scoped(modules: ServiceModules, fn):
    """Wraps an async callable so `modules` is the active service tree
    for its entire body, restoring whatever was active beforehand on
    return -- success or exception -- once it's done. Symmetric at any
    nesting depth (mirrors `world.call_checked`'s own depth-guard
    discipline): if this call itself routes into another service via
    `call_checked`, that call gets the *same* wrapping around its own
    handler, so control always unwinds back through exactly the chain
    of "whichever service was active" it was invoked through, without
    any call site needing to know about `ServiceModules` at all."""

    async def wrapped(*args: Any, **kwargs: Any) -> Any:
        caller_snapshot = _snapshot_active_app_modules()
        modules.activate()
        try:
            return await fn(*args, **kwargs)
        finally:
            sys.modules.update(caller_snapshot)

    return wrapped


def build_service_workers_jobs_apis(
    plan: dict[str, Any],
    scenario: dict[str, Any],
    service: dict[str, Any],
    modules: ServiceModules,
) -> tuple[dict[str, WorkerEntry], dict[str, JobEntry], dict[tuple[str, str], ApiEntry]]:
    """Like `auto_driver.build_workers_and_jobs`/`build_apis`, but for
    one service of an N-service system: every handler this returns is
    wrapped with `_service_scoped(modules, ...)` so it always runs with
    `service`'s own `app.*` tree active, regardless of which service's
    tree happened to be active when the driver decided to call it."""
    service_name = service["name"]
    service_key = service["key"]

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
            service=service_key,
            subject=module.SUBJECT,
            queue_group=module.QUEUE_GROUP,
            handle_once=_service_scoped(modules, handle),
            max_retries=module.MAX_RETRIES,
        )

    jobs: dict[str, JobEntry] = {}
    for job in plan.get("jobs", []):
        if job.get("service_key") != service_key:
            continue
        snake = to_snake_case(job["name"])
        job_module = importlib.import_module(f"app.workers.{snake}")
        jobs[job["name"]] = JobEntry(
            service=service_key,
            schedule=CronSchedule.parse(job_module.SCHEDULE),
            handle_tick=_service_scoped(modules, job_module.handle_tick_once),
        )

    # Unlike single-service `build_apis` (which only ever needs to
    # register whatever the scenario's own `request` steps name),
    # multi-service registration must cover *every* api this service
    # declares, not just the ones a `request` step names directly: an
    # api reachable only via a routed `call <Service>.<Api>` from
    # another service (never itself the target of a scenario `request`
    # step) still needs a `world.register_api` entry or that call has
    # no handler to route to (confirmed by reproducing exactly this
    # failure -- `RoutingError: no handler registered for Billing.
    # Charge` -- while authoring this milestone's own live proof, where
    # `Billing.Charge` is called only via `UploadApi`'s own routed call,
    # never a scenario `request` step in its own right).
    api_names = {api["name"] for api in plan.get("apis", []) if api.get("service_key") == service_key}
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

        apis[(service_name, api_name)] = ApiEntry(service=service_name, call=_service_scoped(modules, call))
    return workers, jobs, apis


async def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("plan_path", type=Path)
    parser.add_argument("scenario_path", type=Path)
    parser.add_argument("--source-hash")
    parser.add_argument("--plan-hash")
    parser.add_argument("--record", type=Path)
    parser.add_argument("--replay", type=Path)
    parser.add_argument(
        "--service",
        action="append",
        default=[],
        metavar="NAME=PATH",
        help="repeatable, one per service, in declaration order",
    )
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

    service_dirs: dict[str, Path] = {}
    for spec in args.service:
        name, _, path = spec.partition("=")
        service_dirs[name] = Path(path)

    try:
        failure_rules = build_failure_rules(scenario)
    except RegistryError as exc:
        print(json.dumps({"scenario": scenario["name"], "passed": False, "error": str(exc)}))
        return
    world = SimWorld(failure_rules=failure_rules, schema=Schema.from_plan_json(plan))

    runner = ScenarioRunner(world=world, multi_service=True)
    try:
        for service in plan["services"]:
            name = service["name"]
            project_dir = service_dirs[name]
            modules = ServiceModules(name, project_dir)
            modules.load()

            modules.activate()
            from app.state import AppState, set_current  # noqa: PLC0415

            set_current(AppState.simulation(world))

            workers, jobs, apis = build_service_workers_jobs_apis(plan, scenario, service, modules)
            runner.workers.update(workers)
            runner.jobs.update(jobs)
            runner.apis.update(apis)

            for api_name, api in apis.items():
                world.register_api(name, api_name[1], _make_call_checked_handler(api))
    except RegistryError as exc:
        print(json.dumps({"scenario": scenario["name"], "passed": False, "error": str(exc)}))
        return

    await apply_given(world, scenario)

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


def _make_call_checked_handler(entry: ApiEntry):
    """Adapts one registered `ApiEntry.call(payload, principal)` (the
    scenario-runner shape, taking a resolved `principal` dict) to the
    `world.call_checked`/`register_api` shape (`handler(payload_dict) ->
    Any`, no principal -- a routed cross-service call carries no
    scenario-level `as` principal of its own, matching production: the
    real HTTP call a `call <Service>.<Api>` step issues carries no
    caller identity either, see `client.py.j2`)."""

    async def handler(payload: dict[str, Any]) -> Any:
        return await entry.call(payload, None)

    return handler


if __name__ == "__main__":
    asyncio.run(main())
