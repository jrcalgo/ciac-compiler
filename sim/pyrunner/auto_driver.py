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
common case -- its only parameter besides `payload` is named
`session`; anything else (auth claims, a second capability instance)
is refused with a clear, disclosed error naming the exact route and
its extra parameters, not a silent skip or an opaque crash.
"""

from __future__ import annotations

import argparse
import asyncio
import importlib
import inspect
import json
from pathlib import Path
from typing import Any

from pydantic import BaseModel

from cron import CronSchedule
from replay import ReplayError, build_replay, check_compatible
from scenario_runner import ApiEntry, JobEntry, ScenarioRunner, WorkerEntry
from world import SimWorld


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
) -> tuple[dict[str, WorkerEntry], dict[str, JobEntry]]:
    workers: dict[str, WorkerEntry] = {}
    for worker in plan.get("workers", []):
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


def build_apis(scenario: dict[str, Any]) -> dict[str, ApiEntry]:
    api_names = {
        step["request"]["api"] for step in scenario["steps"] if "request" in step
    }
    apis: dict[str, ApiEntry] = {}
    for api_name in api_names:
        snake = to_snake_case(api_name)
        module = importlib.import_module(f"app.api.{snake}")
        route = getattr(module, snake)
        params = [p for p in inspect.signature(route).parameters if p != "payload"]
        if params not in ([], ["session"]):
            raise RegistryError(
                f"api {api_name!r} (app.api.{snake}.{snake}) needs manual registration -- "
                f"auto-discovery only handles no extra parameters or a single `session` "
                f"parameter, found {params!r}"
            )
        payload_cls = _payload_constructor(route)
        needs_session = params == ["session"]

        async def call(payload: dict, _route=route, _cls=payload_cls, _needs=needs_session) -> Any:
            value = _cls(**payload) if _cls is not None else payload
            if _needs:
                from app.db import get_sessionmaker

                async with get_sessionmaker("default")() as session:
                    return await _route(value, session)
            return await _route(value)

        apis[api_name] = ApiEntry(service="", call=call)
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
        workers, jobs = build_workers_and_jobs(plan)
        apis = build_apis(scenario)
        failure_rules = build_failure_rules(scenario)
    except RegistryError as exc:
        print(json.dumps({"scenario": scenario["name"], "passed": False, "error": str(exc)}))
        return

    world = SimWorld(failure_rules=failure_rules)
    from app.state import AppState, set_current

    set_current(AppState.simulation(world))
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
