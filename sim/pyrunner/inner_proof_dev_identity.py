"""Runs inside the generated `dev-identity` project's own `uv` venv.
v0.17 M8's live proof for the auth capability fake: "dev-identity
scope behavior passes with fake JWKS and no Keycloak process" (this
document's own M8 wording). See `sim/pyrunner/world.py`'s `FakeAuth`
docstring for why this bypasses JWT/JWKS verification entirely rather
than faking the JWKS HTTP round-trip -- the observable outcome (no
Keycloak process, no real crypto, scope enforcement still real) is the
same either way.
"""

from __future__ import annotations

import asyncio
import json

from fastapi import HTTPException
from fastapi.security import HTTPAuthorizationCredentials
from world import SimWorld

from app.state import AppState, set_current
from app.db import get_sessionmaker
from app.auth import require_auth, require_scope
from app.models import AccountIn
from app.api.account import _store, create_account, get_account


async def verified_claims(token: str) -> dict:
    creds = HTTPAuthorizationCredentials(scheme="Bearer", credentials=token)
    return await require_auth(creds)


async def main() -> None:
    world = SimWorld()
    set_current(AppState.simulation(world))

    world.auth.issue("writer-token", {"sub": "alice", "scope": "accounts:write accounts:read"})
    world.auth.issue("reader-only-token", {"sub": "bob", "scope": "accounts:read"})
    world.auth.issue("soon-expired-token", {"sub": "carol", "scope": "accounts:read"}, expires_in_ms=5_000)

    results: dict[str, object] = {}

    # -- a valid token with the write scope can create an account --
    async with get_sessionmaker()() as session:
        write_claims = await require_scope("accounts:write")(claims=await verified_claims("writer-token"))
        created = await create_account(AccountIn(email="ada@example.com"), write_claims, _store(session=session))
    results["write_scoped_token_created_account"] = created.email == "ada@example.com"

    # -- a read-only token is rejected for a write-scoped route --
    async with get_sessionmaker()() as session:
        try:
            read_claims = await verified_claims("reader-only-token")
            await require_scope("accounts:write")(claims=read_claims)
            results["read_only_token_rejected_for_write"] = False
        except HTTPException as exc:
            results["read_only_token_rejected_for_write"] = exc.status_code == 403

    # -- the read-only token *can* read --
    async with get_sessionmaker()() as session:
        read_claims = await require_scope("accounts:read")(claims=await verified_claims("reader-only-token"))
        fetched = await get_account(created.id, read_claims, _store(session=session))
    results["read_scoped_token_can_read"] = fetched.id == created.id

    # -- an unknown token is rejected outright --
    try:
        await verified_claims("no-such-token")
        results["unknown_token_rejected"] = False
    except HTTPException as exc:
        results["unknown_token_rejected"] = exc.status_code == 401

    # -- token expiry is virtual-time-driven, no real sleep --
    claims = await verified_claims("soon-expired-token")
    results["token_valid_before_expiry"] = claims["sub"] == "carol"
    world.clock.advance_by(6_000)
    try:
        await verified_claims("soon-expired-token")
        results["token_rejected_after_virtual_expiry"] = False
    except HTTPException as exc:
        results["token_rejected_after_virtual_expiry"] = exc.status_code == 401

    results["all_pass"] = all(v is True for v in results.values() if isinstance(v, bool))
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    asyncio.run(main())
