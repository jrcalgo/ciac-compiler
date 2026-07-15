#!/usr/bin/env python3
"""v0.17 M8's live proof against the real generated `extras-verbs`
project: object store, email, search, external HTTP -- no mocks, no
Docker, no real network.

Run: `python3 sim/pyrunner/run_extras_verbs.py`
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PROJECT_DIR = REPO_ROOT / "target" / "sim-m8-checkpoint" / "extras-verbs-py"
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
            "cargo", "run", "-q", "--bin", "ciac", "--",
            "build", "examples/extras-verbs.ciac", "--target", "python", "--out", str(PROJECT_DIR),
        ],
        cwd=REPO_ROOT,
    )


def uv_sync() -> None:
    run(["uv", "sync"], cwd=PROJECT_DIR)


def run_inner_proof() -> dict:
    full_env = dict(os.environ)
    full_env["PYTHONPATH"] = os.pathsep.join([str(PYRUNNER_DIR), str(PROJECT_DIR)])
    result = run(
        ["uv", "run", "python", str(PYRUNNER_DIR / "inner_proof_extras_verbs.py")],
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
    print("\n=== M8 remaining-fakes checkpoint (extras-verbs) ===")
    for name, ok in report.items():
        if name == "all_pass":
            continue
        print(f"  [{'PASS' if ok is True else ok}] {name}")

    print(f"\nDECISION: {'PASS' if report['all_pass'] else 'FAIL'}")
    sys.exit(0 if report["all_pass"] else 1)


if __name__ == "__main__":
    main()
