#!/usr/bin/env python3
"""Convert Roder Harbor trial artifacts into an ATIF-compatible ``trajectory.json``.

Mirrors the schema Harbor's built-in Codex agent emits (``ATIF-v1.7``) so Roder
trials light up the same viewer/upload/trace-export consumers. Reads a trial's
``agent/roder-events.jsonl`` (streaming, never loaded whole), ``roder-run-summary.json``,
``roder-last-message.txt``, ``result.json`` and ``verifier/reward.txt``.

Redaction of secrets and truncation of giant outputs are mandatory and counted
into trajectory metadata. Trials whose event log is missing, empty, or
unconvertible get an explicit ``{"unsupported_reason": ...}`` artifact instead.

Usage:
    python3 roder_trajectory_export.py <trial_dir> [--output PATH]
    python3 roder_trajectory_export.py --job-dir <job> --output <root> [--failed-only]
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

from roder_trajectory_features import Redactor, item_type, iter_events

SCHEMA_VERSION = "ATIF-v1.7"
AGENT_NAME = "roder"
EXPORTER_VERSION = "1"


class UnsupportedTrajectory(Exception):
    """Raised when a trial cannot be converted; carries a human-readable reason."""

    def __init__(self, reason: str) -> None:
        super().__init__(reason)
        self.reason = reason


def _read_text(path: Path) -> str:
    if not path.exists():
        return ""
    try:
        return path.read_text(errors="replace")
    except OSError:
        return ""


def _load_json(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError, ValueError):
        return {}
    return value if isinstance(value, dict) else {}


def trial_reward(trial_dir: Path) -> float | None:
    reward_txt = trial_dir / "verifier" / "reward.txt"
    if reward_txt.exists():
        try:
            return float(reward_txt.read_text().strip())
        except (OSError, ValueError):
            pass
    result = _load_json(trial_dir / "result.json")
    verifier = result.get("verifier_result")
    if isinstance(verifier, dict):
        rewards = verifier.get("rewards")
        if isinstance(rewards, dict):
            try:
                return float(rewards.get("reward"))
            except (TypeError, ValueError):
                return None
    return None


def _compact(mapping: dict[str, Any]) -> dict[str, Any]:
    """Drop keys whose value is None, mirroring Codex's ``exclude_none`` dump."""
    return {key: value for key, value in mapping.items() if value is not None}


def build_trajectory(trial_dir: Path) -> dict[str, Any]:
    """Build an ATIF trajectory dict for one trial, or raise ``UnsupportedTrajectory``."""
    events_path = trial_dir / "agent" / "roder-events.jsonl"
    if not events_path.exists() or events_path.stat().st_size == 0:
        raise UnsupportedTrajectory("missing or empty agent/roder-events.jsonl")

    run_summary = _load_json(trial_dir / "agent" / "roder-run-summary.json")
    result = _load_json(trial_dir / "result.json")
    trial_name = str(result.get("trial_name") or trial_dir.name)
    model_name = run_summary.get("model") or None
    reasoning_effort = run_summary.get("reasoning") or None
    provider = run_summary.get("provider") or None

    redactor = Redactor()
    steps: list[dict[str, Any]] = []
    pending_reasoning: list[str] = []
    pending_tools: dict[str, dict[str, Any]] = {}
    thread_id: str | None = None
    turn_ids: list[str] = []
    usages: list[dict[str, Any]] = []
    tool_seq = 0

    def agent_step(message: str) -> dict[str, Any]:
        step: dict[str, Any] = {
            "step_id": len(steps) + 1,
            "source": "agent",
            "message": message,
            "model_name": model_name,
            "reasoning_effort": reasoning_effort,
            "llm_call_count": 1,
        }
        if pending_reasoning:
            step["reasoning_content"] = "\n\n".join(pending_reasoning)
            pending_reasoning.clear()
        return step

    for event in iter_events(events_path):
        etype = event.get("type")
        if etype == "thread.started":
            thread_id = event.get("thread_id") or thread_id
            continue
        if etype == "turn.started":
            turn_id = event.get("turn_id")
            if turn_id:
                turn_ids.append(str(turn_id))
            continue
        if etype in ("turn.completed", "turn.failed"):
            usage = event.get("usage")
            if isinstance(usage, dict):
                usages.append(usage)
            continue
        if etype == "item.started":
            item = event.get("item")
            if item_type(item) == "toolExecution":
                key = str(item.get("id") or item.get("tool_call_id") or "")
                if key:
                    pending_tools[key] = item
            continue
        if etype != "item.completed":
            continue

        item = event.get("item")
        if not isinstance(item, dict):
            continue
        kind = item.get("type")

        if kind == "reasoning":
            text = redactor.text(item.get("text") or "")
            if text:
                pending_reasoning.append(text)
        elif kind == "userMessage":
            steps.append(
                {
                    "step_id": len(steps) + 1,
                    "source": "user",
                    "message": redactor.text(item.get("text") or ""),
                }
            )
        elif kind == "agentMessage":
            text = redactor.text(item.get("text") or "")
            if not text and not pending_reasoning:
                continue
            steps.append(_compact(agent_step(text)))
        elif kind == "error":
            step = agent_step(redactor.text(item.get("text") or ""))
            step["extra"] = _compact({"error": True, "tool_status": item.get("status")})
            steps.append(_compact(step))
        elif kind == "toolExecution":
            steps.append(
                _build_tool_step(
                    item, pending_tools, redactor, agent_step, tool_seq
                )
            )
            tool_seq += 1

    if not steps:
        raise UnsupportedTrajectory("no convertible events in agent/roder-events.jsonl")

    last_message = redactor.text(_read_text(trial_dir / "agent" / "roder-last-message.txt").strip())
    _ensure_final_message(steps, last_message, agent_step)

    reward = trial_reward(trial_dir)
    trajectory = {
        "schema_version": SCHEMA_VERSION,
        "session_id": thread_id or trial_dir.name,
        "trajectory_id": trial_name,
        "agent": _build_agent(run_summary, model_name, provider, reasoning_effort, trial_dir),
        "steps": steps,
    }
    final_metrics = _build_final_metrics(usages, len(steps))
    if final_metrics:
        trajectory["final_metrics"] = final_metrics
    trajectory["extra"] = _compact(
        {
            "exporter": "roder_trajectory_export",
            "exporter_version": EXPORTER_VERSION,
            "source_events": "agent/roder-events.jsonl",
            "redactions": redactor.redactions,
            "truncations": redactor.truncations,
            "reward": reward,
            "elapsed_seconds": run_summary.get("elapsed_seconds"),
            "soft_timed_out": run_summary.get("soft_timed_out"),
            "deadline_finalized": run_summary.get("deadline_finalized"),
            "deadline_timed_out": run_summary.get("deadline_timed_out"),
            "provider_error_kind": run_summary.get("provider_error_kind"),
            "active_tool": run_summary.get("active_tool"),
            "final_message_chars": len(last_message),
            "turn_ids": turn_ids or None,
        }
    )
    return trajectory


def _build_tool_step(
    item: dict[str, Any],
    pending_tools: dict[str, dict[str, Any]],
    redactor: Redactor,
    agent_step: Any,
    tool_seq: int,
) -> dict[str, Any]:
    name = str(item.get("tool_name") or "unknown")
    status = item.get("status")
    call_id = str(item.get("tool_call_id") or item.get("id") or f"call_{tool_seq}")
    started = pending_tools.pop(str(item.get("id") or ""), None)
    if started is None:
        started = pending_tools.pop(call_id, None)
    raw_args = (started or {}).get("payload")
    if not isinstance(raw_args, dict):
        raw_args = item.get("payload") if isinstance(item.get("payload"), dict) else {}
    arguments = redactor.value(raw_args)
    output = redactor.text(item.get("text") or "")

    step = agent_step("")
    step["tool_calls"] = [
        {"tool_call_id": call_id, "function_name": name, "arguments": arguments}
    ]
    result_entry: dict[str, Any] = {"source_call_id": call_id}
    if output:
        result_entry["content"] = output
    step["observation"] = {"results": [result_entry]}
    step["extra"] = _compact({"tool_status": status})
    return _compact(step)


def _ensure_final_message(steps: list[dict[str, Any]], last_message: str, agent_step: Any) -> None:
    if not last_message:
        return
    agent_messages = {s.get("message") for s in steps if s.get("source") == "agent"}
    if last_message in agent_messages:
        return
    steps.append(_compact(agent_step(last_message)))


