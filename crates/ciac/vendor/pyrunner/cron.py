"""A narrow Python restatement of `ciac_sim::cron::CronSchedule` (Rust,
v0.17 M4), same disclosed status as `world.py`'s `FailureEngine`/
`VirtualClock`: the 5-field grammar `ciac-sema` already validates at
compile time, ported so `scenario_runner.py` can fire cron jobs while
advancing virtual time without shelling out to Rust per tick.
`ciac-sim` remains the canonical, tested implementation.
"""

from __future__ import annotations

import re
from datetime import datetime, timedelta, timezone

MAX_LOOKAHEAD_MINUTES = 5 * 366 * 24 * 60


class CronError(Exception):
    pass


def _parse_field(field: str, lo: int, hi: int) -> set[int]:
    out: set[int] = set()
    for part in field.split(","):
        base, _, step_s = part.partition("/")
        step = int(step_s) if step_s else 1
        if step == 0:
            raise CronError(f"step cannot be zero in `{part}`")
        if base == "*":
            start, end = lo, hi
        elif "-" in base:
            start_s, end_s = base.split("-", 1)
            start, end = int(start_s), int(end_s)
            if not (lo <= start <= end <= hi):
                raise CronError(f"range `{part}` out of bounds {lo}-{hi}")
        else:
            start = end = int(base)
            if not (lo <= start <= hi):
                raise CronError(f"value `{start}` out of bounds {lo}-{hi}")
        out.update(range(start, end + 1, step))
    return out


class CronSchedule:
    def __init__(
        self,
        minutes: set[int],
        hours: set[int],
        days: set[int],
        months: set[int],
        weekdays: set[int],
    ) -> None:
        self.minutes = minutes
        self.hours = hours
        self.days = days
        self.months = months
        self.weekdays = weekdays

    @classmethod
    def parse(cls, expr: str) -> "CronSchedule":
        fields = expr.split()
        if len(fields) != 5:
            raise CronError(f"expected 5 whitespace-separated fields, found {len(fields)}")
        minutes = _parse_field(fields[0], 0, 59)
        hours = _parse_field(fields[1], 0, 23)
        days = _parse_field(fields[2], 1, 31)
        months = _parse_field(fields[3], 1, 12)
        raw_weekdays = _parse_field(fields[4], 0, 7)
        weekdays = {d % 7 for d in raw_weekdays}
        return cls(minutes, hours, days, months, weekdays)

    def matches(self, dt: datetime) -> bool:
        sunday0 = (dt.weekday() + 1) % 7  # Python: Monday=0; ours: Sunday=0
        return (
            dt.minute in self.minutes
            and dt.hour in self.hours
            and dt.day in self.days
            and dt.month in self.months
            and sunday0 in self.weekdays
        )

    def next_fire_after(self, after_ms: int) -> int | None:
        after = datetime.fromtimestamp(after_ms / 1000, tz=timezone.utc)
        candidate = after.replace(second=0, microsecond=0) + timedelta(minutes=1)
        for _ in range(MAX_LOOKAHEAD_MINUTES):
            if self.matches(candidate):
                return int(candidate.timestamp() * 1000)
            candidate += timedelta(minutes=1)
        return None

    def due_instants(self, from_ms: int, to_ms: int, cap: int = 10_000) -> list[int]:
        out: list[int] = []
        cursor = from_ms
        while len(out) < cap:
            fire = self.next_fire_after(cursor)
            if fire is not None and fire <= to_ms:
                out.append(fire)
                cursor = fire
            else:
                break
        return out


_DURATION_RE = re.compile(r"^(\d+)(ms|s|m|h|d)$")
_DURATION_UNIT_MS = {"ms": 1, "s": 1_000, "m": 60_000, "h": 3_600_000, "d": 86_400_000}


def parse_duration_ms(text: str) -> int:
    """Parses `advance.by` strings (`"24h"`, `"7d"`, `"30m"`, ...) --
    the same small duration grammar the plan's own worked examples use."""
    match = _DURATION_RE.match(text)
    if not match:
        raise ValueError(f"unrecognized duration: {text!r}")
    value, unit = match.groups()
    return int(value) * _DURATION_UNIT_MS[unit]
