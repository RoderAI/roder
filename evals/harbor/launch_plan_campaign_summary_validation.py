"""Validate campaign-summary fields copied into a Harbor launch plan."""

from __future__ import annotations

from typing import Any

from pre_eval_campaign_summary_validation import (
    EXPECTED_CAMPAIGN_PRESET,
    is_sha256_hex,
    validate_expected_campaign_counts,
    validate_expected_campaign_expectations,
    validate_expected_campaign_manifest_coverage,
    validate_expected_campaign_no_overlap,
    validate_summary_json_hash,
)


def validate_campaign_summary(
    issues: list[str],
    plan: dict[str, Any],
    *,
    required: bool = False,
) -> None:
    if required and plan.get("requireCampaignSummary") is not True:
        issues.append("required campaign summary is not enabled")
    campaign_summary = plan.get("campaignSummary")
    if not isinstance(campaign_summary, dict) or not campaign_summary:
        if required or plan.get("requireCampaignSummary") is True:
            issues.append("campaignSummary is missing")
        return
    status = str(campaign_summary.get("status") or "")
    if status != "passed":
        issues.append(f"campaignSummary status is {status or '<missing>'}")
    if campaign_summary.get("validationStatus") != "ok":
        issues.append(
            "campaignSummary validationStatus is "
            f"{campaign_summary.get('validationStatus') or '<missing>'}"
        )
    if (
        not isinstance(campaign_summary.get("summaryJson"), str)
        or not campaign_summary.get("summaryJson")
    ):
        issues.append("campaignSummary summaryJson is missing")
    validate_summary_json_hash(
        issues,
        campaign_summary,
        summary_json=campaign_summary.get("summaryJson"),
        verify_file=False,
        missing_issue="campaignSummary summaryJsonSha256 is missing",
        invalid_issue="campaignSummary summaryJsonSha256 is invalid",
        missing_file_issue="campaignSummary summaryJson file is missing",
        mismatch_issue="campaignSummary summaryJsonSha256 mismatch",
    )
    preset = campaign_summary.get("preset")
    if not isinstance(preset, str) or not preset:
        issues.append("campaignSummary preset is missing")
    elif preset != EXPECTED_CAMPAIGN_PRESET:
        issues.append(
            f"campaignSummary preset is {preset}, expected {EXPECTED_CAMPAIGN_PRESET}"
        )
    validate_expected_campaign_counts(issues, campaign_summary)
    validate_expected_campaign_no_overlap(issues, campaign_summary)
    validate_expected_campaign_expectations(issues, campaign_summary)
    check_issues = list_value(campaign_summary.get("issues"))
    if check_issues:
        issues.append(
            "campaignSummary issues: "
            + "; ".join(str(issue) for issue in check_issues)
        )
    manifests = campaign_summary.get("manifests")
    if not isinstance(manifests, list) or not manifests:
        issues.append("campaignSummary manifests are missing")
        return
    validate_expected_campaign_manifest_coverage(issues, campaign_summary)
    for manifest in manifests:
        if not isinstance(manifest, dict):
            issues.append("campaignSummary manifest entry must be an object")
            continue
        if not isinstance(manifest.get("manifest"), str) or not manifest.get("manifest"):
            issues.append("campaignSummary manifest path is missing")
        if (
            not isinstance(manifest.get("manifestSha256"), str)
            or not manifest.get("manifestSha256")
        ):
            issues.append("campaignSummary manifestSha256 is missing")
        elif not is_sha256_hex(manifest.get("manifestSha256")):
            issues.append("campaignSummary manifestSha256 is invalid")


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []
