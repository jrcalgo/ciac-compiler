#!/usr/bin/env python3
"""v0.17 M7's live proof against the real generated `sim-broker-slice`
project: ordering, independent queue groups (fan-out), duplicate
delivery via a lost ack, and cache TTL against virtual time -- no
mocks, no Docker, no wall-clock sleep.

Run: `python3 sim/pyrunner/run_broker_slice.py`
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
PROJECT_DIR = REPO_ROOT / "target" / "sim-m7-checkpoint" / "sim-broker-slice-py"
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
            "examples/sim-broker-slice.ciac",
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
        ["uv", "run", "python", str(PYRUNNER_DIR / "inner_proof_broker_slice.py")],
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

    print("\n=== M7 broker/cache checkpoint ===")
    for name, ok in report.items():
        if name == "all_pass":
            continue
        marker = "PASS" if ok is True else str(ok)
        print(f"  [{marker}] {name}")

    print(f"\nDECISION: {'PASS' if report['all_pass'] else 'FAIL'}")
    sys.exit(0 if report["all_pass"] else 1)


if __name__ == "__main__":
    main()
