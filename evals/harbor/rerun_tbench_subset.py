#!/usr/bin/env python3
"""Generate a Harbor config for rerunning a classified Terminal-Bench subset."""

from __future__ import annotations

import argparse
import copy
import json
import re
import sys
from pathlib import Path
from typing import Any

from analyze_tbench_run import analyze_job


CLASS_ALIASES = {
    "pass": "pass",
    "passes": "pass",
    "scored_fail": "scored_fail",
    "scored_failure": "scored_fail",
    "scored_failures": "scored_fail",
    "docker_registry_bad_gateway": "docker_registry_bad_gateway",
    "registry": "docker_registry_bad_gateway",
    "setup": "agent_setup_failed",
    "agent_setup_failed": "agent_setup_failed",
    "timeout": "agent_timeout",
    "agent_timeout": "agent_timeout",
    "soft_timeout": "soft_timeout",
    "missing_artifacts": "missing_artifacts",
}

DETERMINISTIC_ARTIFACTS = [
    "/logs/agent/roder-cli.txt",
    "/logs/agent/roder-events.jsonl",
    "/logs/agent/roder-stderr.txt",
    "/logs/agent/roder-last-message.txt",
    "/logs/agent/setup-summary.txt",
]


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def find_base_config(source_job: Path, explicit: Path | None) -> Path:
    if explicit:
        return explicit
    for path in sorted(Path("evals/harbor").glob("*.json")):
        try:
            config = load_json(path)
        except Exception:
            continue
        if config.get("job_name") == source_job.name:
            return path
    raise FileNotFoundError(
        "Could not infer base config. Pass --base-config explicitly."
    )


def slug(value: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "-", value).strip("-")


def selected_task_names(analysis: dict[str, Any], class_name: str) -> list[str]:
    canonical = CLASS_ALIASES.get(class_name, class_name)
    manifest = analysis["rerun_manifests"].get(canonical)
    if not manifest:
        available = ", ".join(sorted(analysis["rerun_manifests"].keys()))
        raise ValueError(f"No tasks for class {class_name!r}. Available: {available}")
    return sorted(manifest["task_names"])


def build_subset_config(
    source_job: Path,
    base_config: dict[str, Any],
    task_names: list[str],
    class_name: str,
    job_name: str | None,
    jobs_dir: str | None,
    timeout_sec: float | None,
    soft_timeout_sec: float | None,
) -> dict[str, Any]:
    config = copy.deepcopy(base_config)
    agent = (config.get("agents") or [{}])[0]
    model = str(agent.get("model_name") or "model")
    reasoning = str((agent.get("kwargs") or {}).get("reasoning") or "default")
    config["job_name"] = job_name or (
        f"{source_job.name}-{slug(class_name)}-{slug(model)}-{slug(reasoning)}"
    )
    if jobs_dir:
        config["jobs_dir"] = jobs_dir
    if timeout_sec is not None:
        for agent_config in config.get("agents", []):
            agent_config["override_timeout_sec"] = timeout_sec
    if soft_timeout_sec is not None:
        for agent_config in config.get("agents", []):
            kwargs = agent_config.setdefault("kwargs", {})
            kwargs["soft_timeout_sec"] = soft_timeout_sec
    for dataset in config.get("datasets", []):
        if isinstance(dataset, dict):
            dataset["task_names"] = task_names
            dataset["n_tasks"] = len(task_names)
    config["tasks"] = []
    config["artifacts"] = DETERMINISTIC_ARTIFACTS
    return config


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source-job", type=Path, required=True)
    parser.add_argument("--class", dest="class_name", required=True)
    parser.add_argument("--output-config", type=Path, required=True)
    parser.add_argument("--base-config", type=Path)
    parser.add_argument("--job-name")
    parser.add_argument("--jobs-dir")
    parser.add_argument("--timeout-sec", type=float)
    parser.add_argument("--soft-timeout-sec", type=float)
    parser.add_argument("--task-name", action="append", default=[])
    parser.add_argument("--limit", type=int)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        source_job = args.source_job
        analysis = analyze_job(source_job, group_scored_failures=True)
        task_names = selected_task_names(analysis, args.class_name)
        if args.task_name:
            requested = set(args.task_name)
            task_names = [name for name in task_names if name in requested]
            missing = sorted(requested.difference(task_names))
            if missing:
                raise ValueError(f"Requested task names not in class: {', '.join(missing)}")
        if args.limit is not None:
            task_names = task_names[: args.limit]
        if not task_names:
            raise ValueError("No task names selected for rerun.")
        base_config_path = find_base_config(source_job, args.base_config)
        base_config = load_json(base_config_path)
        subset = build_subset_config(
            source_job=source_job,
            base_config=base_config,
            task_names=task_names,
            class_name=args.class_name,
            job_name=args.job_name,
            jobs_dir=args.jobs_dir,
            timeout_sec=args.timeout_sec,
            soft_timeout_sec=args.soft_timeout_sec,
        )
    except Exception as exc:
        print(f"rerun_tbench_subset: {exc}", file=sys.stderr)
        return 2

    args.output_config.parent.mkdir(parents=True, exist_ok=True)
    args.output_config.write_text(json.dumps(subset, indent=2) + "\n")
    print(
        f"Wrote {args.output_config} with {len(task_names)} tasks "
        f"for class {CLASS_ALIASES.get(args.class_name, args.class_name)}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
