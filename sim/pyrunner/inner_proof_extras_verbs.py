"""Runs inside the generated `extras-verbs` project's own `uv` venv.
v0.17 M8's live proof for the remaining capability fakes: object
store, email, search, external HTTP -- all against real generated
Postgres/Redis/S3/SMTP/OpenSearch-target code, no mocks, no Docker.

Every call below goes through the real generated `Logic` class
directly (`EvictCache(...).handle(...)`, etc.), not through the
generated API route wrapper (`evict_cache_api`, etc.). This is a
disclosed workaround for a real, previously-invisible codegen bug this
proof surfaced: every route wrapper in this fixture unconditionally
does `result.model_dump(mode="json")` on its handler's return value,
but every handler here returns `Bool` or `[String]` (by design -- see
the fixture's own header comment), neither of which has a
`model_dump` method. Calling any of these routes live raises
`AttributeError`. No existing generated test catches it: the unit
tests call each `Logic` class directly, and the smoke test only checks
that each path is *listed* in the OpenAPI spec, never invokes it. See
17UpdatePlan.md's M8 milestone entry -- this is flagged as a real,
disclosed defect for a future fix, not something this milestone's own
capability fakes are chartered to repair.
"""

from __future__ import annotations

import asyncio
import json

from world import SimWorld

from app.state import AppState, set_current
from app.cache import get_cache
from app.object_store import get_object_store
from app.email import get_email
from app.search import get_search
from app.http_clients import get_http_client
from app.schemas import (
    HttpRequest,
    IndexRequest,
    KeyOnly,
    Notification,
    PrefixOnly,
    SearchRequest,
)
from app.logic.call_upstream import CallUpstream
from app.logic.evict_cache import EvictCache
from app.logic.index_doc import IndexDoc
from app.logic.list_docs import ListDocs
from app.logic.notify_user import NotifyUser
from app.logic.remove_doc import RemoveDoc
from app.logic.search_docs import SearchDocs


async def main() -> None:
    world = SimWorld(http_fixtures={"upstream": [{"status": 200, "json": {"accepted": True}}]})
    set_current(AppState.simulation(world))

    results: dict[str, object] = {}

    # -- object store: `put` has no generated handler in this fixture,
    # so it's exercised directly against the fake (disclosed, same
    # pattern M6 used for cascade/restrict delete); list/delete go
    # through the real generated handlers.
    store = get_object_store()
    await store.put("docs/a.txt", b"hello")
    await store.put("docs/b.txt", b"world")
    await store.put("other/c.txt", b"unrelated")
    results["object_store_get_roundtrip"] = (await store.get("docs/a.txt")) == b"hello"

    listed = await ListDocs(object_store=store).handle(PrefixOnly(prefix="docs/"))
    results["list_docs_only_matches_prefix"] = sorted(listed) == ["docs/a.txt", "docs/b.txt"]

    await RemoveDoc(object_store=store).handle(KeyOnly(key="docs/a.txt"))
    results["remove_doc_deleted_the_key"] = "docs/a.txt" not in await store.list("docs/")

    # -- cache: reuses M7's fake --
    await get_cache().set("some-key", "some-value")
    await EvictCache(cache=get_cache()).handle(KeyOnly(key="some-key"))
    results["evict_cache_deleted_the_key"] = await get_cache().get("some-key") is None

    # -- email --
    await NotifyUser(email=get_email()).handle(
        Notification(to="ada@example.com", subject="Hello", body="World")
    )
    sent = get_email().sent
    results["email_was_recorded"] = len(sent) == 1 and sent[0]["to"] == "ada@example.com"

    # -- search --
    await IndexDoc(search=get_search()).handle(IndexRequest(id="doc-1", title="Deterministic simulation"))
    await IndexDoc(search=get_search()).handle(IndexRequest(id="doc-2", title="Real Postgres constraints"))
    found = await SearchDocs(search=get_search()).handle(SearchRequest(query="simulation"))
    results["search_reports_success"] = found is True
    raw_hits = await get_search().search("documents", {"query": {"query_string": {"query": "simulation"}}})
    results["search_hit_is_the_expected_document"] = (
        len(raw_hits) == 1 and raw_hits[0]["value"] == "Deterministic simulation"
    )

    # -- external HTTP: fixture-driven, no real network --
    ok = await CallUpstream(http=get_http_client("upstream")).handle(
        HttpRequest(path="/charge", payload="42")
    )
    results["external_http_call_succeeded_against_fixture"] = ok is True

    try:
        await CallUpstream(http=get_http_client("upstream")).handle(
            HttpRequest(path="/charge", payload="42")
        )
        results["second_call_without_a_fixture_raises"] = False
    except RuntimeError:
        results["second_call_without_a_fixture_raises"] = True

    results["all_pass"] = all(v is True for v in results.values() if isinstance(v, bool))
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    asyncio.run(main())
