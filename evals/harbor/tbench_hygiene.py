#!/usr/bin/env python3
"""Evaluation-neutral completion-hygiene labels and trajectory features.

Computed strictly post-run from Roder artifacts (event log, run summary, final
message). Nothing here inspects hidden verifier intent or scoreable artifacts —
labels describe observable trajectory behaviour only, so they are safe to attach
to every trial regardless of reward.

Consumed by ``analyze_tbench_run.py`` to surface per-trial ``features``/``hygiene``
and to rank failed trials by dominant actionable cause.
"""

from __future__ import annotations

from typing import Any

from roder_trajectory_features import TrajectoryFeatures, scan_features
from tbench_trial import Trial, read_text

# Phrases that mark a final message as provisional / deadline-truncated.
PROVISIONAL_MARKERS: tuple[str, ...] = (
    "provisional",
    "before the deadline",
    "before deadline",
    "before time ran out",
    "not completed before",
    "not complete before",
    "not fully validated",
    "ran out of time",
    "still running",
    "build is still",
    "build still",
    "was still running",
    "did not finish",
    "could not finish",
    "incomplete before",
    "partial answer",
    "best effort before",
)

# A successful tool within this many trailing tool calls counts as "acting near the end".
_FINAL_ANSWER_WINDOW = 3

HYGIENE_LABELS: tuple[str, ...] = (
    "final_answer_only",
    "provisional_final_message",
    "long_command_still_running",
    "failed_last_tool",
    "no_local_validation",
)


def _last_message(trial: Trial) -> str:
    return read_text(trial.path / "agent" / "roder-last-message.txt").strip()


def _events_path(trial: Trial):
    return trial.path / "agent" / "roder-events.jsonl"


def scan_trial_features(trial: Trial) -> TrajectoryFeatures:
    events = _events_path(trial)
    if events.exists() and events.stat().st_size > 0:
        return scan_features(events)
    return TrajectoryFeatures()


def _compact_tool(tool: Any) -> dict[str, Any] | None:
    if not isinstance(tool, dict):
        return None
    compact = {
        key: tool.get(key)
        for key in ("tool_name", "status", "item_type")
        if tool.get(key) is not None
    }
    return compact or None


def feature_summary(trial: Trial, features: TrajectoryFeatures) -> dict[str, Any]:
    """Compact per-trial trajectory feature row for the analysis JSON."""
    last_message = _last_message(trial)
    summary = features.as_dict()
    summary["final_message_chars"] = len(last_message)

    run_summary = trial.run_summary
    for key in ("soft_timed_out", "deadline_timed_out", "deadline_finalized"):
        value = run_summary.get(key)
        if value is not None:
            summary[key] = value
    active = _compact_tool(run_summary.get("active_tool"))
    if active:
        summary["active_tool"] = active
    last_tool = _compact_tool(run_summary.get("last_tool"))
    if last_tool:
        summary["last_tool"] = last_tool

    if not features.last_successful_action and last_message:
        summary["last_successful_action"] = f"final message ({len(last_message)} chars)"
    return summary


def hygiene_labels(trial: Trial, features: TrajectoryFeatures) -> list[str]:
    """Post-run completion-hygiene labels for one trial (evaluation-neutral)."""
    labels: list[str] = []
    run_summary = trial.run_summary
    last_message_lower = _last_message(trial).lower()
    last_tool = run_summary.get("last_tool") if isinstance(run_summary.get("last_tool"), dict) else {}
    last_tool_status = last_tool.get("status")

    if any(marker in last_message_lower for marker in PROVISIONAL_MARKERS):
        labels.append("provisional_final_message")

    if run_summary.get("active_tool") or last_tool_status == "running":
        labels.append("long_command_still_running")

    if last_tool_status == "failed" or features.last_tool_status == "failed":
        labels.append("failed_last_tool")

    no_success_near_end = (
        features.last_successful_action is None
        or (
            features.tools_since_last_success is not None
            and features.tools_since_last_success >= _FINAL_ANSWER_WINDOW
        )
    )
    if no_success_near_end:
        labels.append("final_answer_only")

    if not features.has_validation_after_last_write:
        labels.append("no_local_validation")

    return sorted(labels)


def build_hygiene_record(trial: Trial, classes: list[str]) -> dict[str, Any]:
    """Compute features + hygiene once per trial; reused by task rows and rankings."""
    features = scan_trial_features(trial)
    summary = feature_summary(trial, features)
    labels = hygiene_labels(trial, features)
    return {
        "trial_name": trial.name,
        "task_name": trial.task_name,
        "reward": trial.reward,
        "classes": classes,
        "features": summary,
        "hygiene": labels,
        "elapsed_seconds": trial.run_summary.get("elapsed_seconds"),
        "provider_error_kind": trial.run_summary.get("provider_error_kind"),
        "failed_tools": features.failed_tools,
        "verification_reviews": features.verification_reviews,
    }


def hygiene_summary(records: list[dict[str, Any]]) -> dict[str, Any]:
    """Counts and task lists for each hygiene label (failed trials only)."""
    summary: dict[str, Any] = {}
    for label in HYGIENE_LABELS:
        tasks = sorted(
            {rec["task_name"] for rec in records if rec["reward"] == 0.0 and label in rec["hygiene"]}
        )
        summary[label] = {"count": len(tasks), "tasks": tasks}
    return summary


def _rank_entry(record: dict[str, Any], **extra: Any) -> dict[str, Any]:
    entry = {"trial_name": record["trial_name"], "task_name": record["task_name"]}
    entry.update({key: value for key, value in extra.items() if value is not None})
    return entry


def rank_failed_trials(records: list[dict[str, Any]]) -> dict[str, list[dict[str, Any]]]:
    """Rank reward-0 trials by dominant actionable cause (PRD Stage 1 acceptance)."""
    failed = [rec for rec in records if rec["reward"] == 0.0]

    missing_verification = [
        _rank_entry(rec, verification_reviews=rec["verification_reviews"])
        for rec in failed
        if "no_local_validation" in rec["hygiene"]
    ]
    missing_verification.sort(key=lambda e: (e["task_name"]))

    deadline = [
        _rank_entry(rec, elapsed_seconds=rec["elapsed_seconds"])
        for rec in failed
        if rec["features"].get("deadline_finalized")
        or rec["features"].get("soft_timed_out")
        or rec["features"].get("deadline_timed_out")
        or rec["provider_error_kind"] == "turn_deadline_expired"
    ]
    deadline.sort(key=lambda e: (e.get("elapsed_seconds") or 0), reverse=True)

    failed_last_tool = [
        _rank_entry(rec, failed_tools=rec["failed_tools"])
        for rec in failed
        if "failed_last_tool" in rec["hygiene"]
    ]
    failed_last_tool.sort(key=lambda e: (e.get("failed_tools") or 0), reverse=True)

    policy_block = [
        _rank_entry(rec, provider_error_kind=rec["provider_error_kind"])
        for rec in failed
        if "provider_policy_block" in rec["classes"] or rec["provider_error_kind"] == "policy_block"
    ]
    policy_block.sort(key=lambda e: e["task_name"])

    environment_service = [
        _rank_entry(rec, failed_tools=rec["failed_tools"])
        for rec in failed
        if rec["failed_tools"] > 0
    ]
    environment_service.sort(key=lambda e: (e.get("failed_tools") or 0), reverse=True)

    return {
        "missing_verification": missing_verification,
        "deadline_burn": deadline,
        "failed_last_tool": failed_last_tool,
        "policy_block": policy_block,
        "environment_service": environment_service,
    }
