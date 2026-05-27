"""Summarize Harbor config invariants for pre-eval handoff reports."""

from __future__ import annotations

import json
import hashlib
from pathlib import Path
from typing import Any

from tbench_deadline_policy import deadline_policy_summary
from validate_harbor_readiness import validate_config as validate_readiness_config


DEFAULT_CONFIGS = (
    Path("evals/harbor/tbench-full-gpt55-medium.json"),
    Path("evals/harbor/tbench-smoke.json"),
)


def harbor_config_summary(paths: tuple[Path, ...] | list[Path] | None = None) -> dict[str, Any]:
    entries: list[dict[str, Any]] = []
    issues: list[str] = []
    for path in paths or DEFAULT_CONFIGS:
        entry = summarize_config(path)
        entries.append(entry)
        issues.extend(config_issues(entry))
    return {
        "status": "failed" if issues else "passed",
        "configs": len(entries),
        "issues": issues,
        "deadlinePolicy": deadline_policy_summary(),
        "entries": entries,
    }


def summarize_config(path: Path) -> dict[str, Any]:
    base: dict[str, Any] = {"path": str(path)}
    try:
        raw = path.read_bytes()
        config = json.loads(raw)
    except Exception as exc:
        return {**base, "error": str(exc)}
    agent = first_agent(config)
    kwargs = agent.get("kwargs") if isinstance(agent.get("kwargs"), dict) else {}
    readiness_issues = validate_readiness_config(path, config)
    return {
        **base,
        "sha256": hashlib.sha256(raw).hexdigest(),
        "jobName": config.get("job_name"),
        "modelName": agent.get("model_name"),
        "reasoning": kwargs.get("reasoning"),
        "readinessIssues": readiness_issues,
        "nConcurrentTrials": nested(config, "orchestrator", "n_concurrent_trials"),
        "environmentDelete": nested(config, "environment", "delete"),
        "overrideTimeoutSec": agent.get("override_timeout_sec"),
        "softTimeoutSec": kwargs.get("soft_timeout_sec"),
        "evalDeadlineSeconds": kwargs.get("speed_policy_eval_deadline_seconds"),
        "speedPolicyEnabled": kwargs.get("speed_policy_enabled"),
        "taskLedgerRequired": kwargs.get("task_ledger_required"),
        "includePrebuiltBinary": bool_value(kwargs.get("include_prebuilt_binary")),
        "includeLocalSource": bool_value(kwargs.get("include_local_source")),
    }


def config_issues(entry: dict[str, Any]) -> list[str]:
    path = entry.get("path")
    if "error" in entry:
        return [f"{path}: failed to load config: {entry['error']}"]
    readiness_issues = entry.get("readinessIssues")
    if not isinstance(readiness_issues, list):
        return [f"{path}: readiness issues must be a list"]
    return [str(issue) for issue in readiness_issues]


def first_agent(config: dict[str, Any]) -> dict[str, Any]:
    agents = config.get("agents")
    if not isinstance(agents, list) or not agents or not isinstance(agents[0], dict):
        return {}
    return agents[0]


def nested(value: dict[str, Any], *keys: str) -> Any:
    current: Any = value
    for key in keys:
        if not isinstance(current, dict):
            return None
        current = current.get(key)
    return current


def bool_value(value: Any) -> bool | None:
    if isinstance(value, bool):
        return value
    if value is None:
        return None
    text = str(value).strip().lower()
    if text in {"1", "true", "yes", "on"}:
        return True
    if text in {"0", "false", "no", "off"}:
        return False
    return None
