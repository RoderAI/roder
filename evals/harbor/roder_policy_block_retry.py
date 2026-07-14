"""Bounded fresh-thread retries for provider policy blocks with no tool progress.

Provider safety filters occasionally flag a benchmark task's instruction
before the agent has done any work; the identical prompt usually passes on a
fresh thread. Retrying such zero-progress blocks is evaluation-neutral: the
agent gains no task information, and the retry runs inside the same
unmodified task window.
"""

from __future__ import annotations

import shlex

POLICY_BLOCK_MARKER = "flagged for possible cybersecurity risk"
PER_RETRY_NOTE_PREFIX = "policy-block retry"
POLICY_BLOCK_RETRY_MIN_BUDGET_SEC = 240
POLICY_BLOCK_RETRY_SLACK_SEC = 15


def policy_block_check_command(*, events_path: str, stderr_path: str) -> str:
    """Shell command exiting 0 iff the run was policy-blocked with no tool calls."""
    check = (
        f"grep -q {shlex.quote(POLICY_BLOCK_MARKER)} {shlex.quote(stderr_path)} && "
        f"! grep -q '\"tool_name\"' {shlex.quote(events_path)}"
    )
    return f"bash -lc {shlex.quote(check)}"


def policy_block_retry_budget_sec(
    *,
    soft_timeout_sec: int | None,
    elapsed_sec: float,
) -> int | None:
    """Remaining soft budget for one retry, or None when a retry cannot fit."""
    if soft_timeout_sec is None:
        return None
    remaining = int(soft_timeout_sec - elapsed_sec) - POLICY_BLOCK_RETRY_SLACK_SEC
    if remaining < POLICY_BLOCK_RETRY_MIN_BUDGET_SEC:
        return None
    return remaining
