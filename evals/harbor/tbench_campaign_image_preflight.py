"""Image-preflight validation for generated Harbor Terminal-Bench campaigns."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Protocol

from pre_eval_image_preflight_validation import (
    validate_image_manifest_file,
    validate_image_preflight_clean_details,
    validate_image_preflight_evidence,
)


class IssueSink(Protocol):
    def add(self, issue: str) -> None: ...


def validate_route_image_preflight(
    result: IssueSink,
    name: str,
    *,
    route: dict[str, Any],
    config_path: str,
    preflight_dir: Path,
) -> None:
    manifest_path = route.get("imageManifest")
    if isinstance(manifest_path, str) and manifest_path:
        path = Path(manifest_path)
    else:
        path = preflight_dir / f"{name}-images.json"
    try:
        manifest = load_json(path)
    except Exception as exc:
        result.add(f"route {name} image preflight manifest cannot be read: {exc}")
        return
    if manifest.get("clean") is not True:
        result.add(f"route {name} image preflight is not clean")
    if manifest.get("config") != config_path:
        result.add(f"route {name} image preflight config mismatch")
    summary = dict_value(manifest.get("summary"))
    if int_value(summary.get("tasks")) != int_value(route.get("taskCount")):
        result.add(f"route {name} image preflight task count mismatch")
    for metric in ("missing", "unresolved", "pull_failed"):
        if int_value(summary.get(metric)) != 0:
            result.add(f"route {name} image preflight has {metric}={summary.get(metric)}")
    if list_value(manifest.get("selection_errors")):
        result.add(f"route {name} image preflight has selection errors")
    validate_route_manifest_task_names(result, name, route=route, manifest=manifest)
    validate_route_image_manifest_details(
        result,
        name,
        path=path,
        config_path=config_path,
        manifest=manifest,
        summary=summary,
    )


def validate_route_manifest_task_names(
    result: IssueSink,
    name: str,
    *,
    route: dict[str, Any],
    manifest: dict[str, Any],
) -> None:
    tasks = manifest.get("tasks")
    if not isinstance(tasks, list):
        return
    actual = sorted(
        str(task.get("task_name") or "")
        for task in tasks
        if isinstance(task, dict)
    )
    expected = sorted(str(task) for task in list_value(route.get("tasks")))
    if actual != expected:
        result.add(f"route {name} imagePreflight manifest task names mismatch")


def validate_route_image_manifest_details(
    result: IssueSink,
    name: str,
    *,
    path: Path,
    config_path: str,
    manifest: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    image_preflight = {
        "manifest": str(path),
        "config": config_path,
        "tasks": int_value(summary.get("tasks")),
        "uniqueImages": int_value(summary.get("unique_images")),
        "present": int_value(summary.get("present")),
        "missing": int_value(summary.get("missing")),
        "unresolved": int_value(summary.get("unresolved")),
        "pullFailed": int_value(summary.get("pull_failed")),
        "selectionErrors": list_value(manifest.get("selection_errors")),
        "blockedTasks": [],
    }
    if "offline" in manifest:
        image_preflight["offline"] = manifest.get("offline")
    if "pull" in manifest:
        image_preflight["pull"] = manifest.get("pull")
    manifest_issues: list[str] = []
    validate_image_preflight_evidence(manifest_issues, image_preflight)
    validate_image_preflight_clean_details(manifest_issues, image_preflight)
    validate_image_manifest_file(manifest_issues, image_preflight)
    for issue in manifest_issues:
        result.add(f"route {name} {issue}")


def load_json(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text())
    if not isinstance(data, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return data


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0