def _build_agent(
    run_summary: dict[str, Any],
    model_name: str | None,
    provider: str | None,
    reasoning_effort: str | None,
    trial_dir: Path,
) -> dict[str, Any]:
    version = run_summary.get("roder_version")
    if not version:
        version = _read_text(trial_dir / "agent" / "roder-version.txt").strip()
    agent = _compact(
        {
            "name": AGENT_NAME,
            "version": str(version) if version else "unknown",
            "model_name": model_name,
        }
    )
    agent["extra"] = _compact(
        {
            "provider": provider,
            "reasoning_effort": reasoning_effort,
            "policy_mode": run_summary.get("policy_mode"),
        }
    )
    if not agent["extra"]:
        del agent["extra"]
    return agent


def _build_final_metrics(usages: list[dict[str, Any]], total_steps: int) -> dict[str, Any]:
    if not usages:
        return {"total_steps": total_steps}

    def total(key: str) -> int:
        return sum(int(usage.get(key) or 0) for usage in usages)

    metrics = _compact(
        {
            "total_prompt_tokens": total("input_tokens"),
            "total_completion_tokens": total("output_tokens"),
            "total_cached_tokens": total("cached_input_tokens"),
            "total_steps": total_steps,
        }
    )
    reasoning_tokens = total("reasoning_output_tokens")
    if reasoning_tokens:
        metrics["extra"] = {"reasoning_output_tokens": reasoning_tokens}
    return metrics


def export_trial(trial_dir: Path, output_path: Path) -> dict[str, Any]:
    """Write a trajectory (or unsupported-reason) artifact for one trial."""
    output_path.parent.mkdir(parents=True, exist_ok=True)
    trial_name = trial_dir.name
    try:
        trajectory = build_trajectory(trial_dir)
    except UnsupportedTrajectory as exc:
        artifact = {"unsupported_reason": exc.reason, "trial": trial_name}
        output_path.write_text(json.dumps(artifact, indent=2) + "\n")
        return {"status": "unsupported", "trial": trial_name, "reason": exc.reason, "path": str(output_path)}
    output_path.write_text(json.dumps(trajectory, indent=2) + "\n")
    extra = trajectory.get("extra", {})
    return {
        "status": "exported",
        "trial": trial_name,
        "path": str(output_path),
        "steps": len(trajectory["steps"]),
        "redactions": extra.get("redactions", 0),
        "truncations": extra.get("truncations", 0),
    }


def _trial_dirs(job_dir: Path) -> list[Path]:
    return sorted(result.parent for result in job_dir.glob("*/result.json"))


def export_job(job_dir: Path, output_root: Path, failed_only: bool = False) -> dict[str, Any]:
    """Export every trial (or only reward != 1 trials) under a Harbor job dir."""
    results: list[dict[str, Any]] = []
    for trial_dir in _trial_dirs(job_dir):
        if failed_only and trial_reward(trial_dir) == 1.0:
            continue
        output_path = output_root / trial_dir.name / "trajectory.json"
        results.append(export_trial(trial_dir, output_path))
    exported = [r for r in results if r["status"] == "exported"]
    unsupported = [r for r in results if r["status"] == "unsupported"]
    return {
        "job_dir": str(job_dir),
        "output_root": str(output_root),
        "failed_only": failed_only,
        "trials": len(results),
        "exported": len(exported),
        "unsupported": len(unsupported),
        "redactions": sum(r.get("redactions", 0) for r in exported),
        "truncations": sum(r.get("truncations", 0) for r in exported),
        "results": results,
    }


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("trial_dir", type=Path, nargs="?")
    parser.add_argument("--output", type=Path, help="Output path (single trial) or root dir (--job-dir)")
    parser.add_argument("--job-dir", type=Path, help="Export every trial under this Harbor job dir")
    parser.add_argument("--failed-only", action="store_true", help="With --job-dir, only reward != 1 trials")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.job_dir:
        output_root = args.output or args.job_dir
        summary = export_job(args.job_dir, output_root, failed_only=args.failed_only)
        print(json.dumps({k: v for k, v in summary.items() if k != "results"}, indent=2))
        return 0
    if not args.trial_dir:
        print("roder_trajectory_export: trial_dir or --job-dir required", file=sys.stderr)
        return 2
    output_path = args.output or (args.trial_dir / "agent" / "trajectory.json")
    result = export_trial(args.trial_dir, output_path)
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
