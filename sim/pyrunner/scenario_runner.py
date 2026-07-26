"""v0.17 M9: a generic interpreter for `ciac_sim::scenario::Scenario`
JSON -- the closed step vocabulary (`request`/`publish`/`advance`/
`drain`/`expect`), executed against a real generated project's routes
and worker/job entry points through `world.SimWorld` (M5-M8's fakes).

This replaces M5-M8's own hand-written per-fixture translations
(`inner_proof_*.py`'s manual step-by-step scripts) with one interpreter
that reads the checked-in scenario JSON directly -- the gap every prior
milestone in this arc disclosed ("no generic JSON-scenario
interpreter... M9/M10's job").

What this interpreter is not:

- Not auto-discovering. The caller supplies a small registry mapping
  each scenario's `api`/`worker`/`job`/`stream` *names* to the actual
  generated callables (`ScenarioRunner(world, apis={...}, workers=
  {...}, jobs={...}, streams={...})`) -- resolving those names against
  a live Python module tree automatically (walking `SimPlan` the way
  `ciac sim` eventually will) is bounded-child-protocol territory,
  M10's job, not this interpreter's.
- Not multi-service. One `ScenarioRunner` drives one service's
  registry; loading every service of a multi-service project into one
  process is a separate, larger capability -- see 17UpdatePlan.md's M9
  milestone entry for why it's deferred, not silently assumed to work.
- `expect.response.status` does not check a real HTTP status code --
  routes are called as plain functions, not through a running ASGI
  stack (the same disclosed gap M5's own `call_place_order_api`
  named). `200` here means "the call did not raise"; a non-2xx-shaped
  scenario expectation is out of this interpreter's reach until a real
  request boundary is simulated (M9's own "handler scaffolds, complete"
  deferred to a further pass).
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Awaitable, Callable

from cron import CronSchedule, parse_duration_ms
from world import SEARCH_INDEX_NAME


class ScenarioAssertionError(AssertionError):
    """One `expect` step's condition did not hold."""


@dataclass
class ApiEntry:
    """One registered API: `call(payload_json) -> Any`, raising on
    failure. `service` is the scenario's own name for the owning
    service, matching `request.service` -- unused for routing today
    (single-service only, see module docstring) but recorded so a
    scenario naming the wrong service is at least visible in the
    registry, not silently ignored."""

    service: str
    call: Callable[[dict[str, Any], dict[str, Any] | None], Awaitable[Any]]


@dataclass
class WorkerEntry:
    """`handle_once` is the generated `handle_message_once` (M1's
    confirmed real per-attempt entry point), not `handle_message` --
    the scenario runner drives its own retry loop
    (`SimWorld.deliver_counting_attempts`) so `worker_attempts` can
    count real attempts, matching M4's `retry_eligible` design intent."""

    service: str
    subject: str
    queue_group: str
    handle_once: Callable[[dict[str, Any]], Awaitable[Any]]
    max_retries: int


@dataclass
class JobEntry:
    service: str
    schedule: CronSchedule
    handle_tick: Callable[[], Awaitable[Any]]


@dataclass
class StreamEntry:
    service: str
    subject: str


