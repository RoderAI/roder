"""Validate launch-plan fields copied from a pre-eval summary."""

from __future__ import annotations

from typing import Any


def validate_plan_copies_match_summary(
    issues: list[str],
    plan: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    summary_config_sha = summary_config_hash(summary, plan.get("harborConfig"))
    if (
        isinstance(plan.get("harborConfig"), str)
        and plan.get("harborConfig")
        and summary_config_sha is None
    ):
        issues.append(
            "pre-eval summary missing Harbor config entry: " + plan["harborConfig"]
        )
    if (
        isinstance(plan.get("preEvalHarborConfigSha256"), str)
        and summary_config_sha is not None
        and plan.get("preEvalHarborConfigSha256") != summary_config_sha
    ):
        issues.append("pre-eval Harbor config summary SHA-256 mismatch")
    validate_prebuilt_copy(issues, plan, summary)
    validate_auth_copy(issues, plan, summary)
    validate_harness_copy(issues, plan, summary)
    validate_harness_tests_copy(issues, plan, summary)
    validate_deadline_policy_copy(issues, plan, summary)
    validate_image_preflight_copy(issues, plan, summary)
    validate_campaign_summary_copy(issues, plan, summary)


def validate_prebuilt_copy(
    issues: list[str],
    plan: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    summary_prebuilt = dict_value(summary.get("prebuiltBinary"))
    plan_prebuilt = dict_value(plan.get("prebuiltBinary"))
    if not plan_prebuilt or not summary_prebuilt:
        return
    if plan_prebuilt.get("path") != summary_prebuilt.get("path"):
        issues.append("prebuilt binary summary path mismatch")
    if plan_prebuilt.get("sha256") != summary_prebuilt.get("sha256"):
        issues.append("prebuilt binary summary SHA-256 mismatch")
    for field in ("executable", "linuxX8664Elf"):
        if (
            field in plan_prebuilt
            and field in summary_prebuilt
            and plan_prebuilt.get(field) != summary_prebuilt.get(field)
        ):
            issues.append(f"prebuilt binary summary {field} mismatch")
    for field in ("sizeBytes", "modifiedAt", "fileType"):
        if (
            field in plan_prebuilt or field in summary_prebuilt
        ) and plan_prebuilt.get(field) != summary_prebuilt.get(field):
            issues.append(f"prebuilt binary summary {field} mismatch")


def validate_auth_copy(
    issues: list[str],
    plan: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    summary_auth = dict_value(summary.get("authFile"))
    plan_auth = dict_value(plan.get("authFile"))
    if not plan_auth or not summary_auth:
        return
    if plan_auth.get("path") != summary_auth.get("path"):
        issues.append("auth file summary path mismatch")
    if plan_auth.get("validJson") != summary_auth.get("validJson"):
        issues.append("auth file summary validJson mismatch")
    if list_value(plan_auth.get("jsonFields")) != list_value(summary_auth.get("jsonFields")):
        issues.append("auth file summary fields mismatch")
    for field in ("sizeBytes", "modifiedAt"):
        if (
            field in plan_auth or field in summary_auth
        ) and plan_auth.get(field) != summary_auth.get(field):
            issues.append(f"auth file summary {field} mismatch")


def validate_harness_copy(
    issues: list[str],
    plan: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    summary_harness = dict_value(
        dict_value(summary.get("checks")).get("harborHarness")
    )
    plan_harness = dict_value(plan.get("harborHarness"))
    if not plan_harness or not summary_harness:
        return
    if plan_harness.get("status") != summary_harness.get("status"):
        issues.append("harbor harness summary status mismatch")
    if plan_harness.get("combinedSha256") != summary_harness.get("combinedSha256"):
        issues.append("harbor harness summary combined SHA-256 mismatch")
    if plan_harness.get("files") != summary_harness.get("files"):
        issues.append("harbor harness summary files mismatch")
    if list_value(plan_harness.get("entries")) != list_value(summary_harness.get("entries")):
        issues.append("harbor harness summary entries mismatch")
    if list_value(plan_harness.get("issues")) != list_value(summary_harness.get("issues")):
        issues.append("harbor harness summary issues mismatch")


def validate_harness_tests_copy(
    issues: list[str],
    plan: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    summary_harness_tests = dict_value(
        dict_value(summary.get("checks")).get("harborHarnessTests")
    )
    plan_harness_tests = dict_value(plan.get("harborHarnessTests"))
    if not summary_harness_tests:
        return
    if not plan_harness_tests:
        issues.append("harbor harness tests summary status mismatch")
    elif plan_harness_tests.get("status") != summary_harness_tests.get("status"):
        issues.append("harbor harness tests summary status mismatch")


def validate_deadline_policy_copy(
    issues: list[str],
    plan: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    summary_policy = dict_value(
        dict_value(dict_value(summary.get("checks")).get("harborConfigs")).get(
            "deadlinePolicy"
        )
    )
    if not summary_policy:
        return
    if dict_value(plan.get("deadlinePolicy")) != summary_policy:
        issues.append("deadline policy summary mismatch")


def validate_image_preflight_copy(
    issues: list[str],
    plan: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    summary_image_preflight = dict_value(
        dict_value(summary.get("checks")).get("imagePreflight")
    )
    plan_image_preflight = dict_value(plan.get("imagePreflight"))
    if not plan_image_preflight or not summary_image_preflight:
        return
    for field in (
        "status",
        "config",
        "manifest",
        "tasks",
        "uniqueImages",
        "present",
        "missing",
        "unresolved",
        "pullFailed",
        "selectionErrors",
        "blockedTasks",
    ):
        if (
            field in plan_image_preflight or field in summary_image_preflight
        ) and plan_image_preflight.get(field) != summary_image_preflight.get(field):
            issues.append(f"image preflight summary {field} mismatch")


def validate_campaign_summary_copy(
    issues: list[str],
    plan: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    summary_campaign = dict_value(
        dict_value(summary.get("checks")).get("campaignSummary")
    )
    plan_campaign = dict_value(plan.get("campaignSummary"))
    if not summary_campaign:
        return
    if not plan_campaign:
        issues.append("campaign summary status mismatch")
        return
    for field in (
        "status",
        "summaryJson",
        "summaryJsonSha256",
        "preset",
        "validationStatus",
        "issues",
        "uniqueTasks",
        "projectedPasses",
        "duplicateTasks",
        "duplicates",
        "requireNoOverlap",
        "expectUniqueTasks",
        "expectProjectedPasses",
        "expectCampaigns",
        "expectRoutes",
        "expectTasks",
        "expectOwners",
        "manifests",
    ):
        if (
            field in plan_campaign or field in summary_campaign
        ) and plan_campaign.get(field) != summary_campaign.get(field):
            issues.append(f"campaign summary {field} mismatch")


def summary_config_hash(summary: dict[str, Any], path: Any) -> str | None:
    if not isinstance(path, str) or not path:
        return None
    checks = summary.get("checks") if isinstance(summary.get("checks"), dict) else {}
    configs = checks.get("harborConfigs") if isinstance(checks, dict) else {}
    entries = configs.get("entries") if isinstance(configs, dict) else None
    if not isinstance(entries, list):
        return None
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        if entry.get("path") == path and isinstance(entry.get("sha256"), str):
            return entry.get("sha256")
    return None


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []
