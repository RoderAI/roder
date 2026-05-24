#!/usr/bin/env python3
"""Analyze a Harbor Terminal-Bench job produced by the Roder harness."""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


HARNESS_ERROR_CLASSES = {
    "docker_registry_bad_gateway",
    "agent_setup_failed",
    "agent_timeout",
    "missing_artifacts",
    "verifier_error",
    "unknown_error",
}

CORE_ARTIFACTS = (
    "roder-cli.txt",
    "roder-events.jsonl",
    "roder-stderr.txt",
    "roder-last-message.txt",
)

SCORED_GROUP_PATTERNS = {
    "ML/scientific": (
        "torch",
        "train-fasttext",
        "mteb",
        "raman",
        "mcmc",
        "protein",
        "financial-document",
        "tune-mjcf",
        "count-dataset",
        "query-optimize",
    ),
    "systems/emulation/services": (
        "kv-store",
        "mailman",
        "install-windows",
        "mips",
        "make-doom",
        "polyglot-rust-c",
    ),
    "media/geometry": (
        "path-tracing",
        "video-processing",
        "gcode",
    ),
    "synthesis/security/math": (
        "gpt2-codegolf",
        "regex",
        "overfull-hbox",
        "fix-code-vulnerability",
        "chess",
        "winning-avg-corewars",
    ),
}

SCORED_GROUP_SUBSYSTEMS = {
    "ML/scientific": "runtime context, package-install planning, long-running command monitoring, and verification discipline",
    "systems/emulation/services": "shell/process tooling, service startup validation, and timeout/deadline handling",
    "media/geometry": "artifact inspection, binary/media tooling, and iterative verifier feedback",
    "synthesis/security/math": "search/context retrieval, exact-output discipline, and test-driven repair loops",
    "other": "task-specific analysis after clean harness artifacts are available",
}


@dataclass
class Trial:
    name: str
    task_name: str
    path: Path
    result: dict[str, Any]
    config: dict[str, Any]
    trial_log: str
    exception_text: str
    setup_text: str

    @property
    def combined_text(self) -> str:
        chunks = [self.trial_log, self.exception_text, self.setup_text]
        return "\n".join(chunk for chunk in chunks if chunk)

    @property
    def exception_info(self) -> dict[str, Any] | None:
        info = self.result.get("exception_info")
        return info if isinstance(info, dict) else None

    @property
    def exception_type(self) -> str | None:
        info = self.exception_info
        value = info.get("exception_type") if info else None
        return str(value) if value else None

    @property
    def reward(self) -> float | None:
        verifier = self.result.get("verifier_result")
        if not isinstance(verifier, dict):
            return None
        rewards = verifier.get("rewards")
        if not isinstance(rewards, dict):
            return None
        reward = rewards.get("reward")
        try:
            return float(reward)
        except (TypeError, ValueError):
            return None

    @property
    def expected_artifacts(self) -> list[str]:
        artifacts = self.config.get("artifacts")
        if not isinstance(artifacts, list):
            return []
        names: list[str] = []
        for artifact in artifacts:
            if not isinstance(artifact, str):
                continue
            if artifact.startswith("/logs/agent/"):
                names.append(artifact.removeprefix("/logs/agent/"))
            else:
                names.append(Path(artifact).name)
        return names

    def has_agent_started(self) -> bool:
        return self.result.get("agent_execution") is not None or (
            self.path / "agent" / "command-0"
        ).exists()

    def missing_expected_artifacts(self) -> list[str]:
        agent_dir = self.path / "agent"
        missing = [name for name in self.expected_artifacts if not (agent_dir / name).exists()]
        if self.has_agent_started():
            missing.extend(
                name for name in CORE_ARTIFACTS if name not in missing and not (agent_dir / name).exists()
            )
        return sorted(set(missing))


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def read_text(path: Path) -> str:
    if not path.exists():
        return ""
    try:
        return path.read_text(errors="replace")
    except OSError:
        return ""


def setup_text(trial_dir: Path) -> str:
    setup_dir = trial_dir / "agent" / "setup"
    chunks = []
    for name in ("return-code.txt", "stdout.txt", "stderr.txt"):
        text = read_text(setup_dir / name)
        if text:
            chunks.append(f"--- {name} ---\n{text}")
    summary = read_text(trial_dir / "agent" / "setup-summary.txt")
    if summary:
        chunks.append(f"--- setup-summary.txt ---\n{summary}")
    return "\n".join(chunks)


def task_name_from_trial_name(name: str) -> str:
    return name.split("__", 1)[0]


def load_trials(job_dir: Path) -> list[Trial]:
    trials: list[Trial] = []
    for result_path in sorted(job_dir.glob("*/result.json")):
        trial_dir = result_path.parent
        result = load_json(result_path)
        config_path = trial_dir / "config.json"
        config = load_json(config_path) if config_path.exists() else {}
        name = str(result.get("trial_name") or trial_dir.name)
        task_name = str(result.get("task_name") or task_name_from_trial_name(name))
        exception = read_text(trial_dir / "exception.txt")
        trials.append(
            Trial(
                name=name,
                task_name=task_name,
                path=trial_dir,
                result=result,
                config=config,
                trial_log=read_text(trial_dir / "trial.log"),
                exception_text=exception,
                setup_text=setup_text(trial_dir),
            )
        )
    return trials


def classify_trial(trial: Trial) -> set[str]:
    classes: set[str] = set()
    text = trial.combined_text

    if trial.reward == 1.0 and not trial.exception_info:
        classes.add("pass")
    if trial.reward == 0.0:
        classes.add("scored_fail")

    if "registry-1.docker.io" in text and "Bad Gateway" in text:
        classes.add("docker_registry_bad_gateway")
    elif "Bad Gateway" in text and re.search(r"\bImage\b|\bdocker\b", text, re.I):
        classes.add("docker_registry_bad_gateway")

    if trial.exception_type == "AgentTimeoutError" or "Agent execution timed out" in text:
        classes.add("agent_timeout")
    if "roder exec soft-timed-out" in text:
        classes.add("soft_timeout")

    setup_return = read_text(trial.path / "agent" / "setup" / "return-code.txt").strip()
    if (
        "Agent setup failed" in text
        or (setup_return and setup_return != "0" and not trial.has_agent_started())
    ):
        classes.add("agent_setup_failed")

    if "Failed to download artifact" in trial.trial_log:
        classes.add("missing_artifacts")
    elif trial.has_agent_started() and trial.missing_expected_artifacts():
        classes.add("missing_artifacts")

    if trial.exception_info and not classes.intersection(
        {"docker_registry_bad_gateway", "agent_timeout", "agent_setup_failed"}
    ):
        if trial.exception_type and "Verifier" in trial.exception_type:
            classes.add("verifier_error")
        else:
            classes.add("unknown_error")

    if not classes:
        classes.add("unknown")
    return classes


def task_entry(trial: Trial) -> dict[str, Any]:
    entry = {
        "trial_name": trial.name,
        "task_name": trial.task_name,
        "path": str(trial.path),
    }
    if trial.reward is not None:
        entry["reward"] = trial.reward
    if trial.exception_type:
        entry["exception_type"] = trial.exception_type
    missing = trial.missing_expected_artifacts()
    if missing:
        entry["missing_artifacts"] = missing
    return entry


def classify_scored_failure(task_name: str) -> str:
    for group, patterns in SCORED_GROUP_PATTERNS.items():
        if any(pattern in task_name for pattern in patterns):
            return group
    return "other"


def build_scored_groups(scored_trials: list[Trial]) -> dict[str, Any]:
    groups: dict[str, dict[str, Any]] = {
        group: {
            "nearest_roder_subsystem": subsystem,
            "tasks": [],
        }
        for group, subsystem in SCORED_GROUP_SUBSYSTEMS.items()
    }
    for trial in scored_trials:
        group = classify_scored_failure(trial.task_name)
        groups[group]["tasks"].append(task_entry(trial))
    return {name: value for name, value in groups.items() if value["tasks"]}


def explain_scored_trial_difference(stats: dict[str, Any], classes: dict[str, list[dict[str, Any]]]) -> str:
    total_trials = int(stats.get("n_trials") or 0)
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
    for trial in trials:
        classified = sorted(classify_trial(trial))
        trial_classes[trial.name] = classified
        for class_name in classified:
            classes[class_name].append(task_entry(trial))

    stats = job_result.get("stats") if isinstance(job_result.get("stats"), dict) else {}
    clean_errors = {
        name: entries
        for name, entries in classes.items()
        if name in HARNESS_ERROR_CLASSES and entries
    }
    scored_trials = [trial for trial in trials if trial.reward == 0.0]
    analysis: dict[str, Any] = {
        "generated_at": datetime.now(timezone.utc).isoformat(),
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
        ],
        "classes": {name: entries for name, entries in sorted(classes.items())},
        "trial_classes": trial_classes,
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
        analysis["scored_failure_groups"] = build_scored_groups(scored_trials)
    return analysis


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
            lines.append(f"- `{entry['trial_name']}` task=`{entry['task_name']}`{suffix}")
        lines.append("")

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
