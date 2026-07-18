"""Direct tests of `cron.py`'s Python restatement of
`ciac_sim::cron::CronSchedule`, mirroring that crate's own test names
and fixtures so the two stay comparable.

Run: `python3 sim/pyrunner/test_cron.py`
"""

from __future__ import annotations

import sys
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from cron import CronSchedule, parse_duration_ms


def dt_ms(y: int, mo: int, d: int, h: int, mi: int) -> int:
    return int(datetime(y, mo, d, h, mi, tzinfo=timezone.utc).timestamp() * 1000)


def test_daily_three_am_fires_once_per_day() -> None:
    schedule = CronSchedule.parse("0 3 * * *")
    start = dt_ms(2030, 1, 1, 0, 0)
    first = schedule.next_fire_after(start)
    assert first == dt_ms(2030, 1, 1, 3, 0)
    second = schedule.next_fire_after(first)
    assert second == dt_ms(2030, 1, 2, 3, 0)


def test_advance_24_hours_observes_the_0300_job_exactly_once() -> None:
    schedule = CronSchedule.parse("0 3 * * *")
    start = dt_ms(2030, 1, 1, 0, 0)
    end = start + 24 * 60 * 60 * 1000
    due = schedule.due_instants(start, end)
    assert due == [dt_ms(2030, 1, 1, 3, 0)]


def test_catch_up_returns_every_missed_instant_oldest_first() -> None:
    schedule = CronSchedule.parse("0 * * * *")
    start = dt_ms(2030, 1, 1, 0, 30)
    end = dt_ms(2030, 1, 1, 5, 30)
    due = schedule.due_instants(start, end)
    assert len(due) == 5
    assert due == sorted(due)
    assert due[0] == dt_ms(2030, 1, 1, 1, 0)
    assert due[-1] == dt_ms(2030, 1, 1, 5, 0)


def test_weekday_seven_and_zero_both_mean_sunday() -> None:
    zero = CronSchedule.parse("0 0 * * 0")
    seven = CronSchedule.parse("0 0 * * 7")
    sunday_midnight = dt_ms(2030, 1, 6, 0, 0)  # 2030-01-06 is a Sunday
    start = sunday_midnight - 1
    assert zero.next_fire_after(start) == sunday_midnight
    assert seven.next_fire_after(start) == sunday_midnight


def test_impossible_schedule_bounds_out() -> None:
    schedule = CronSchedule.parse("0 0 31 2 *")  # February never has a 31st
    start = dt_ms(2030, 1, 1, 0, 0)
    assert schedule.next_fire_after(start) is None


def test_parse_duration_ms() -> None:
    assert parse_duration_ms("24h") == 24 * 60 * 60 * 1000
    assert parse_duration_ms("7d") == 7 * 24 * 60 * 60 * 1000
    assert parse_duration_ms("30m") == 30 * 60 * 1000
    assert parse_duration_ms("500ms") == 500


def main() -> None:
    tests = [v for k, v in globals().items() if k.startswith("test_") and callable(v)]
    for test in tests:
        test()
        print(f"  ok  {test.__name__}")
    print(f"{len(tests)} tests passed")


if __name__ == "__main__":
    main()
