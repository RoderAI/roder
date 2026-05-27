"""Shared Terminal-Bench deadline policy for local Harbor eval configs."""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any


@dataclass(frozen=True)
class TBenchDeadlinePolicy:
    override_timeout_sec: int
    soft_timeout_sec: int
    eval_deadline_seconds: int


TBENCH_DEADLINE_POLICY = TBenchDeadlinePolicy(
    override_timeout_sec=1800,
    soft_timeout_sec=1780,
    eval_deadline_seconds=1740,
)


def deadline_policy_summary() -> dict[str, int]:
    return {
        "overrideTimeoutSec": TBENCH_DEADLINE_POLICY.override_timeout_sec,
        "softTimeoutSec": TBENCH_DEADLINE_POLICY.soft_timeout_sec,
        "evalDeadlineSeconds": TBENCH_DEADLINE_POLICY.eval_deadline_seconds,
    }


def validate_deadline_policy(
    issues: list[str],
    value: Any,
    *,
    issue_prefix: str,
) -> None:
    if not isinstance(value, dict):
        issues.append(f"{issue_prefix} is missing")
        return
    expected = (
        ("overrideTimeoutSec", TBENCH_DEADLINE_POLICY.override_timeout_sec),
        ("softTimeoutSec", TBENCH_DEADLINE_POLICY.soft_timeout_sec),
        ("evalDeadlineSeconds", TBENCH_DEADLINE_POLICY.eval_deadline_seconds),
    )
    for field, expected_value in expected:
        actual = optional_int_value(value.get(field))
        if actual is None:
            issues.append(f"{issue_prefix} {field} is missing")
        elif actual != expected_value:
            issues.append(f"{issue_prefix} {field} is {actual}, expected {expected_value}")


def optional_int_value(value: Any) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None
