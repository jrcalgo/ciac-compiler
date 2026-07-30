"""28UpdatePlan.md M3c: the N-service Python system driver's own
package-aliasing seam.

Every generated Python project's top-level package is literally named
`app` (`app/__init__.py`, `app/api/*.py`, ...), and every generated
file that reaches into another module of its own project does so with
an *absolute* import of that literal name (`from app.db import
get_sessionmaker`, `from app.state import current`, ...) -- confirmed
empirically across `db.py.j2`, `state.py.j2`, `api.py.j2`,
`client.py.j2`. Loading N such projects into one Python process the
straightforward way (`sys.path` holding all N project directories at
once) is unsound: whichever project's `app/db.py` happens to resolve
first on `sys.path` wins for *every* service's `from app.db import
...`, silently misrouting every other service's database access to the
wrong project's module -- not a hypothetical, this was reproduced and
confirmed live in this milestone's own scratch testing before writing
`ServiceModules` below.

Rewriting every generated template's imports to some per-service alias
(`app_billing.db` instead of `app.db`) would fix this at the source,
but it is a far larger, riskier change than this milestone's own scope
-- it touches every Python backend template, not just the multi-service
composition path, and would make every *single*-service generated
project's own imports inconsistent with the multi-service ones for no
benefit to single-service users. `ServiceModules` instead accepts the
literal-`app`-name constraint and works around it with a
capture-once/swap-before-invoke discipline: each service's fully
imported `app.*` module tree is captured into a private dict once (at
registration time, with `sys.path` scoped to just that project so a
not-yet-cached submodule resolves to the right files), then that whole
tree is written back into `sys.modules` immediately before invoking any
of that service's code.

This is sound specifically because this driver's event loop never runs
two coroutines' bodies concurrently on the same thread: `ScenarioRunner`
(`scenario_runner.py`) awaits each request/drain/advance step in strict
sequence, never via `asyncio.gather` or a spawned task that could
interleave with another service's own call. "Which service's `app.*` is
currently live in `sys.modules`" is therefore a well-defined, single
value at every point in the program's execution; swapping it in right
before a call -- and never mid-call -- is enough. A *lazy* import
executed at call time (`get_sessionmaker`'s own `from app.db import
...` inside `db.py.j2`, reached only when a route actually runs, not
when its module is first loaded) is handled by the same mechanism: as
long as `app.db` was already imported (and thus cached in
`sys.modules`) during that service's own `load()` pass, the lazy
import is a cache hit against the just-`activate()`d tree, not a fresh
disk lookup that could resolve to the wrong project.
"""

from __future__ import annotations

import importlib
import pkgutil
import sys
from pathlib import Path
from types import ModuleType


def _import_all_submodules(package_name: str) -> None:
    """Imports every `.py` file under `package_name` recursively, so a
    module reached only via a call-time (not load-time) import --
    `client.py.j2`'s own lazy `from app.db import ...`, for instance --
    is already cached in `sys.modules` before this service's tree is
    captured. `ModuleNotFoundError` for a package a given project
    simply doesn't have (not every service declares `db`, `queue`,
    call clients, ...) is the caller's concern, not this helper's."""
    package = importlib.import_module(package_name)
    package_path = getattr(package, "__path__", None)
    if package_path is None:
        return
    for module_info in pkgutil.iter_modules(package_path, prefix=f"{package_name}."):
        importlib.import_module(module_info.name)
        if module_info.ispkg:
            _import_all_submodules(module_info.name)


# Every module this milestone's generated projects might reach via a
# call-time (not load-time) import -- not every service declares all of
# these (a service with no `db` capability has no `app/db.py` at all),
# so each import is individually optional.
_OPTIONAL_TOP_LEVEL_MODULES = (
    "app.db",
    "app.queue",
    "app.cache",
    "app.object_store",
    "app.email",
    "app.search",
    "app.http_clients",
    "app.auth",
    "app.observability",
)

# Packages (not single modules) worth walking exhaustively: every route,
# worker, job, and call client a scenario or another service's call
# might reach, whether or not this milestone's own scenario corpus
# happens to exercise it.
_OPTIONAL_PACKAGES = ("app.api", "app.workers", "app.clients", "app.services")


class ServiceModules:
    """One service's captured `app.*` module tree. See the module
    docstring for why capturing once and swapping before every call is
    sound here."""

    def __init__(self, service: str, project_dir: Path) -> None:
        self.service = service
        self.project_dir = project_dir
        self._modules: dict[str, ModuleType] = {}

    def load(self) -> None:
        """Imports this service's own `app` package fresh -- evicting
        any other service's `app`/`app.*` residue from `sys.modules`
        first, and restricting `sys.path` to just this project
        directory for the duration of the import so a not-yet-cached
        submodule resolves to *this* project's file -- then captures
        every `app`/`app.*` key `sys.modules` now holds. Call once per
        service, in declaration order, before any scenario runs."""
        for name in [m for m in sys.modules if m == "app" or m.startswith("app.")]:
            del sys.modules[name]
        importlib.invalidate_caches()
        original_path = list(sys.path)
        sys.path.insert(0, str(self.project_dir))
        try:
            importlib.import_module("app")
            importlib.import_module("app.state")
            for pkg in _OPTIONAL_PACKAGES:
                try:
                    _import_all_submodules(pkg)
                except ModuleNotFoundError:
                    pass
            for mod in _OPTIONAL_TOP_LEVEL_MODULES:
                try:
                    importlib.import_module(mod)
                except ModuleNotFoundError:
                    pass
        finally:
            sys.path[:] = original_path
        self._modules = {
            name: mod
            for name, mod in sys.modules.items()
            if name == "app" or name.startswith("app.")
        }

    def activate(self) -> None:
        """Overwrites every `app`/`app.*` entry currently in
        `sys.modules` with this service's own captured tree. Call
        immediately before invoking any callable that belongs to this
        service -- an api route, a worker's `handle_message_once`, a
        job's `handle_tick_once`, or a call client's method -- and
        never in the middle of one (see the module docstring for why
        that invariant is enough)."""
        sys.modules.update(self._modules)
