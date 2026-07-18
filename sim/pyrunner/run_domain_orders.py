#!/usr/bin/env python3
"""v0.17 M6's live proof against the real generated `domain-orders`
project (the flagship 17UpdatePlan.md's M6 milestone names): regenerate
fresh, dump its `SimPlan` (see `crates/ciac-sim/examples/dump_plan.rs`)
next to it, `uv sync`, then run `inner_proof_domain_orders.py` inside
that project's own venv to exercise reference existence, `unique`, and
transaction rollback through real generated code -- no mocks.

Run: `python3 sim/pyrunner/run_domain_orders.py`
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from world import Schema  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[2]
PROJECT_DIR = REPO_ROOT / "target" / "sim-m6-checkpoint" / "domain-orders-py"
PYRUNNER_DIR = Path(__file__).resolve().parent


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    print(f"+ {' '.join(cmd)}", file=sys.stderr)
    try:
        return subprocess.run(cmd, check=True, **kwargs)
    except subprocess.CalledProcessError as e:
        if e.stdout:
            print(e.stdout, file=sys.stderr)
        if e.stderr:
            print(e.stderr, file=sys.stderr)
        raise


def build_project() -> None:
    if PROJECT_DIR.exists():
        shutil.rmtree(PROJECT_DIR)
    PROJECT_DIR.parent.mkdir(parents=True, exist_ok=True)
    run(
        [
            "cargo",
            "run",
            "-q",
            "--bin",
            "ciac",
            "--",
            "build",
            "examples/domain-orders.ciac",
            "--target",
            "python",
            "--out",
            str(PROJECT_DIR),
        ],
        cwd=REPO_ROOT,
    )


def dump_plan() -> None:
    """Writes the raw `SimPlan` JSON (see `dump_plan.rs`) into the
    generated project's own directory so `inner_proof_domain_orders.py`
    -- running inside that project's `uv` venv, a different Python
    interpreter than this orchestrator -- can read it as a plain file
    rather than needing this process's in-memory `Schema` object."""
    result = run(
        ["cargo", "run", "-q", "--example", "dump_plan", "-p", "ciac-sim", "--", "examples/domain-orders.ciac"],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
    )
    plan = json.loads(result.stdout)
    Schema.from_plan_json(plan)  # fails fast here if dump_plan.rs's shape ever drifts
    (PROJECT_DIR / "sim_plan.json").write_text(json.dumps(plan))


def uv_sync() -> None:
    run(["uv", "sync"], cwd=PROJECT_DIR)


def run_inner_proof() -> dict:
    full_env = dict(os.environ)
    full_env["PYTHONPATH"] = os.pathsep.join([str(PYRUNNER_DIR), str(PROJECT_DIR)])
    result = run(
        ["uv", "run", "python", str(PYRUNNER_DIR / "inner_proof_domain_orders.py")],
        cwd=PROJECT_DIR,
        env=full_env,
        capture_output=True,
        text=True,
    )
    print(result.stderr, file=sys.stderr)
    return json.loads(result.stdout)


def main() -> None:
    build_project()
    dump_plan()
    uv_sync()
    report = run_inner_proof()

    print(json.dumps(report, indent=2))

    print("\n=== M6 domain-orders infrastructure-free proof ===")
    for name, ok in report.items():
        if name == "all_pass":
            continue
        print(f"  [{'PASS' if ok else 'FAIL'}] {name}")

    print(f"\nDECISION: {'PASS' if report['all_pass'] else 'FAIL'}")
    sys.exit(0 if report["all_pass"] else 1)


if __name__ == "__main__":
    main()
