#!/usr/bin/env python3
"""Analyze a Harbor Terminal-Bench job produced by the Roder harness.

Trial loading lives in ``tbench_trial``, class assignment in ``tbench_classify``,
and evaluation-neutral completion-hygiene labels + trajectory features in
``tbench_hygiene``. This module orchestrates them into the analysis JSON /
Markdown report and the rerun manifests.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import platform
import shutil
import subprocess
import sys
from collections import defaultdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from tbench_analysis_constants import (
    HARNESS_ERROR_CLASSES,
    RUN_SUMMARY_TASK_FIELDS,
    SCORED_GROUP_PATTERNS,
    SCORED_GROUP_SUBSYSTEMS,
)
from tbench_classify import classify_trial
from tbench_hygiene import (
    HYGIENE_LABELS,
    build_hygiene_record,
    hygiene_summary,
    rank_failed_trials,
)
from tbench_trial import Trial, load_json, load_trials


def task_entry(trial: Trial, hygiene: dict[str, Any] | None = None) -> dict[str, Any]:
    entry = {
        "trial_name": trial.name,
        "task_name": trial.task_name,
        "path": str(trial.path),
    }
    if trial.reward is not None:
        entry["reward"] = trial.reward
    if trial.exception_type:
        entry["exception_type"] = trial.exception_type
    exit_status = trial.roder_exit_status()
    if exit_status is not None:
        entry["roder_exit_status"] = exit_status
    if trial.run_summary:
        entry["run_summary_path"] = str(trial.path / "agent" / "roder-run-summary.json")
        for key in RUN_SUMMARY_TASK_FIELDS:
            if key in trial.run_summary:
                entry[key] = trial.run_summary[key]
    missing = trial.missing_expected_artifacts()
    if missing:
        entry["missing_artifacts"] = missing
    artifact_sizes = {
        name: trial.agent_artifact_size(name)
        for name in (
            "roder-events.jsonl",
            "roder-run-summary.json",
            "roder-last-message.txt",
            "roder-stderr.txt",
            "setup-summary.txt",
        )
    }
    entry["agent_artifact_sizes"] = {
        name: size for name, size in artifact_sizes.items() if size is not None
    }
    setup_tail = "\n".join(trial.setup_text.splitlines()[-12:])
    if setup_tail:
        entry["setup_tail"] = setup_tail
    exception_tail = "\n".join(trial.exception_text.splitlines()[-12:])
    if exception_tail:
        entry["exception_tail"] = exception_tail
    if hygiene is not None:
        entry["features"] = hygiene["features"]
        if hygiene["hygiene"]:
            entry["hygiene"] = hygiene["hygiene"]
    return entry


def command_output(command: list[str]) -> str | None:
    try:
        result = subprocess.run(
            command,
            check=False,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    text = result.stdout.strip()
    return text or None


def file_sha256(path: Path) -> str | None:
    if not path.exists() or not path.is_file():
        return None
    digest = hashlib.sha256()
    try:
        with path.open("rb") as handle:
            for chunk in iter(lambda: handle.read(1024 * 1024), b""):
                digest.update(chunk)
    except OSError:
        return None
    return digest.hexdigest()


def build_environment_metadata() -> dict[str, Any]:
    harbor_path = shutil.which("harbor")
    metadata: dict[str, Any] = {
        "host_machine": platform.machine(),
        "host_platform": platform.platform(),
        "harbor_path": harbor_path,
        "harbor_version": command_output(["harbor", "--version"]) if harbor_path else None,
    }
    for arch in ("amd64", "arm64"):
        binary = Path(f"evals/harbor/artifacts/roder-linux-{arch}")
        if binary.exists():
            metadata[f"roder_linux_{arch}_sha256"] = file_sha256(binary)
    return {key: value for key, value in metadata.items() if value is not None}


def classify_scored_failure(task_name: str) -> str:
    for group, patterns in SCORED_GROUP_PATTERNS.items():
        if any(pattern in task_name for pattern in patterns):
            return group
    return "other"


def build_scored_groups(
    scored_trials: list[Trial], hygiene_by_trial: dict[str, dict[str, Any]]
) -> dict[str, Any]:
    groups: dict[str, dict[str, Any]] = {
        group: {
            "nearest_roder_subsystem": subsystem,
            "tasks": [],
        }
        for group, subsystem in SCORED_GROUP_SUBSYSTEMS.items()
    }
    for trial in scored_trials:
        group = classify_scored_failure(trial.task_name)
        groups[group]["tasks"].append(task_entry(trial, hygiene_by_trial.get(trial.name)))
    return {name: value for name, value in groups.items() if value["tasks"]}


def explain_scored_trial_difference(stats: dict[str, Any], classes: dict[str, list[dict[str, Any]]]) -> str:
    total_trials = int(
        stats.get("n_trials")
        or (
            int(stats.get("n_completed_trials") or 0)
            + int(stats.get("n_errored_trials") or 0)
            + int(stats.get("n_cancelled_trials") or 0)
            + int(stats.get("n_running_trials") or 0)
            + int(stats.get("n_pending_trials") or 0)
        )
    )
    evals = stats.get("evals")
    scored_trials = 0
    if isinstance(evals, dict):
        for value in evals.values():
            if isinstance(value, dict):
                scored_trials += int(value.get("n_trials") or 0)
    if not scored_trials:
        scored_trials = len(classes.get("pass", [])) + len(classes.get("scored_fail", []))
    unscored = total_trials - scored_trials
    return (
        f"Harbor total trials: {total_trials}; scored trials: {scored_trials}; "
        f"unscored setup/environment errors: {unscored}."
    )


def analyze_job(job_dir: Path, group_scored_failures: bool = False) -> dict[str, Any]:
    job_dir = job_dir.resolve()
    result_path = job_dir / "result.json"
    if not result_path.exists():
        raise FileNotFoundError(f"Missing Harbor job result: {result_path}")
    job_result = load_json(result_path)
    trials = load_trials(job_dir)

    classes: dict[str, list[dict[str, Any]]] = defaultdict(list)
    trial_classes: dict[str, list[str]] = {}
    records: list[dict[str, Any]] = []
    hygiene_by_trial: dict[str, dict[str, Any]] = {}
    for trial in trials:
        classified = sorted(classify_trial(trial))
        trial_classes[trial.name] = classified
        record = build_hygiene_record(trial, classified)
        records.append(record)
        hygiene_by_trial[trial.name] = record
        for class_name in classified:
            classes[class_name].append(task_entry(trial, record))

    stats = job_result.get("stats") if isinstance(job_result.get("stats"), dict) else {}
    clean_errors = {
        name: entries
        for name, entries in classes.items()
        if name in HARNESS_ERROR_CLASSES and entries
    }
    scored_trials = [trial for trial in trials if trial.reward == 0.0]
    analysis: dict[str, Any] = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "environment": build_environment_metadata(),
        "job_dir": str(job_dir),
        "job_name": job_dir.name,
        "result_id": job_result.get("id"),
        "started_at": job_result.get("started_at"),
        "finished_at": job_result.get("finished_at"),
        "stats": {
            "harbor": stats,
            "task_dirs": len(trials),
            "passes": len(classes.get("pass", [])),
            "scored_failures": len(classes.get("scored_fail", [])),
            "harness_error_classes": {
                name: len(entries) for name, entries in sorted(clean_errors.items())
            },
        },
        "explanations": [
            explain_scored_trial_difference(stats, classes),
            "Clean-run errors exclude reward-0 scored failures and include setup, environment, timeout, verifier, unknown, and artifact failures.",
            "Soft timeouts are adapter-controlled early exits before Harbor's hard timeout; they are scored normally and are not clean-run errors.",
            "Soft-timeout pass/fail subsets identify timeout-ladder rerun candidates without changing clean-run status.",
            "Hygiene labels and failed-trial rankings are evaluation-neutral post-run signals; they never inspect scoreable artifacts during a trial.",
        ],
        "classes": {name: entries for name, entries in sorted(classes.items())},
        "trial_classes": trial_classes,
        "trial_hygiene": {
            rec["trial_name"]: rec["hygiene"] for rec in records if rec["hygiene"]
        },
        "hygiene": hygiene_summary(records),
        "failed_trial_rankings": rank_failed_trials(records),
        "rerun_manifests": {
            name: {
                "class": name,
                "task_names": sorted({entry["task_name"] for entry in entries}),
                "trial_names": sorted({entry["trial_name"] for entry in entries}),
            }
            for name, entries in sorted(classes.items())
            if name != "pass"
        },
        "clean": not clean_errors,
    }
    if group_scored_failures:
        analysis["scored_failure_groups"] = build_scored_groups(scored_trials, hygiene_by_trial)
    return analysis


def _render_hygiene(analysis: dict[str, Any], lines: list[str]) -> None:
    hygiene = analysis.get("hygiene")
    if not hygiene:
        return
    lines.extend(["## Completion Hygiene", ""])
    lines.append("Evaluation-neutral post-run labels over reward-0 trials.")
    lines.append("")
    for label in HYGIENE_LABELS:
        info = hygiene.get(label, {"count": 0, "tasks": []})
        lines.append(f"### {label} ({info['count']})")
        if info["tasks"]:
            lines.append(f"- tasks: {', '.join(info['tasks'])}")
        lines.append("")


def _render_rankings(analysis: dict[str, Any], lines: list[str]) -> None:
    rankings = analysis.get("failed_trial_rankings")
    if not rankings:
        return
    lines.extend(["## Failed-Trial Rankings", ""])
    for dimension, entries in rankings.items():
        lines.append(f"### {dimension} ({len(entries)})")
        for entry in entries[:15]:
            detail = ", ".join(
                f"{key}={value}"
                for key, value in entry.items()
                if key not in ("trial_name", "task_name")
            )
            suffix = f" ({detail})" if detail else ""
            lines.append(f"- `{entry['task_name']}`{suffix}")
        lines.append("")


def render_markdown(analysis: dict[str, Any], include_groups: bool = False) -> str:
    stats = analysis["stats"]
    lines = [
        f"# Harbor TBench Analysis: {analysis['job_name']}",
        "",
        f"- Job dir: `{analysis['job_dir']}`",
        f"- Clean: `{str(analysis['clean']).lower()}`",
        f"- Task dirs: {stats['task_dirs']}",
        f"- Passes: {stats['passes']}",
        f"- Scored failures: {stats['scored_failures']}",
        "",
        "## Explanations",
        "",
    ]
    for explanation in analysis["explanations"]:
        lines.append(f"- {explanation}")
    lines.extend(["", "## Classes", ""])
    for name, entries in analysis["classes"].items():
        lines.append(f"### {name} ({len(entries)})")
        if not entries:
            lines.append("")
            continue
        for entry in entries:
            suffix = ""
            if "exception_type" in entry:
                suffix += f" exception={entry['exception_type']}"
            if entry.get("missing_artifacts"):
                suffix += f" missing_artifacts={','.join(entry['missing_artifacts'])}"
            if entry.get("hygiene"):
                suffix += f" hygiene={','.join(entry['hygiene'])}"
            lines.append(f"- `{entry['trial_name']}` task=`{entry['task_name']}`{suffix}")
        lines.append("")

    _render_hygiene(analysis, lines)
    _render_rankings(analysis, lines)

    if include_groups and analysis.get("scored_failure_groups"):
        lines.extend(["## Scored Failure Groups", ""])
        for name, group in analysis["scored_failure_groups"].items():
            lines.append(f"### {name} ({len(group['tasks'])})")
            lines.append(f"- Nearest Roder subsystem: {group['nearest_roder_subsystem']}")
            for entry in group["tasks"]:
                lines.append(f"- `{entry['trial_name']}`")
            lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def write_manifests(analysis: dict[str, Any], manifest_dir: Path) -> None:
    manifest_dir.mkdir(parents=True, exist_ok=True)
    for name, manifest in analysis["rerun_manifests"].items():
        (manifest_dir / f"{name}.json").write_text(json.dumps(manifest, indent=2) + "\n")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("job_dir", type=Path)
    parser.add_argument("--json", dest="json_path", type=Path)
    parser.add_argument("--markdown", type=Path)
    parser.add_argument("--manifest-dir", type=Path)
    parser.add_argument("--require-clean", action="store_true")
    parser.add_argument("--group-scored-failures", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        analysis = analyze_job(args.job_dir, group_scored_failures=args.group_scored_failures)
    except Exception as exc:
        print(f"analyze_tbench_run: {exc}", file=sys.stderr)
        return 2

    if args.json_path:
        args.json_path.parent.mkdir(parents=True, exist_ok=True)
        args.json_path.write_text(json.dumps(analysis, indent=2) + "\n")

    markdown = render_markdown(analysis, include_groups=args.group_scored_failures)
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        args.markdown.write_text(markdown)

    if args.manifest_dir:
        write_manifests(analysis, args.manifest_dir)

    if not args.json_path and not args.markdown:
        print(markdown, end="")

    if args.require_clean and not analysis["clean"]:
        errors = analysis["stats"]["harness_error_classes"]
        print(f"Harbor run is not clean: {errors}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
