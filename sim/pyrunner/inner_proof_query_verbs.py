"""Runs inside the generated `query-verbs` project's own `uv` venv
(invoked as `uv run python inner_proof_query_verbs.py`). 27UpdatePlan.md
M9's live proof: real generated SQLite-target Python code, driven
through `world.SimWorld`'s now-complete `FakeDatabase`/`_FakeSession`,
exercising `db.query`/`db.count`/`db.delete_where` (predicate-filtered,
via real `select`/`sql_delete` statement objects and `_compile_predicate`)
and `db.update` (via `_FakeSession.get()`-then-`setattr` mutation
tracking) -- the two verb families v0.17-v0.26 never faked. `db.delete`
(plain, by id) is exercised too, alongside the read-only-query-adds-no-
transcript-effect guard `_FakeSession`'s own docstring commits to.

Exercises the `app.logic.*` handler classes directly rather than the
generated `app.api.*_api` route wrappers: found live while writing this
proof, every `api ... -> Return` pipeline whose final handler returns a
non-`Record` type (`list[Note]`, `Int`, `Bool` -- exactly `ListActive`/
`CountActive`/`DeleteByActive`/`Remove` here) crashes with
`AttributeError: 'list'/'int'/'bool' object has no attribute
'model_dump'` at `api.py.j2:103`, which unconditionally calls
`result.model_dump(mode="json")` regardless of the pipeline's actual
return type. This is a real, pre-existing Python codegen defect --
predating this arc, affecting the real production HTTP path exactly as
much as simulation, and orthogonal to 27UpdatePlan.md's simulation-
fidelity charter -- disclosed in `27UpdatePlan.md`'s M9 Shipped note as
an explicit out-of-scope finding, not fixed here.
"""

from __future__ import annotations

import asyncio
import json

from world import SimWorld

from app.state import AppState, set_current
from app.db import get_sessionmaker
from app.schemas import ActiveFilter, IdOnly, NoteUpdate
from app.logic.list_active import ListActive
from app.logic.count_active import CountActive
from app.logic.delete_by_active import DeleteByActive
from app.logic.replace import Replace
from app.logic.remove import Remove


async def call(handler_cls, payload):
    async with get_sessionmaker("default")() as session:
        return await handler_cls(session).handle(payload)


async def main() -> None:
    world = SimWorld()
    set_current(AppState.simulation(world))

    results: dict[str, object] = {}

    world.db.insert("notes", "1", {"id": "1", "title": "hello world", "active": True})
    world.db.insert("notes", "2", {"id": "2", "title": "goodbye", "active": False})
    world.db.insert("notes", "3", {"id": "3", "title": "hello there", "active": True})

    active_notes = await call(ListActive, ActiveFilter(active=True))
    results["query_filters_by_predicate"] = sorted(n.id for n in active_notes) == ["1", "3"]

    inactive_count = await call(CountActive, ActiveFilter(active=False))
    results["count_filters_by_predicate"] = inactive_count == 1

    transcript_len_before_read = len(world.transcript)
    await call(ListActive, ActiveFilter(active=False))
    results["read_only_query_adds_no_transcript_effect"] = (
        len(world.transcript) == transcript_len_before_read
    )

    updated = await call(Replace, NoteUpdate(id="1", title="HELLO WORLD", active=False))
    row_after_update = world.db.get("notes", "1")
    results["update_persists_setattr_mutation"] = (
        row_after_update is not None
        and row_after_update["title"] == "HELLO WORLD"
        and row_after_update["active"] is False
    )
    results["update_response_reflects_new_value"] = updated.title == "HELLO WORLD"
    results["update_records_transcript_effect"] = (
        world.transcript[-1] == {"effect": "db.update", "subject": "notes"}
    )

    deleted_count = await call(DeleteByActive, ActiveFilter(active=True))
    results["delete_where_deletes_matched_rows_only"] = (
        deleted_count == 1
        and world.db.get("notes", "3") is None
        and world.db.get("notes", "2") is not None
    )

    removed = await call(Remove, IdOnly(id="2"))
    # id=1 (updated, not deleted) is the only row left after id=3 was
    # removed by delete_where and id=2 by this plain delete.
    results["plain_delete_by_id_still_works"] = (
        removed is True and world.db.count("notes") == 1 and world.db.get("notes", "1") is not None
    )

    results["all_pass"] = all(v is True for k, v in results.items() if k != "all_pass")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    asyncio.run(main())
