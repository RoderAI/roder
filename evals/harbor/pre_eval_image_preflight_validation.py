"""Validation helpers for Harbor image-preflight handoff evidence."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

TERMINAL_BENCH_V2_TASKS = 89


def validate_image_preflight(
    issues: list[str],
    checks: dict[str, Any],
    options: dict[str, Any],
    required: bool,
    *,
    verify_manifest: bool = False,
    required_config: str | None = None,
) -> None:
    if not required:
        return
    if options.get("preflightImages") is not True:
        issues.append("required image preflight did not run")
    require_check_status(issues, checks, "imagePreflight", {"passed"})
    image_preflight = checks.get("imagePreflight")
    if not isinstance(image_preflight, dict):
        return
    requested_config = options.get("imageConfig")
    preflight_config = image_preflight.get("config")
    if not isinstance(requested_config, str) or not requested_config:
        issues.append("required image preflight config is missing")
    elif preflight_config != requested_config:
        issues.append("image preflight config does not match requested config")
    if required_config and preflight_config != required_config:
        issues.append("image preflight config does not match required config")
    if options.get("offlineImages") is True and image_preflight.get("offline") is not True:
        issues.append("image preflight did not run in offline mode")
    validate_image_preflight_evidence(issues, image_preflight)
    validate_image_preflight_clean_details(issues, image_preflight)
    if verify_manifest:
        validate_image_manifest_file(issues, image_preflight)


def validate_image_manifest_file(
    issues: list[str],
    image_preflight: dict[str, Any],
) -> None:
    manifest_path = image_preflight.get("manifest")
    if not isinstance(manifest_path, str) or not manifest_path:
        issues.append("imagePreflight manifest is missing")
        return
    try:
        manifest = json.loads(Path(manifest_path).read_text())
    except OSError as exc:
        issues.append(f"imagePreflight manifest cannot be read: {exc}")
        return
    except json.JSONDecodeError as exc:
        issues.append(f"imagePreflight manifest is invalid: {exc}")
        return
    if not isinstance(manifest, dict):
        issues.append("imagePreflight manifest must be a JSON object")
        return
    if manifest.get("clean") is not True:
        issues.append("imagePreflight manifest is not clean")
    config = manifest.get("config")
    if config != image_preflight.get("config"):
        issues.append("imagePreflight manifest config mismatch")
    if manifest.get("offline") is not image_preflight.get("offline"):
        issues.append("imagePreflight manifest offline mismatch")
    summary = manifest.get("summary")
    if not isinstance(summary, dict):
        issues.append("imagePreflight manifest summary is missing")
        return
    validate_image_manifest_config_task_count(issues, manifest, summary)
    validate_image_manifest_rows(issues, manifest, summary)
    validate_image_manifest_row_identities(issues, manifest)
    validate_image_manifest_status_counts(issues, manifest, summary)
    validate_image_manifest_counts(issues, image_preflight, summary)
    validate_image_manifest_lists(issues, image_preflight, manifest)


def validate_image_manifest_config_task_count(
    issues: list[str],
    manifest: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    expected = expected_task_count_for_config(manifest.get("config"), issues)
    if expected is None:
        return
    actual = int_value(summary.get("tasks"))
    if actual != expected:
        issues.append(
            "imagePreflight manifest task count does not match config: "
            f"expected {expected}, got {actual}"
        )


def expected_task_count_for_config(config_path: Any, issues: list[str]) -> int | None:
    if not isinstance(config_path, str) or not config_path:
        return None
    try:
        config = json.loads(Path(config_path).read_text())
    except OSError as exc:
        issues.append(f"imagePreflight config cannot be read: {exc}")
        return None
    except json.JSONDecodeError as exc:
        issues.append(f"imagePreflight config is invalid: {exc}")
        return None
    if not isinstance(config, dict):
        return None
    datasets = config.get("datasets")
    if not isinstance(datasets, list):
        return None
    total = 0
    has_expected_scope = False
    for dataset in datasets:
        if not isinstance(dataset, dict):
            continue
        task_names = dataset.get("task_names")
        if isinstance(task_names, list) and task_names:
            total += len(task_names)
            has_expected_scope = True
        elif (
            dataset.get("name") == "terminal-bench"
            and str(dataset.get("version") or "") == "2.0"
        ):
            total += TERMINAL_BENCH_V2_TASKS
            has_expected_scope = True
    return total if has_expected_scope else None


def validate_image_manifest_rows(
    issues: list[str],
    manifest: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    tasks = manifest.get("tasks")
    if not isinstance(tasks, list):
        issues.append("imagePreflight manifest task rows are missing")
    elif len(tasks) != int_value(summary.get("tasks")):
        issues.append("imagePreflight manifest task rows mismatch")
    images = manifest.get("images")
    if not isinstance(images, list):
        issues.append("imagePreflight manifest image rows are missing")
    elif len(images) != int_value(summary.get("unique_images")):
        issues.append("imagePreflight manifest image rows mismatch")


def validate_image_manifest_counts(
    issues: list[str],
    image_preflight: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    manifest_field_map = {
        "tasks": "tasks",
        "uniqueImages": "unique_images",
        "present": "present",
        "missing": "missing",
        "unresolved": "unresolved",
        "pullFailed": "pull_failed",
    }
    for image_field, manifest_field in manifest_field_map.items():
        manifest_value = int_value(summary.get(manifest_field))
        image_value = int_value(image_preflight.get(image_field))
        if manifest_value != image_value:
            issues.append(f"imagePreflight manifest {image_field} mismatch")


def validate_image_manifest_row_identities(
    issues: list[str],
    manifest: dict[str, Any],
) -> None:
    tasks = manifest.get("tasks")
    if isinstance(tasks, list):
        task_names = [
            str(task.get("task_name") or "")
            for task in tasks
            if isinstance(task, dict)
        ]
        if any(not name for name in task_names):
            issues.append("imagePreflight manifest task rows have missing names")
        if len(set(task_names)) != len(task_names):
            issues.append("imagePreflight manifest task rows are not unique")
    images = manifest.get("images")
    if isinstance(images, list):
        image_names = [
            str(image.get("image") or "")
            for image in images
            if isinstance(image, dict)
        ]
        if any(not name for name in image_names):
            issues.append("imagePreflight manifest image rows have missing names")
        if len(set(image_names)) != len(image_names):
            issues.append("imagePreflight manifest image rows are not unique")
    validate_image_manifest_task_image_set(issues, tasks, images)
    validate_image_manifest_image_task_mapping(issues, tasks, images)


def validate_image_manifest_task_image_set(
    issues: list[str],
    tasks: Any,
    images: Any,
) -> None:
    if not isinstance(tasks, list) or not isinstance(images, list):
        return
    task_images = {
        str(task.get("image") or "")
        for task in tasks
        if isinstance(task, dict) and task.get("image")
    }
    image_names = {
        str(image.get("image") or "")
        for image in images
        if isinstance(image, dict) and image.get("image")
    }
    if task_images != image_names:
        issues.append("imagePreflight manifest task images do not match image rows")


def validate_image_manifest_image_task_mapping(
    issues: list[str],
    tasks: Any,
    images: Any,
) -> None:
    if not isinstance(tasks, list) or not isinstance(images, list):
        return
    expected: dict[str, list[str]] = {}
    for task in tasks:
        if not isinstance(task, dict):
            continue
        image = str(task.get("image") or "")
        task_name = str(task.get("task_name") or "")
        if image and task_name:
            expected.setdefault(image, []).append(task_name)

    saw_mismatch = False
    for image_row in images:
        if not isinstance(image_row, dict):
            continue
        image = str(image_row.get("image") or "")
        if not image:
            continue
        row_tasks = image_row.get("tasks")
        if not isinstance(row_tasks, list):
            saw_mismatch = True
            continue
        actual = sorted(str(task) for task in row_tasks)
        if actual != sorted(expected.get(image, [])):
            saw_mismatch = True
    if saw_mismatch:
        issues.append("imagePreflight manifest image task mapping mismatch")


def validate_image_manifest_status_counts(
    issues: list[str],
    manifest: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    tasks = manifest.get("tasks")
    if not isinstance(tasks, list):
        return
    statuses = [
        str(task.get("status") or "")
        for task in tasks
        if isinstance(task, dict)
    ]
    status_counts = {
        "present": sum(1 for status in statuses if status in {"present", "pulled"}),
        "missing": sum(1 for status in statuses if status == "missing"),
        "unresolved": sum(1 for status in statuses if status == "unresolved")
        + len(selection_errors(manifest)),
        "pull_failed": sum(1 for status in statuses if status == "pull_failed"),
    }
    for field, actual in status_counts.items():
        if actual != int_value(summary.get(field)):
            label = "pullFailed" if field == "pull_failed" else field
            issues.append(f"imagePreflight manifest {label} status count mismatch")


def validate_image_manifest_lists(
    issues: list[str],
    image_preflight: dict[str, Any],
    manifest: dict[str, Any],
) -> None:
    if selection_errors(manifest) != list_value(image_preflight.get("selectionErrors")):
        issues.append("imagePreflight manifest selectionErrors mismatch")
    if blocked_image_tasks(manifest) != list_value(image_preflight.get("blockedTasks")):
        issues.append("imagePreflight manifest blockedTasks mismatch")


def validate_image_preflight_evidence(
    issues: list[str],
    image_preflight: dict[str, Any],
) -> None:
    manifest = image_preflight.get("manifest")
    if not isinstance(manifest, str) or not manifest:
        issues.append("imagePreflight manifest is missing")
    tasks = positive_image_preflight_count(issues, image_preflight, "tasks")
    unique_images = positive_image_preflight_count(
        issues,
        image_preflight,
        "uniqueImages",
    )
    present = positive_image_preflight_count(issues, image_preflight, "present")
    if (
        tasks is not None
        and present is not None
        and present > tasks
    ):
        issues.append("imagePreflight present exceeds tasks")
    if (
        tasks is not None
        and unique_images is not None
        and unique_images > tasks
    ):
        issues.append("imagePreflight uniqueImages exceeds tasks")


def positive_image_preflight_count(
    issues: list[str],
    image_preflight: dict[str, Any],
    field: str,
) -> int | None:
    if field not in image_preflight:
        issues.append(f"imagePreflight {field} is missing")
        return None
    value = int_value(image_preflight.get(field))
    if value <= 0:
        issues.append(f"imagePreflight {field} is not positive")
    return value


def validate_image_preflight_clean_details(
    issues: list[str],
    image_preflight: dict[str, Any],
) -> None:
    blocked_count_fields_clean = True
    for field in ("missing", "unresolved", "pullFailed"):
        if field not in image_preflight:
            issues.append(f"imagePreflight {field} is missing")
            blocked_count_fields_clean = False
            continue
        value = int_value(image_preflight.get(field))
        if value != 0:
            issues.append(f"imagePreflight {field} is {value}")
            blocked_count_fields_clean = False
    validate_image_preflight_empty_list(issues, image_preflight, "selectionErrors")
    validate_image_preflight_empty_list(issues, image_preflight, "blockedTasks")
    if not blocked_count_fields_clean:
        return
    tasks = int_value(image_preflight.get("tasks"))
    present = int_value(image_preflight.get("present"))
    if tasks > 0 and present > 0 and present != tasks:
        issues.append("imagePreflight present does not cover all tasks")


def validate_image_preflight_empty_list(
    issues: list[str],
    image_preflight: dict[str, Any],
    field: str,
) -> None:
    value = image_preflight.get(field)
    if field not in image_preflight:
        issues.append(f"imagePreflight {field} is missing")
    elif not isinstance(value, list):
        issues.append(f"imagePreflight {field} is not a list")
    elif value and field == "selectionErrors":
        issues.append("imagePreflight selectionErrors are non-empty")
    elif value and field == "blockedTasks":
        issues.append("imagePreflight blockedTasks are non-empty")


def selection_errors(manifest: dict[str, Any]) -> list[str]:
    errors = manifest.get("selection_errors")
    if not isinstance(errors, list):
        return []
    return [str(error) for error in errors]


def blocked_image_tasks(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    tasks = manifest.get("tasks")
    if not isinstance(tasks, list):
        return []
    blocked: list[dict[str, Any]] = []
    for task in tasks:
        if not isinstance(task, dict):
            continue
        status = str(task.get("status") or "")
        if status not in {"missing", "unresolved", "pull_failed"}:
            continue
        blocked.append(
            {
                "taskName": str(task.get("task_name") or ""),
                "status": status,
                "image": task.get("image"),
                "imageSource": task.get("image_source"),
            }
        )
    return blocked


def require_check_status(
    issues: list[str],
    checks: dict[str, Any],
    name: str,
    allowed: set[str],
) -> None:
    check = checks.get(name)
    if not isinstance(check, dict):
        issues.append(f"{name} check missing")
        return
    status = str(check.get("status") or "")
    if status not in allowed:
        issues.append(f"{name} status is {status or '<missing>'}")


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0
