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

# Per-task ladder margins: Harbor hard-kills the exec at the task's own
# timeout, so the adapter SIGINTs roder one margin earlier (time for the
# run-script tail to write summaries) and roder's internal turn deadline sits
# one more margin below that (time to stream a graceful final answer).
PER_TASK_SOFT_TIMEOUT_MARGIN_SEC = 60
PER_TASK_EVAL_DEADLINE_MARGIN_SEC = 60
MIN_PER_TASK_SOFT_TIMEOUT_SEC = 120
MIN_PER_TASK_EVAL_DEADLINE_SEC = 90


@dataclass(frozen=True)
class TaskDeadlineLadder:
    task_timeout_sec: int
    hard_timeout_sec: int
    soft_timeout_sec: int
    eval_deadline_seconds: int


def derive_task_deadline_ladder(
    task_timeout_sec: float | int | None,
    *,
    agent_timeout_multiplier: float | None = None,
) -> TaskDeadlineLadder | None:
    """Derive the adapter's soft timeout and roder's turn deadline for one task.

    ``task_timeout_sec`` is the unmodified `task.toml [agent] timeout_sec`;
    ``agent_timeout_multiplier`` must mirror the Harbor job config (1.0 for
    leaderboard-valid runs).
    """
    if not task_timeout_sec or task_timeout_sec <= 0:
        return None
    multiplier = agent_timeout_multiplier if agent_timeout_multiplier else 1.0
    hard = int(task_timeout_sec * multiplier)
    soft = max(hard - PER_TASK_SOFT_TIMEOUT_MARGIN_SEC, MIN_PER_TASK_SOFT_TIMEOUT_SEC)
    if soft >= hard:
        soft = max(hard - 10, 1)
    eval_deadline = max(
        soft - PER_TASK_EVAL_DEADLINE_MARGIN_SEC, MIN_PER_TASK_EVAL_DEADLINE_SEC
    )
    if eval_deadline >= soft:
        eval_deadline = max(soft - 5, 1)
    return TaskDeadlineLadder(
        task_timeout_sec=int(task_timeout_sec),
        hard_timeout_sec=hard,
        soft_timeout_sec=soft,
        eval_deadline_seconds=eval_deadline,
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
