"""28UpdatePlan.md M3: direct tests of `world.py`'s M3 additions --
`namespaced_table_key` and `SimWorld.register_api`/`call_checked` --
mirroring `ciac_sim::world`'s own Rust unit tests (M2) for the same two
mechanisms, adapted to this file's asyncio idiom. Not run through any
generated driver on purpose: these are pure `world.py` mechanics, no
generated project needed to exercise them.

Run: `python3 sim/pyrunner/test_call_router.py`
"""

from __future__ import annotations

import asyncio

from world import RoutingError, SimWorld, namespaced_table_key


def test_namespaced_table_key_is_bare_for_the_single_service_degenerate_path() -> None:
    assert namespaced_table_key(None, "orders") == "orders"
    assert namespaced_table_key("Billing", "orders") == "Billing/orders"


def test_call_checked_routes_to_the_registered_handler_and_returns_its_response() -> None:
    async def run() -> None:
        w = SimWorld()

        async def handler(req: dict) -> dict:
            return {"echo": req}

        w.register_api("Callee", "Out", handler)
        result = await w.call_checked("Caller", "Callee", "Out", {"x": 1})
        assert result == {"echo": {"x": 1}}, result

    asyncio.run(run())


def test_call_checked_passes_through_the_handler_s_error_envelope_verbatim() -> None:
    async def run() -> None:
        w = SimWorld()

        async def raises(req: dict) -> dict:
            raise ValueError("boom")

        w.register_api("Svc", "Fails", raises)
        try:
            await w.call_checked("Caller", "Svc", "Fails", {})
        except ValueError as exc:
            assert str(exc) == "boom"
        else:
            raise AssertionError("expected ValueError")
        assert w._call_depth == 0, "depth must unwind even on handler exception"

    asyncio.run(run())


def test_call_checked_refuses_an_unregistered_api_with_a_clear_error() -> None:
    async def run() -> None:
        w = SimWorld()
        try:
            await w.call_checked("Caller", "Callee", "Missing", {})
        except RoutingError as exc:
            assert "no handler registered for Callee.Missing (called from Caller)" in str(exc)
        else:
            raise AssertionError("expected RoutingError")

    asyncio.run(run())


def test_call_checked_honors_an_injected_call_request_failure() -> None:
    async def run() -> None:
        w = SimWorld(failure_rules=[("call.request", "Out", 1)])

        async def handler(req: dict) -> dict:
            return req

        w.register_api("Svc", "Out", handler)
        try:
            await w.call_checked("Caller", "Svc", "Out", {})
        except RuntimeError as exc:
            assert "simulated call.request failure" in str(exc)
        else:
            raise AssertionError("expected the injected call.request failure")

    asyncio.run(run())


def test_call_checked_depth_guard_refuses_runaway_recursion() -> None:
    async def run() -> None:
        w = SimWorld()

        async def recurse(req: dict) -> dict:
            return await w.call_checked("Callee", "Callee", "Out", req)

        w.register_api("Callee", "Out", recurse)
        try:
            await w.call_checked("Caller", "Callee", "Out", {})
        except RoutingError as exc:
            assert f"depth exceeded {SimWorld.MAX_CALL_DEPTH}" in str(exc)
        else:
            raise AssertionError("expected the depth guard to refuse")
        assert w._call_depth == 0, "depth must unwind back to zero after the guard fires"

    asyncio.run(run())


def test_re_registering_the_same_service_api_replaces_the_handler() -> None:
    async def run() -> None:
        w = SimWorld()

        async def first(req: dict) -> str:
            return "first"

        async def second(req: dict) -> str:
            return "second"

        w.register_api("Svc", "Api", first)
        w.register_api("Svc", "Api", second)
        result = await w.call_checked("Caller", "Svc", "Api", {})
        assert result == "second"

    asyncio.run(run())


def main() -> None:
    tests = [v for k, v in globals().items() if k.startswith("test_") and callable(v)]
    for test in tests:
        test()
        print(f"  ok  {test.__name__}")
    print(f"{len(tests)} tests passed")


if __name__ == "__main__":
    main()
