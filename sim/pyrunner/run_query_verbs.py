#!/usr/bin/env python3
"""27UpdatePlan.md M9's live proof against the real generated
`query-verbs` project: regenerate fresh, `uv sync`, then run
`inner_proof_query_verbs.py` inside that project's own venv to exercise
`db.update`/`db.query`/`db.count`/`db.delete_where` through real
generated handler classes and real SQLAlchemy `select`/`delete`
statement objects -- no mocks, and no hand-simulated statement shapes
(mirrors `run_domain_orders.py`'s own M6 pattern for `db.insert`/
references/transactions).

Run: `python3 sim/pyrunner/run_query_verbs.py`
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PROJECT_DIR = REPO_ROOT / "target" / "sim-m9-query-verbs" / "query-verbs-py"
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
            "examples/query-verbs.ciac",
            "--target",
            "python",
            "--out",
            str(PROJECT_DIR),
        ],
        cwd=REPO_ROOT,
    )


def uv_sync() -> None:
    run(["uv", "sync"], cwd=PROJECT_DIR)


def run_inner_proof() -> dict:
    full_env = dict(os.environ)
    full_env["PYTHONPATH"] = os.pathsep.join([str(PYRUNNER_DIR), str(PROJECT_DIR)])
    result = run(
        ["uv", "run", "python", str(PYRUNNER_DIR / "inner_proof_query_verbs.py")],
        cwd=PROJECT_DIR,
        env=full_env,
        capture_output=True,
        text=True,
    )
    print(result.stderr, file=sys.stderr)
    return json.loads(result.stdout)


def main() -> None:
    build_project()
    uv_sync()
    report = run_inner_proof()

    print(json.dumps(report, indent=2))

    print("\n=== M9 query-verbs infrastructure-free proof ===")
    for name, ok in report.items():
        if name == "all_pass":
            continue
        print(f"  [{'PASS' if ok else 'FAIL'}] {name}")

    print(f"\nDECISION: {'PASS' if report['all_pass'] else 'FAIL'}")
    sys.exit(0 if report["all_pass"] else 1)


if __name__ == "__main__":
    main()
