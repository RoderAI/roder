"""Summarize combined campaign handoffs for pre-eval diagnostics."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from pre_eval_campaign_summary_validation import (
    EXPECTED_CAMPAIGN_PROJECTED_PASSES,
    EXPECTED_CAMPAIGN_PRESET,
    EXPECTED_CAMPAIGN_UNIQUE_TASKS,
    expected_campaign_expectation_values,
    file_sha256,
    int_value,
    is_sha256_hex,
    validate_expected_campaign_expectations,
    validate_expected_campaign_manifest_coverage,
    validate_manifest_hash_binding,
)


def campaign_summary_check(path: Path) -> dict[str, Any]:
    base = {
        "summaryJson": str(path),
        "summaryJsonSha256": None,
        "preset": None,
        "validationStatus": None,
        "issues": [],
        "uniqueTasks": 0,
        "projectedPasses": 0,
        "duplicateTasks": 0,
        "duplicates": [],
        "manifests": [],
    }
    base.update({field: None for field in expected_campaign_expectation_values()})
    if not path.exists():
        return {"status": "missing", **base}
    try:
        report = json.loads(path.read_text())
    except Exception as exc:
        return {"status": "failed", "error": str(exc), **base}
    if not isinstance(report, dict):
        return {"status": "failed", "error": "campaign summary must be a JSON object", **base}

    validation = dict_value(report.get("validation"))
    summary = dict_value(report.get("summary"))
    projection = dict_value(report.get("scoreProjection"))
    manifests = campaign_summary_manifests(report)
    issues = [str(issue) for issue in validation.get("issues", []) if str(issue)]
    unique_tasks = int_value(summary.get("uniqueTasks"))
    projected_passes = int_value(projection.get("projectedPassesIfAllRoutesPass"))
    duplicate_tasks = (
        int_value(summary.get("duplicateTasks"))
        if "duplicateTasks" in summary
        else None
    )
    duplicates_value = report.get("duplicates")
    duplicates = duplicates_value if isinstance(duplicates_value, list) else []
    if not manifests:
        issues.append("campaign manifests are missing")
    if any(not item.get("manifestSha256") for item in manifests):
        issues.append("campaign manifest SHA-256 is missing")
    if any(
        item.get("manifestSha256") and not is_sha256_hex(item.get("manifestSha256"))
        for item in manifests
    ):
        issues.append("campaign manifest SHA-256 is invalid")
    for manifest in manifests:
        validate_manifest_hash_binding(
            issues,
            manifest,
            summary_json=path,
            missing_issue="campaign manifest file is missing",
            mismatch_issue="campaign manifest SHA-256 mismatch",
        )
    validation_status = validation.get("status")
    preset = validation.get("preset")
    if not isinstance(preset, str) or not preset:
        issues.append("campaign preset is missing")
    elif preset != EXPECTED_CAMPAIGN_PRESET:
        issues.append(
            f"campaign preset is {preset}, expected {EXPECTED_CAMPAIGN_PRESET}"
        )
    else:
        validate_expected_campaign_expectations(
            issues,
            validation,
            issue_prefix="campaign",
        )
        validate_expected_campaign_manifest_coverage(
            issues,
            {"manifests": manifests},
            issue_prefix="campaign",
        )
        if unique_tasks != EXPECTED_CAMPAIGN_UNIQUE_TASKS:
            issues.append(
                "campaign uniqueTasks expected "
                f"{EXPECTED_CAMPAIGN_UNIQUE_TASKS}, got {unique_tasks}"
            )
        if projected_passes != EXPECTED_CAMPAIGN_PROJECTED_PASSES:
            issues.append(
                "campaign projectedPasses expected "
                f"{EXPECTED_CAMPAIGN_PROJECTED_PASSES}, got {projected_passes}"
            )
        if duplicate_tasks is None:
            issues.append("campaign duplicateTasks is missing")
        elif duplicate_tasks != 0:
            issues.append(f"campaign duplicateTasks expected 0, got {duplicate_tasks}")
        if not isinstance(duplicates_value, list):
            issues.append("campaign duplicates are missing")
        elif duplicates:
            issues.append("campaign duplicates are present")
    if validation_status != "ok" and not issues:
        issues.append(f"campaign validation status is {validation_status or '<missing>'}")
    return {
        **base,
        "status": "passed" if validation_status == "ok" and not issues else "failed",
        "summaryJsonSha256": file_sha256(path),
        "preset": preset,
        "validationStatus": validation_status,
        "issues": issues,
        "uniqueTasks": unique_tasks,
        "projectedPasses": projected_passes,
        "duplicateTasks": duplicate_tasks if duplicate_tasks is not None else 0,
        "duplicates": duplicates,
        **{
            field: validation.get(field)
            for field in expected_campaign_expectation_values()
        },
        "manifests": manifests,
    }


def campaign_summary_manifests(report: dict[str, Any]) -> list[dict[str, Any]]:
    campaigns = report.get("campaigns")
    if not isinstance(campaigns, list):
        return []
    manifests: list[dict[str, Any]] = []
    for campaign in campaigns:
        if not isinstance(campaign, dict):
            continue
        entry: dict[str, Any] = {
            "campaign": campaign.get("campaign"),
            "manifest": campaign.get("manifest"),
        }
        if "manifestSha256" in campaign:
            entry["manifestSha256"] = campaign.get("manifestSha256")
        manifests.append(entry)
    return manifests


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}
