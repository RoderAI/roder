#!/usr/bin/env python3
"""Compare two analyzed Harbor Terminal-Bench runs."""

from __future__ import annotations

import argparse
import json
import sys
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from analyze_tbench_run import analyze_job


@dataclass
class TaskState:
    task_name: str
    trial_name: str | None
    reward: float | None
    classes: set[str]
    roder_exit_status: int | None
    missing_artifacts: list[str]


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def load_analysis(path: Path) -> dict[str, Any]:
    if path.is_dir():
        return analyze_job(path, group_scored_failures=True)
    data = load_json(path)
    if "classes" in data and "stats" in data:
        return data
    raise ValueError(f"{path} is neither a Harbor job dir nor analyzer JSON")


def class_counts(analysis: dict[str, Any]) -> dict[str, int]:
    classes = analysis.get("classes")
    if not isinstance(classes, dict):
        return {}
    return {str(name): len(entries) for name, entries in sorted(classes.items())}


def mean_score(analysis: dict[str, Any]) -> float | None:
    harbor = analysis.get("stats", {}).get("harbor")
    if not isinstance(harbor, dict):
        return None
    evals = harbor.get("evals")
    if not isinstance(evals, dict):
        return None
    for value in evals.values():
        if not isinstance(value, dict):
            continue
        metrics = value.get("metrics")
        if not isinstance(metrics, list) or not metrics:
            continue
        metric = metrics[0]
        if not isinstance(metric, dict):
            continue
        try:
            return float(metric["mean"])
        except (KeyError, TypeError, ValueError):
            continue
    return None


def task_name(entry: dict[str, Any]) -> str:
    if entry.get("task_name"):
        return str(entry["task_name"])
    if entry.get("task"):
        return str(entry["task"])
    trial_name = str(entry.get("trial_name") or "")
    return trial_name.split("__", 1)[0]


def infer_reward(class_name: str, entry: dict[str, Any]) -> float | None:
    if "reward" in entry:
        try:
            return float(entry["reward"])
        except (TypeError, ValueError):
            pass
    if class_name == "pass":
        return 1.0
    if class_name == "scored_fail":
        return 0.0
    return None


def task_states(analysis: dict[str, Any]) -> dict[str, TaskState]:
    states: dict[str, TaskState] = {}
    classes = analysis.get("classes")
    if not isinstance(classes, dict):
        return states
    for class_name, entries in classes.items():
        if not isinstance(entries, list):
            continue
        for entry in entries:
            if not isinstance(entry, dict):
                continue
            name = task_name(entry)
            state = states.setdefault(
                name,
                TaskState(
                    task_name=name,
                    trial_name=None,
                    reward=None,
                    classes=set(),
                    roder_exit_status=None,
                    missing_artifacts=[],
                ),
            )
            state.classes.add(str(class_name))
            if entry.get("trial_name"):
                state.trial_name = str(entry["trial_name"])
            reward = infer_reward(str(class_name), entry)
            if reward is not None:
                state.reward = reward
            if entry.get("roder_exit_status") is not None:
                try:
                    state.roder_exit_status = int(entry["roder_exit_status"])
                except (TypeError, ValueError):
                    pass
            missing = entry.get("missing_artifacts")
            if isinstance(missing, list):
                state.missing_artifacts = sorted({str(value) for value in missing})
    return states


def summarize(analysis: dict[str, Any]) -> dict[str, Any]:
    stats = analysis.get("stats", {})
    harbor = stats.get("harbor") if isinstance(stats, dict) else {}
    return {
        "job_name": analysis.get("job_name"),
        "job_dir": analysis.get("job_dir"),
        "clean": analysis.get("clean"),
        "n_trials": harbor.get("n_trials") if isinstance(harbor, dict) else None,
        "n_errors": harbor.get("n_errors") if isinstance(harbor, dict) else None,
        "mean": mean_score(analysis),
        "passes": stats.get("passes") if isinstance(stats, dict) else None,
        "scored_failures": stats.get("scored_failures") if isinstance(stats, dict) else None,
        "class_counts": class_counts(analysis),
    }


def state_entry(state: TaskState | None) -> dict[str, Any] | None:
    if state is None:
        return None
    entry: dict[str, Any] = {
        "trial_name": state.trial_name,
        "reward": state.reward,
        "classes": sorted(state.classes),
    }
    if state.roder_exit_status is not None:
        entry["roder_exit_status"] = state.roder_exit_status
    if state.missing_artifacts:
        entry["missing_artifacts"] = state.missing_artifacts
    return entry


def task_delta(task: str, baseline: TaskState | None, current: TaskState | None) -> dict[str, Any]:
    return {
        "task_name": task,
        "baseline": state_entry(baseline),
        "current": state_entry(current),
    }


def compare_analyses(
    baseline: dict[str, Any],
    current: dict[str, Any],
    baseline_path: Path,
    current_path: Path,
) -> dict[str, Any]:
    baseline_tasks = task_states(baseline)
    current_tasks = task_states(current)
    all_tasks = sorted(set(baseline_tasks) | set(current_tasks))

    improved = []
    regressed = []
    reward_unchanged_class_changed = []
    missing_in_baseline = []
    missing_in_current = []

    for task in all_tasks:
        before = baseline_tasks.get(task)
        after = current_tasks.get(task)
        if before is None:
            missing_in_baseline.append(task_delta(task, before, after))
            continue
        if after is None:
            missing_in_current.append(task_delta(task, before, after))
            continue
        if before.reward == 0.0 and after.reward == 1.0:
            improved.append(task_delta(task, before, after))
        elif before.reward == 1.0 and after.reward == 0.0:
            regressed.append(task_delta(task, before, after))
        elif before.classes != after.classes:
            reward_unchanged_class_changed.append(task_delta(task, before, after))

    baseline_counts = class_counts(baseline)
    current_counts = class_counts(current)
    class_count_delta = {
        name: {
            "baseline": baseline_counts.get(name, 0),
            "current": current_counts.get(name, 0),
            "delta": current_counts.get(name, 0) - baseline_counts.get(name, 0),
        }
        for name in sorted(set(baseline_counts) | set(current_counts))
    }

    baseline_summary = summarize(baseline)
    current_summary = summarize(current)
    pass_delta = None
    if baseline_summary["passes"] is not None and current_summary["passes"] is not None:
        pass_delta = current_summary["passes"] - baseline_summary["passes"]
    mean_delta = None
    if baseline_summary["mean"] is not None and current_summary["mean"] is not None:
        mean_delta = current_summary["mean"] - baseline_summary["mean"]

    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "baseline_path": str(baseline_path),
        "current_path": str(current_path),
        "baseline": baseline_summary,
        "current": current_summary,
        "delta": {
            "passes": pass_delta,
            "mean": mean_delta,
            "improved_count": len(improved),
            "regressed_count": len(regressed),
            "reward_unchanged_class_changed_count": len(reward_unchanged_class_changed),
            "missing_in_baseline_count": len(missing_in_baseline),
            "missing_in_current_count": len(missing_in_current),
            "class_counts": class_count_delta,
        },
        "improved": improved,
        "regressed": regressed,
        "reward_unchanged_class_changed": reward_unchanged_class_changed,
        "missing_in_baseline": missing_in_baseline,
        "missing_in_current": missing_in_current,
    }


def format_reward(value: float | None) -> str:
    if value is None:
        return "?"
    return f"{value:g}"


def class_string(state: dict[str, Any] | None) -> str:
    if state is None:
        return "missing"
    classes = state.get("classes") or []
    return ", ".join(classes) if classes else "none"


def render_task_delta(entry: dict[str, Any]) -> str:
    baseline = entry.get("baseline")
    current = entry.get("current")
    left = f"{format_reward((baseline or {}).get('reward'))} [{class_string(baseline)}]"
    right = f"{format_reward((current or {}).get('reward'))} [{class_string(current)}]"
    return f"- `{entry['task_name']}`: {left} -> {right}"


def render_markdown(report: dict[str, Any]) -> str:
    baseline = report["baseline"]
    current = report["current"]
    delta = report["delta"]
    lines = [
        f"# Harbor TBench Comparison: {baseline['job_name']} -> {current['job_name']}",
        "",
        f"- Baseline: `{report['baseline_path']}`",
        f"- Current: `{report['current_path']}`",
        f"- Baseline score: {baseline['passes']}/{baseline['n_trials']} mean `{baseline['mean']}`",
        f"- Current score: {current['passes']}/{current['n_trials']} mean `{current['mean']}`",
        f"- Pass delta: `{delta['passes']}`",
        f"- Mean delta: `{delta['mean']}`",
        f"- Improved tasks: {delta['improved_count']}",
        f"- Regressed tasks: {delta['regressed_count']}",
        f"- Class-only changes: {delta['reward_unchanged_class_changed_count']}",
        "",
        "## Class Count Delta",
        "",
        "| Class | Baseline | Current | Delta |",
        "| --- | ---: | ---: | ---: |",
    ]
    for name, values in delta["class_counts"].items():
        lines.append(
            f"| `{name}` | {values['baseline']} | {values['current']} | {values['delta']} |"
        )

    sections = (
        ("Improved", report["improved"]),
        ("Regressed", report["regressed"]),
        ("Reward-Unchanged Class Changes", report["reward_unchanged_class_changed"]),
        ("Missing In Baseline", report["missing_in_baseline"]),
        ("Missing In Current", report["missing_in_current"]),
    )
    for title, entries in sections:
        lines.extend(["", f"## {title}", ""])
        if not entries:
            lines.append("- None")
            continue
        for entry in entries:
            lines.append(render_task_delta(entry))
    return "\n".join(lines).rstrip() + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("baseline", type=Path, help="Baseline analyzer JSON or Harbor job dir")
    parser.add_argument("current", type=Path, help="Current analyzer JSON or Harbor job dir")
    parser.add_argument("--json", dest="json_path", type=Path)
    parser.add_argument("--markdown", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        baseline = load_analysis(args.baseline)
        current = load_analysis(args.current)
        report = compare_analyses(baseline, current, args.baseline, args.current)
    except Exception as exc:
        print(f"compare_tbench_runs: {exc}", file=sys.stderr)
        return 2

    if args.json_path:
        args.json_path.parent.mkdir(parents=True, exist_ok=True)
        args.json_path.write_text(json.dumps(report, indent=2) + "\n")

    markdown = render_markdown(report)
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        args.markdown.write_text(markdown)

    if not args.json_path and not args.markdown:
        print(markdown, end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
