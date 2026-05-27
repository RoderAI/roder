#!/usr/bin/env python3
"""Validate generated routed Harbor Terminal-Bench campaign configs."""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path
from typing import Any

from tbench_campaign_handoff import validate_pre_eval_handoff
from tbench_campaign_run_script import validate_run_script
from tbench_campaign_image_preflight import validate_route_image_preflight
from tbench_campaign_launch_plans import LaunchPlanSet, validate_route_launch_plan
from tbench_campaign_route_readiness import validate_route_config_agent_readiness
from tbench_campaign_score_projection import score_projection_for_tasks
from validate_tbench_analysis import (
    DEFAULT_BASELINE,
    compare_analysis_to_baseline,
)


class ValidationResult:
    def __init__(self) -> None:
        self.issues: list[str] = []

    @property
    def ok(self) -> bool:
        return not self.issues

    def add(self, issue: str) -> None:
        self.issues.append(issue)


def load_json(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text())
    if not isinstance(data, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return data


def validate_campaign_manifest(
    manifest: dict[str, Any],
    *,
    manifest_path: Path,
    require_image_preflight: bool = False,
    require_analysis: bool = False,
    require_launch_plans: bool = False,
    allow_dry_run_launch_plans: bool = False,
    preflight_dir: Path | None = None,
    analysis_baseline: Path = DEFAULT_BASELINE,
) -> ValidationResult:
    result = ValidationResult()
    routes = list_value(manifest.get("routes"))
    summary = dict_value(manifest.get("summary"))
    if not isinstance(manifest.get("campaign"), str) or not manifest.get("campaign"):
        result.add("campaign is missing")
    if not routes:
        result.add("routes are missing")

    validate_run_script(result, manifest.get("runScript"), routes=routes)
    validate_pre_eval_handoff(
        result,
        manifest.get("preEval"),
        manifest_path=manifest_path,
        run_script=manifest.get("runScript"),
    )
    validate_manifest_summary(result, routes, summary)
    validate_score_projection(result, routes, manifest.get("scoreProjection"))
    seen_tasks: dict[str, str] = {}
    launch_plan_set = LaunchPlanSet() if require_launch_plans else None
    for route in routes:
        if not isinstance(route, dict):
            result.add("route entry must be an object")
            continue
        validate_route(
            result,
            route,
            seen_tasks=seen_tasks,
            require_image_preflight=require_image_preflight,
            require_analysis=require_analysis,
            require_launch_plans=require_launch_plans,
            allow_dry_run_launch_plans=allow_dry_run_launch_plans,
            launch_plan_set=launch_plan_set,
            preflight_dir=preflight_dir or manifest_path.parent,
            analysis_baseline=analysis_baseline,
        )
    return result


def validate_manifest_summary(
    result: ValidationResult,
    routes: list[Any],
    summary: dict[str, Any],
) -> None:
    route_count = len([route for route in routes if isinstance(route, dict)])
    route_tasks = [
        str(task)
        for route in routes
        if isinstance(route, dict)
        for task in list_value(route.get("tasks"))
    ]
    if int_value(summary.get("routes")) != route_count:
        result.add("summary routes count mismatch")
    if int_value(summary.get("tasks")) != len(route_tasks):
        result.add("summary tasks count mismatch")
    if int_value(summary.get("uniqueTasks")) != len(set(route_tasks)):
        result.add("summary uniqueTasks count mismatch")
    unique_names = [str(task) for task in list_value(summary.get("uniqueTaskNames"))]
    if unique_names and unique_names != sorted(set(route_tasks)):
        result.add("summary uniqueTaskNames mismatch")


def validate_score_projection(
    result: ValidationResult,
    routes: list[Any],
    projection: Any,
) -> None:
    if not isinstance(projection, dict):
        result.add("scoreProjection is missing")
        return
    route_tasks = [
        str(task)
        for route in routes
        if isinstance(route, dict)
        for task in list_value(route.get("tasks"))
    ]
    expected = score_projection_for_tasks(route_tasks)
    if set(projection) != set(expected):
        result.add("scoreProjection fields mismatch")
    for field, expected_value in expected.items():
        if projection.get(field) != expected_value:
            result.add(f"scoreProjection {field} mismatch")


def validate_route(
    result: ValidationResult,
    route: dict[str, Any],
    *,
    seen_tasks: dict[str, str],
    require_image_preflight: bool,
    require_analysis: bool,
    require_launch_plans: bool,
    allow_dry_run_launch_plans: bool,
    launch_plan_set: LaunchPlanSet | None,
    preflight_dir: Path,
    analysis_baseline: Path,
) -> None:
    name = str(route.get("name") or "<missing>")
    tasks = [str(task) for task in list_value(route.get("tasks"))]
    if not tasks:
        result.add(f"route {name} has no tasks")
    if int_value(route.get("taskCount")) != len(tasks):
        result.add(f"route {name} taskCount mismatch")
    for task in tasks:
        if task in seen_tasks:
            result.add(f"task {task} appears in multiple routes")
        seen_tasks[task] = name

    config_path = route.get("config")
    if not isinstance(config_path, str) or not config_path:
        result.add(f"route {name} config is missing")
        return
    config_file = Path(config_path)
    try:
        config = load_json(config_file)
    except Exception as exc:
        result.add(f"route {name} config cannot be read: {exc}")
        return
    validate_route_config_hash(result, name, route, config_file)

    if config.get("job_name") != route.get("jobName"):
        result.add(f"route {name} job_name mismatch")
    validate_route_analysis_paths(result, name, route, config, preflight_dir)
    validate_route_image_manifest_reference(result, name, route)
    validate_route_config_dataset(result, name, config, tasks)
    validate_route_config_agent(result, name, config, route)
    validate_route_config_runtime(result, name, config)
    if require_image_preflight:
        validate_route_image_preflight(
            result,
            name,
            route=route,
            config_path=config_path,
            preflight_dir=preflight_dir,
        )
    if require_analysis:
        validate_route_analysis_outputs(result, name, route, analysis_baseline)
    if require_launch_plans:
        validate_route_launch_plan(
            result,
            name,
            route=route,
            allow_dry_run=allow_dry_run_launch_plans,
            plan_set=launch_plan_set,
        )


def validate_route_analysis_paths(
    result: ValidationResult,
    name: str,
    route: dict[str, Any],
    config: dict[str, Any],
    output_dir: Path,
) -> None:
    job_name = route.get("jobName")
    if not isinstance(job_name, str) or not job_name:
        result.add(f"route {name} jobName is missing")
        return
    expected_job_dir = str(Path(config.get("jobs_dir", "evals/harbor/jobs")) / job_name)
    expected_analysis_json = str(output_dir / f"{name}-analysis.json")
    expected_analysis_markdown = str(output_dir / f"{name}.md")
    expected_manifest_dir = str(output_dir / "manifests" / name)
    expected_launch_plan = str(output_dir / f"{name}-launch-plan.json")
    for field, expected in (
        ("jobDir", expected_job_dir),
        ("analysisJson", expected_analysis_json),
        ("analysisMarkdown", expected_analysis_markdown),
        ("analysisManifestDir", expected_manifest_dir),
        ("launchPlan", expected_launch_plan),
    ):
        if route.get(field) != expected:
            result.add(f"route {name} {field} mismatch")


def validate_route_image_manifest_reference(
    result: ValidationResult,
    name: str,
    route: dict[str, Any],
) -> None:
    manifest_path = route.get("imageManifest")
    if not isinstance(manifest_path, str) or not manifest_path:
        result.add(f"route {name} imageManifest is missing")


def validate_route_config_dataset(
    result: ValidationResult,
    name: str,
    config: dict[str, Any],
    tasks: list[str],
) -> None:
    datasets = list_value(config.get("datasets"))
    if len(datasets) != 1 or not isinstance(datasets[0], dict):
        result.add(f"route {name} must have exactly one dataset")
        return
    dataset = datasets[0]
    if [str(task) for task in list_value(dataset.get("task_names"))] != tasks:
        result.add(f"route {name} task_names mismatch")
    if int_value(dataset.get("n_tasks")) != len(tasks):
        result.add(f"route {name} n_tasks mismatch")


def validate_route_config_agent(
    result: ValidationResult,
    name: str,
    config: dict[str, Any],
    route: dict[str, Any],
) -> None:
    agents = list_value(config.get("agents"))
    if len(agents) != 1 or not isinstance(agents[0], dict):
        result.add(f"route {name} must have exactly one agent")
        return
    agent = agents[0]
    kwargs = dict_value(agent.get("kwargs"))
    if kwargs.get("reasoning") != route.get("reasoning"):
        result.add(f"route {name} reasoning mismatch")
    validate_route_config_agent_readiness(result, name, agent, kwargs)
    plan_first = route.get("planFirst") is True
    if bool(kwargs.get("plan_first_enabled", False)) != plan_first:
        result.add(f"route {name} plan-first setting mismatch")
    if plan_first:
        if kwargs.get("plan_first_reasoning") != route.get("planFirstReasoning"):
            result.add(f"route {name} plan-first reasoning mismatch")
        if int_value(kwargs.get("plan_first_soft_timeout_sec")) != int_value(
            route.get("planFirstSoftTimeoutSec")
        ):
            result.add(f"route {name} plan-first soft timeout mismatch")
        artifacts = {str(item) for item in list_value(config.get("artifacts"))}
        if "/logs/agent/roder-plan.md" not in artifacts:
            result.add(f"route {name} missing plan-first artifacts")


def validate_route_config_runtime(
    result: ValidationResult,
    name: str,
    config: dict[str, Any],
) -> None:
    orchestrator = dict_value(config.get("orchestrator"))
    if int_value(orchestrator.get("n_concurrent_trials")) != 4:
        result.add(f"route {name} parallelism is not 4")
    environment = dict_value(config.get("environment"))
    if environment.get("delete") is not False:
        result.add(f"route {name} environment.delete is not false")
    artifacts = {str(item) for item in list_value(config.get("artifacts"))}
    required_artifacts = {
        "/logs/agent/roder-cli.txt",
        "/logs/agent/roder-events.jsonl",
        "/logs/agent/roder-stderr.txt",
        "/logs/agent/roder-last-message.txt",
        "/logs/agent/setup-summary.txt",
        "/logs/agent/roder-run-summary.json",
    }
    missing = sorted(required_artifacts.difference(artifacts))
    if missing:
        result.add(f"route {name} missing deterministic artifacts: {', '.join(missing)}")


def validate_route_analysis_outputs(
    result: ValidationResult,
    name: str,
    route: dict[str, Any],
    analysis_baseline: Path,
) -> None:
    analysis_json = route.get("analysisJson")
    analysis_markdown = route.get("analysisMarkdown")
    if not isinstance(analysis_json, str) or not analysis_json:
        result.add(f"route {name} analysisJson is missing")
        return
    try:
        analysis = load_json(Path(analysis_json))
    except Exception as exc:
        result.add(f"route {name} analysis JSON cannot be read: {exc}")
        return
    if not isinstance(analysis_markdown, str) or not analysis_markdown:
        result.add(f"route {name} analysisMarkdown is missing")
    elif not Path(analysis_markdown).is_file():
        result.add(f"route {name} analysis Markdown cannot be read")
    if analysis.get("clean") is not True:
        result.add(f"route {name} analysis is not clean")
    stats = dict_value(analysis.get("stats"))
    harbor = dict_value(stats.get("harbor"))
    task_count = int_value(route.get("taskCount"))
    if int_value(harbor.get("n_errors")) != 0:
        result.add(f"route {name} Harbor errors are non-zero")
    if int_value(harbor.get("n_trials")) != task_count:
        result.add(f"route {name} analysis trial count mismatch")
    scored_trials = int_value(stats.get("passes")) + int_value(stats.get("scored_failures"))
    if scored_trials != task_count:
        result.add(f"route {name} analysis scored trial count mismatch")
    validate_route_analysis_task_entries(result, name, route, analysis)
    harness_errors = dict_value(stats.get("harness_error_classes"))
    if sum(int_value(value) for value in harness_errors.values()) != 0:
        result.add(f"route {name} analysis has harness errors")
    validate_route_analysis_baseline(
        result,
        name,
        analysis=analysis,
        task_count=task_count,
        baseline_path=analysis_baseline,
    )


def validate_route_analysis_baseline(
    result: ValidationResult,
    name: str,
    *,
    analysis: dict[str, Any],
    task_count: int,
    baseline_path: Path,
) -> None:
    try:
        baseline = load_json(baseline_path)
    except Exception as exc:
        result.add(f"route {name} analysis baseline cannot be read: {exc}")
        return
    comparison = compare_analysis_to_baseline(
        analysis,
        baseline,
        expected_trials=task_count,
    )
    blocked = [
        str(row.get("metric") or "<missing>")
        for row in list_value(comparison.get("rows"))
        if isinstance(row, dict) and row.get("status") == "blocked"
    ]
    if blocked:
        result.add(
            f"route {name} analysis baseline blocked: {', '.join(blocked)}"
        )


def validate_route_analysis_task_entries(
    result: ValidationResult,
    name: str,
    route: dict[str, Any],
    analysis: dict[str, Any],
) -> None:
    expected = sorted(str(task) for task in list_value(route.get("tasks")))
    entries = scored_analysis_task_entries(analysis)
    if len(entries) != len(expected) or len(set(entries)) != len(entries):
        result.add(f"route {name} analysis scored task entries mismatch")
    if sorted(set(entries)) != expected:
        result.add(f"route {name} analysis task names mismatch")


def scored_analysis_task_entries(analysis: dict[str, Any]) -> list[str]:
    classes = dict_value(analysis.get("classes"))
    names: list[str] = []
    for class_name in ("pass", "scored_fail"):
        for entry in list_value(classes.get(class_name)):
            if not isinstance(entry, dict):
                continue
            name = entry.get("task_name") or entry.get("task")
            if not isinstance(name, str) or not name:
                trial_name = entry.get("trial_name")
                if isinstance(trial_name, str) and trial_name:
                    name = trial_name.split("__", 1)[0]
            if isinstance(name, str) and name:
                names.append(name)
    return names


def validate_route_config_hash(
    result: ValidationResult,
    name: str,
    route: dict[str, Any],
    config_path: Path,
) -> None:
    expected = route.get("configSha256")
    if not isinstance(expected, str) or not expected:
        result.add(f"route {name} configSha256 is missing")
        return
    actual = hashlib.sha256(config_path.read_bytes()).hexdigest()
    if actual != expected:
        result.add(f"route {name} configSha256 mismatch")


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    parser.add_argument("--require-image-preflight", action="store_true")
    parser.add_argument("--require-analysis", action="store_true")
    parser.add_argument("--require-launch-plans", action="store_true")
    parser.add_argument("--allow-dry-run-launch-plans", action="store_true")
    parser.add_argument("--preflight-dir", type=Path)
    parser.add_argument("--analysis-baseline", type=Path, default=DEFAULT_BASELINE)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        manifest = load_json(args.manifest)
        result = validate_campaign_manifest(
            manifest,
            manifest_path=args.manifest,
            require_image_preflight=args.require_image_preflight,
            require_analysis=args.require_analysis,
            require_launch_plans=args.require_launch_plans,
            allow_dry_run_launch_plans=args.allow_dry_run_launch_plans,
            preflight_dir=args.preflight_dir,
            analysis_baseline=args.analysis_baseline,
        )
    except Exception as exc:
        print(f"validate_tbench_campaign: {exc}", file=sys.stderr)
        return 2
    if not result.ok:
        for issue in result.issues:
            print(issue, file=sys.stderr)
        return 1
    print(f"TBench campaign validation passed: {args.manifest}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