@dataclass
class ScenarioRunner:
    world: Any  # world.SimWorld
    apis: dict[str, ApiEntry] = field(default_factory=dict)
    workers: dict[str, WorkerEntry] = field(default_factory=dict)
    jobs: dict[str, JobEntry] = field(default_factory=dict)
    streams: dict[str, StreamEntry] = field(default_factory=dict)

    _saved: dict[str, tuple[bool, Any]] = field(default_factory=dict, init=False)
    _worker_attempts: dict[str, int] = field(default_factory=dict, init=False)
    _job_runs: dict[str, int] = field(default_factory=dict, init=False)

    async def run(self, scenario: dict[str, Any]) -> None:
        if scenario.get("simulation_version") != 1:
            raise ScenarioAssertionError(
                f"unsupported simulation_version {scenario.get('simulation_version')!r}"
            )
        for step in scenario["steps"]:
            await self._run_step(step)

    async def _run_step(self, step: dict[str, Any]) -> None:
        if "request" in step:
            await self._request(step["request"])
        elif "publish" in step:
            await self._publish(step["publish"])
        elif "advance" in step:
            await self._advance(step["advance"]["by"])
        elif "drain" in step:
            await self._drain()
        elif "expect" in step:
            await self._expect(step["expect"])
        else:
            raise ScenarioAssertionError(f"unrecognized scenario step: {step!r}")

    async def _request(self, spec: dict[str, Any]) -> None:
        entry = self.apis[spec["api"]]
        ok, value = True, None
        try:
            value = await entry.call(spec.get("json", {}), spec.get("as"))
        except Exception as exc:  # noqa: BLE001 -- recorded, not swallowed
            ok, value = False, exc
        if spec.get("save_as"):
            self._saved[spec["save_as"]] = (ok, value)

    async def _publish(self, spec: dict[str, Any]) -> None:
        entry = self.streams[spec["stream"]]
        await self.world.publish(entry.subject, spec.get("json", {}))

    async def _advance(self, by: str) -> None:
        delta_ms = parse_duration_ms(by)
        from_ms = self.world.clock.now_ms()
        to_ms = from_ms + delta_ms
        # Fire every job whose schedule has a due instant in this
        # window before advancing the clock the rest of the way, so
        # `handle_tick_once` observes the virtual time it actually
        # fired at, not the window's end.
        for job_name, job in self.jobs.items():
            for _fire_ms in job.schedule.due_instants(from_ms, to_ms):
                await job.handle_tick()
                self._job_runs[job_name] = self._job_runs.get(job_name, 0) + 1
        self.world.clock.advance_by(delta_ms)

    async def _drain(self) -> None:
        for worker_name, worker in self.workers.items():
            attempts = await self.world.deliver_counting_attempts(
                worker.subject, worker.queue_group, worker.handle_once, worker.max_retries
            )
            self._worker_attempts[worker_name] = (
                self._worker_attempts.get(worker_name, 0) + attempts
            )

    async def _expect(self, spec: dict[str, Any]) -> None:
        if "response" in spec:
            self._expect_response(spec["response"])
        elif "row" in spec:
            self._expect_row(spec["row"])
        elif "worker_attempts" in spec:
            self._expect_worker_attempts(spec["worker_attempts"])
        elif "job_runs" in spec:
            self._expect_job_runs(spec["job_runs"])
        elif "quiescence" in spec:
            self._expect_quiescence()
        elif "email" in spec:
            self._expect_email(spec["email"])
        elif "cache" in spec:
            await self._expect_cache(spec["cache"])
        elif "object" in spec:
            await self._expect_object(spec["object"])
        elif "search_hits" in spec:
            await self._expect_search_hits(spec["search_hits"])
        elif "http_calls" in spec:
            self._expect_http_calls(spec["http_calls"])
        else:
            raise ScenarioAssertionError(f"unrecognized expect step: {spec!r}")

    def _expect_response(self, spec: dict[str, Any]) -> None:
        of = spec.get("of")
        if of not in self._saved:
            raise ScenarioAssertionError(f"expect.response.of references unknown save_as {of!r}")
        ok, value = self._saved[of]
        want_status = spec.get("status")
        if want_status is not None:
            # See module docstring: no real HTTP transport, so "2xx"
            # means "the call didn't raise," nothing finer-grained.
            is_2xx = 200 <= want_status < 300
            if is_2xx != ok:
                raise ScenarioAssertionError(
                    f"expect.response.of={of!r}: expected "
                    f"{'success' if is_2xx else 'failure'}, call {'raised' if not ok else 'succeeded'}"
                )
        # 27UpdatePlan.md M4: this was silently never checked -- found
        # via Rust's own stricter `expect.response.json` implementation
        # catching a genuine wrong expectation in
        # `sim/relational-depth.ciac-sim.json` that this gap had let
        # through since M2. Only meaningful on the success path (`ok`);
        # `value` is the raised exception itself otherwise, not a value
        # comparable to a JSON body.
        want_json = spec.get("json")
        if want_json is not None and ok:
            if value != want_json:
                raise ScenarioAssertionError(
                    f"expect.response.of={of!r}: expected json {want_json!r}, got {value!r}"
                )

    def _expect_row(self, spec: dict[str, Any]) -> None:
        table = spec["table"]
        where = spec.get("where", {})
        rows = self.world.db.snapshot().get(table, {}).values()
        found = any(all(row.get(k) == v for k, v in where.items()) for row in rows)
        if found != spec["present"]:
            raise ScenarioAssertionError(
                f"expect.row: table={table!r} where={where!r} present={spec['present']!r}, found={found}"
            )

    def _expect_worker_attempts(self, spec: dict[str, Any]) -> None:
        actual = self._worker_attempts.get(spec["worker"], 0)
        if actual != spec["count"]:
            raise ScenarioAssertionError(
                f"expect.worker_attempts: worker={spec['worker']!r} expected {spec['count']}, got {actual}"
            )

    def _expect_job_runs(self, spec: dict[str, Any]) -> None:
        actual = self._job_runs.get(spec["job"], 0)
        if actual != spec["count"]:
            raise ScenarioAssertionError(
                f"expect.job_runs: job={spec['job']!r} expected {spec['count']}, got {actual}"
            )

    def _expect_quiescence(self) -> None:
        for worker_name, worker in self.workers.items():
            pending = self.world.broker.pending_count(worker.subject, worker.queue_group)
            if pending:
                raise ScenarioAssertionError(
                    f"expect.quiescence: worker {worker_name!r} still has {pending} undelivered message(s)"
                )

    def _expect_email(self, spec: dict[str, Any]) -> None:
        # 27UpdatePlan.md M1 (`ciac_sim::scenario::ExpectStep::Email`)
        # carries no `instance` field -- it counts sends across every
        # `email` instance the world has seen, not one named sender.
        to = spec.get("to")
        subject_contains = spec.get("subject_contains")
        sent = [msg for fake in self.world._emails.values() for msg in fake.sent]
        if to is not None:
            sent = [msg for msg in sent if msg["to"] == to]
        if subject_contains is not None:
            sent = [msg for msg in sent if subject_contains in msg["subject"]]
        actual = len(sent)
        if actual != spec["count"]:
            raise ScenarioAssertionError(
                f"expect.email: to={to!r} subject_contains={subject_contains!r} "
                f"expected {spec['count']}, got {actual}"
            )

    async def _expect_cache(self, spec: dict[str, Any]) -> None:
        cached = await self.world.fake_cache(spec["instance"]).get(spec["key"])
        present = cached is not None
        if present != spec["present"]:
            raise ScenarioAssertionError(
                f"expect.cache: instance={spec['instance']!r} key={spec['key']!r} "
                f"expected present={spec['present']!r}, found present={present}"
            )
        if spec.get("value") is not None:
            actual_value = json.loads(cached) if cached is not None else None
            if actual_value != spec["value"]:
                raise ScenarioAssertionError(
                    f"expect.cache: instance={spec['instance']!r} key={spec['key']!r} "
                    f"expected value={spec['value']!r}, got {actual_value!r}"
                )

    async def _expect_object(self, spec: dict[str, Any]) -> None:
        try:
            await self.world.fake_object_store(spec["store"]).get(spec["key"])
            present = True
        except KeyError:
            present = False
        if present != spec["present"]:
            raise ScenarioAssertionError(
                f"expect.object: store={spec['store']!r} key={spec['key']!r} "
                f"expected present={spec['present']!r}, found present={present}"
            )

    async def _expect_search_hits(self, spec: dict[str, Any]) -> None:
        docs = await self.world.fake_search(spec["instance"]).search(
            SEARCH_INDEX_NAME, {"query": {"query_string": {"query": spec["query"]}}}
        )
        actual = len(docs)
        if actual != spec["count"]:
            raise ScenarioAssertionError(
                f"expect.search_hits: instance={spec['instance']!r} query={spec['query']!r} "
                f"expected {spec['count']}, got {actual}"
            )

    def _expect_http_calls(self, spec: dict[str, Any]) -> None:
        actual = len(self.world.fake_http_client(spec["instance"]).requests)
        if actual != spec["count"]:
            raise ScenarioAssertionError(
                f"expect.http_calls: instance={spec['instance']!r} expected {spec['count']}, got {actual}"
            )
