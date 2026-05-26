#!/usr/bin/env python3
"""Summarize combined Terminal-Bench campaign manifests."""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from tbench_campaign_score_projection import score_projection_for_tasks  # noqa: E402


ManifestInput = tuple[Path, dict[str, Any]]


PRESETS: dict[str, dict[str, Any]] = {
    "validated-plus-historical": {
        "require_no_overlap": True,
        "expect_unique_tasks": 18,
        "expect_projected_passes": 68,
        "expect_tasks": [
            "password-recovery",
            "qemu-startup",
            "vulnerable-secret",
        ],
        "expect_owners": [
            "password-recovery=historical-wins/policy-framed",
            "qemu-startup=historical-wins/environment-targeted",
            "vulnerable-secret=historical-wins/policy-framed",
        ],
    },
}


def load_json_object(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def summarize_campaign_manifests(
    manifests: list[ManifestInput] | tuple[ManifestInput, ...],
) -> dict[str, Any]:
    campaigns: list[dict[str, Any]] = []
    task_owners: dict[str, list[str]] = {}
    route_count = 0
    task_rows = 0

    for path, manifest in manifests:
        campaign_name = manifest_campaign_name(path, manifest)
        routes = manifest_routes(path, manifest)
        campaign_task_rows = 0
        campaign_tasks: set[str] = set()
        route_entries: list[dict[str, Any]] = []

        for route in routes:
            route_name = route_name_value(path, route)
            tasks = route_tasks(path, campaign_name, route_name, route)
            owner = f"{campaign_name}/{route_name}"
            route_count += 1
            task_rows += len(tasks)
            campaign_task_rows += len(tasks)
            campaign_tasks.update(tasks)
            for task in tasks:
                task_owners.setdefault(task, []).append(owner)
            route_entries.append(
                {
                    "name": route_name,
                    "tasks": len(tasks),
                    "uniqueTasks": len(set(tasks)),
                    "taskNames": tasks,
                }
            )

        campaigns.append(
            {
                "campaign": campaign_name,
                "manifest": str(path),
                "routes": len(route_entries),
                "tasks": campaign_task_rows,
                "uniqueTasks": len(campaign_tasks),
                "routeEntries": route_entries,
            }
        )

    duplicates = [
        {"taskName": task, "owners": owners}
        for task, owners in sorted(task_owners.items())
        if len(owners) > 1
    ]
    unique_tasks = sorted(task_owners)
    return {
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "summary": {
            "campaigns": len(campaigns),
            "routes": route_count,
            "tasks": task_rows,
            "uniqueTasks": len(unique_tasks),
            "duplicateTasks": len(duplicates),
            "uniqueTaskNames": unique_tasks,
        },
        "scoreProjection": score_projection_for_tasks(unique_tasks),
        "duplicates": duplicates,
        "campaigns": campaigns,
    }


def manifest_campaign_name(path: Path, manifest: dict[str, Any]) -> str:
    value = manifest.get("campaign")
    if isinstance(value, str) and value:
        return value
    return path.stem.removesuffix("-manifest")


def manifest_routes(path: Path, manifest: dict[str, Any]) -> list[dict[str, Any]]:
    value = manifest.get("routes")
    if not isinstance(value, list):
        raise ValueError(f"{path} routes must be a list")
    routes: list[dict[str, Any]] = []
    for index, item in enumerate(value):
        if not isinstance(item, dict):
            raise ValueError(f"{path} route {index} must be a JSON object")
        routes.append(item)
    return routes


def route_name_value(path: Path, route: dict[str, Any]) -> str:
    value = route.get("name")
    if not isinstance(value, str) or not value:
        raise ValueError(f"{path} route name is missing")
    return value


def route_tasks(
    path: Path,
    campaign_name: str,
    route_name: str,
    route: dict[str, Any],
) -> list[str]:
    value = route.get("tasks")
    if not isinstance(value, list):
        raise ValueError(f"{path} route {campaign_name}/{route_name} tasks must be a list")
    tasks = [str(item) for item in value if str(item)]
    if not tasks:
        raise ValueError(f"{path} route {campaign_name}/{route_name} has no tasks")
    return tasks


def render_markdown(report: dict[str, Any]) -> str:
    summary = report["summary"]
    projection = report["scoreProjection"]
    projected_passes = projection["projectedPassesIfAllRoutesPass"]
    suite_tasks = projection["suiteTasks"]
    lines = [
        "# TBench Campaign Combination Summary",
        "",
        f"- Campaigns: `{summary['campaigns']}`",
        f"- Routes: `{summary['routes']}`",
        f"- Task rows: `{summary['tasks']}`",
        f"- Unique tasks: `{summary['uniqueTasks']}`",
        f"- Duplicate tasks: `{summary['duplicateTasks']}`",
        f"- Projected passes if all routes pass: `{projected_passes}/{suite_tasks}`",
        f"- Codex CLI gap: `{projection['codexCliGap']}`",
        f"- SOTA gap: `{projection['sotaGap']}`",
        "",
        "| Campaign | Routes | Task rows | Unique tasks |",
        "| --- | ---: | ---: | ---: |",
    ]
    for campaign in report["campaigns"]:
        lines.append(
            "| `{}` | {} | {} | {} |".format(
                campaign["campaign"],
                campaign["routes"],
                campaign["tasks"],
                campaign["uniqueTasks"],
            )
        )
    lines.extend(["", "## Duplicate Tasks", ""])
    duplicates = report.get("duplicates")
    if not duplicates:
        lines.append("No duplicate campaign tasks.")
    else:
        lines.extend(
            [
                "| Task | Owners |",
                "| --- | --- |",
            ]
        )
        for duplicate in duplicates:
            lines.append(
                "| `{}` | {} |".format(
                    duplicate["taskName"],
                    ", ".join(f"`{owner}`" for owner in duplicate["owners"]),
                )
            )
    return "\n".join(lines).rstrip() + "\n"


def expectation_issues(
    report: dict[str, Any],
    *,
    expected_unique_tasks: int | None = None,
    expected_projected_passes: int | None = None,
    expected_tasks: list[str] | tuple[str, ...] | None = None,
    expected_owners: list[str] | tuple[str, ...] | None = None,
) -> list[str]:
    summary = report["summary"]
    projection = report["scoreProjection"]
    unique_tasks = set(summary["uniqueTaskNames"])
    owners_by_task = task_owners_by_name(report)
    checks = [
        (
            "uniqueTasks",
            expected_unique_tasks,
            summary["uniqueTasks"],
        ),
        (
            "projectedPassesIfAllRoutesPass",
            expected_projected_passes,
            projection["projectedPassesIfAllRoutesPass"],
        ),
    ]
    issues = [
        f"{name} expected {expected}, got {actual}"
        for name, expected, actual in checks
        if expected is not None and actual != expected
    ]
    missing_tasks = sorted(set(expected_tasks or ()).difference(unique_tasks))
    if missing_tasks:
        issues.append("missing expected tasks: " + ", ".join(missing_tasks))
    for expectation in expected_owners or ():
        task_name, expected_owner = parse_owner_expectation(expectation)
        owners = owners_by_task.get(task_name, [])
        if expected_owner not in owners:
            actual = ", ".join(owners) if owners else "<missing>"
            issues.append(
                f"{task_name} expected owner {expected_owner}, got {actual}"
            )
    return issues


def task_owners_by_name(report: dict[str, Any]) -> dict[str, list[str]]:
    owners: dict[str, list[str]] = {}
    for campaign in report.get("campaigns", []):
        if not isinstance(campaign, dict):
            continue
        campaign_name = str(campaign.get("campaign") or "")
        for route in campaign.get("routeEntries", []):
            if not isinstance(route, dict):
                continue
            route_name = str(route.get("name") or "")
            owner = f"{campaign_name}/{route_name}"
            tasks = route.get("taskNames")
            if not isinstance(tasks, list):
                continue
            for task in tasks:
                owners.setdefault(str(task), []).append(owner)
    return owners


def parse_owner_expectation(value: str) -> tuple[str, str]:
    if "=" not in value:
        raise ValueError(f"--expect-owner must use TASK=CAMPAIGN/ROUTE: {value}")
    task_name, expected_owner = value.split("=", 1)
    if not task_name or not expected_owner:
        raise ValueError(f"--expect-owner must use TASK=CAMPAIGN/ROUTE: {value}")
    return task_name, expected_owner


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifests", type=Path, nargs="+")
    parser.add_argument(
        "--preset",
        choices=sorted(PRESETS),
        help="Apply a named expectation preset for a known campaign handoff.",
    )
    parser.add_argument(
        "--require-no-overlap",
        action="store_true",
        help="Exit non-zero when any task appears in multiple campaign routes.",
    )
    parser.add_argument(
        "--expect-unique-tasks",
        type=int,
        help="Exit non-zero unless the combined manifest has this unique task count.",
    )
    parser.add_argument(
        "--expect-projected-passes",
        type=int,
        help="Exit non-zero unless the combined projection reaches this pass count.",
    )
    parser.add_argument(
        "--expect-task",
        action="append",
        default=[],
        help="Require a task name to be present in the combined unique task set. Repeatable.",
    )
    parser.add_argument(
        "--expect-owner",
        action="append",
        default=[],
        metavar="TASK=CAMPAIGN/ROUTE",
        help="Require a task to be owned by a specific campaign route. Repeatable.",
    )
    parser.add_argument("--json", dest="json_path", type=Path)
    parser.add_argument("--markdown", type=Path)
    return parser.parse_args()


def apply_preset(args: argparse.Namespace) -> argparse.Namespace:
    if args.preset is None:
        return args
    preset = PRESETS[args.preset]
    args.require_no_overlap = bool(
        args.require_no_overlap or preset.get("require_no_overlap")
    )
    if args.expect_unique_tasks is None:
        args.expect_unique_tasks = preset.get("expect_unique_tasks")
    if args.expect_projected_passes is None:
        args.expect_projected_passes = preset.get("expect_projected_passes")
    args.expect_task = list(dict.fromkeys(
        [*preset.get("expect_tasks", []), *args.expect_task]
    ))
    args.expect_owner = list(dict.fromkeys(
        [*preset.get("expect_owners", []), *args.expect_owner]
    ))
    return args


def main() -> int:
    args = apply_preset(parse_args())
    try:
        report = summarize_campaign_manifests(
            [(path, load_json_object(path)) for path in args.manifests]
        )
    except Exception as exc:
        print(f"summarize_tbench_campaigns: {exc}", file=sys.stderr)
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

    if args.require_no_overlap and report["duplicates"]:
        names = ", ".join(item["taskName"] for item in report["duplicates"])
        print(f"summarize_tbench_campaigns: duplicate campaign tasks: {names}", file=sys.stderr)
        return 1
    try:
        issues = expectation_issues(
            report,
            expected_unique_tasks=args.expect_unique_tasks,
            expected_projected_passes=args.expect_projected_passes,
            expected_tasks=args.expect_task,
            expected_owners=args.expect_owner,
        )
    except ValueError as exc:
        print(f"summarize_tbench_campaigns: {exc}", file=sys.stderr)
        return 2
    if issues:
        print(
            "summarize_tbench_campaigns: expectation mismatch: " + "; ".join(issues),
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
