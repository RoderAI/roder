"""Campaign-summary checks for Harbor pre-eval summaries."""

from __future__ import annotations

import hashlib
from pathlib import Path
from typing import Any

SHA256_HEX_LENGTH = 64
EXPECTED_CAMPAIGN_PRESET = "validated-plus-historical"
EXPECTED_CAMPAIGN_UNIQUE_TASKS = 18
EXPECTED_CAMPAIGN_PROJECTED_PASSES = 68
EXPECTED_CAMPAIGN_DUPLICATE_TASKS = 0
EXPECTED_CAMPAIGN_EXPECT_CAMPAIGNS = (
    "validated-conversions",
    "historical-wins",
)
EXPECTED_CAMPAIGN_EXPECT_ROUTES = (
    "validated-conversions/medium-validated",
    "validated-conversions/xhigh-validated",
    "validated-conversions/xhigh-plan-first",
    "historical-wins/policy-framed",
    "historical-wins/environment-targeted",
)
EXPECTED_CAMPAIGN_EXPECT_TASKS = (
    "password-recovery",
    "qemu-startup",
    "vulnerable-secret",
)
EXPECTED_CAMPAIGN_EXPECT_OWNERS = (
    "password-recovery=historical-wins/policy-framed",
    "qemu-startup=historical-wins/environment-targeted",
    "vulnerable-secret=historical-wins/policy-framed",
)


def validate_campaign_summary(
    issues: list[str],
    checks: dict[str, Any],
    options: dict[str, Any],
    required: bool,
    *,
    expected_campaign_summary: Path | str | None = None,
) -> None:
    check = checks.get("campaignSummary")
    option_path = options.get("campaignSummary")
    if required and not option_path:
        issues.append("required campaign summary did not run")
    if expected_campaign_summary is not None and not path_values_match(
        option_path,
        expected_campaign_summary,
    ):
        issues.append("campaignSummary option path mismatch")
    if check is None:
        if required:
            issues.append("campaignSummary check missing")
        return
    if not isinstance(check, dict):
        issues.append("campaignSummary check missing")
        return
    summary_json = check.get("summaryJson")
    if (
        isinstance(option_path, str)
        and option_path
        and isinstance(summary_json, str)
        and summary_json
        and not path_values_match(summary_json, option_path)
    ):
        issues.append("campaignSummary option and summaryJson mismatch")
    status = str(check.get("status") or "")
    if status != "passed":
        issues.append(f"campaignSummary status is {status or '<missing>'}")
    if check.get("validationStatus") != "ok":
        issues.append(
            f"campaignSummary validationStatus is {check.get('validationStatus') or '<missing>'}"
        )
    check_issues = list_value(check.get("issues"))
    if check_issues:
        issues.append(
            "campaignSummary issues: "
            + "; ".join(str(issue) for issue in check_issues)
        )
    if not isinstance(summary_json, str) or not summary_json:
        issues.append("campaignSummary summaryJson is missing")
    elif expected_campaign_summary is not None and not path_values_match(
        summary_json,
        expected_campaign_summary,
    ):
        issues.append("campaignSummary summaryJson path mismatch")
    validate_summary_json_hash(
        issues,
        check,
        summary_json=summary_json,
        verify_file=expected_campaign_summary is not None,
        missing_issue="campaignSummary summaryJsonSha256 is missing",
        invalid_issue="campaignSummary summaryJsonSha256 is invalid",
        missing_file_issue="campaignSummary summaryJson file is missing",
        mismatch_issue="campaignSummary summaryJsonSha256 mismatch",
    )
    preset = check.get("preset")
    if not isinstance(preset, str) or not preset:
        issues.append("campaignSummary preset is missing")
    elif preset != EXPECTED_CAMPAIGN_PRESET:
        issues.append(
            f"campaignSummary preset is {preset}, expected {EXPECTED_CAMPAIGN_PRESET}"
        )
    validate_expected_campaign_counts(issues, check)
    validate_expected_campaign_no_overlap(issues, check)
    validate_expected_campaign_expectations(issues, check)
    manifests = check.get("manifests")
    if not isinstance(manifests, list) or not manifests:
        issues.append("campaignSummary manifests are missing")
        return
    validate_expected_campaign_manifest_coverage(issues, check)
    verify_manifest_files = expected_campaign_summary is not None
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
        elif verify_manifest_files:
            validate_manifest_hash_binding(
                issues,
                manifest,
                summary_json=summary_json,
                missing_issue="campaignSummary manifest file is missing",
                mismatch_issue="campaignSummary manifestSha256 mismatch",
            )


def validate_manifest_hash_binding(
    issues: list[str],
    manifest: dict[str, Any],
    *,
    summary_json: Path | str | None,
    missing_issue: str,
    mismatch_issue: str,
) -> None:
    manifest_path = manifest.get("manifest")
    manifest_sha = manifest.get("manifestSha256")
    if not isinstance(manifest_path, str) or not manifest_path:
        return
    if not is_sha256_hex(manifest_sha):
        return
    resolved = resolve_manifest_path(manifest_path, summary_json)
    if not resolved.exists():
        issues.append(missing_issue)
        return
    if file_sha256(resolved) != manifest_sha:
        issues.append(mismatch_issue)


def validate_summary_json_hash(
    issues: list[str],
    campaign_summary: dict[str, Any],
    *,
    summary_json: Any,
    verify_file: bool,
    missing_issue: str,
    invalid_issue: str,
    missing_file_issue: str,
    mismatch_issue: str,
) -> None:
    summary_sha = campaign_summary.get("summaryJsonSha256")
    if not isinstance(summary_sha, str) or not summary_sha:
        issues.append(missing_issue)
        return
    if not is_sha256_hex(summary_sha):
        issues.append(invalid_issue)
        return
    if not verify_file:
        return
    if not isinstance(summary_json, str) or not summary_json:
        return
    summary_path = Path(summary_json)
    if not summary_path.exists():
        issues.append(missing_file_issue)
        return
    if file_sha256(summary_path) != summary_sha:
        issues.append(mismatch_issue)


def validate_expected_campaign_counts(
    issues: list[str],
    campaign_summary: dict[str, Any],
) -> None:
    unique_tasks = int_value(campaign_summary.get("uniqueTasks"))
    if unique_tasks <= 0:
        issues.append("campaignSummary uniqueTasks is not positive")
    elif unique_tasks != EXPECTED_CAMPAIGN_UNIQUE_TASKS:
        issues.append(
            "campaignSummary uniqueTasks is "
            f"{unique_tasks}, expected {EXPECTED_CAMPAIGN_UNIQUE_TASKS}"
        )
    projected_passes = int_value(campaign_summary.get("projectedPasses"))
    if projected_passes <= 0:
        issues.append("campaignSummary projectedPasses is not positive")
    elif projected_passes != EXPECTED_CAMPAIGN_PROJECTED_PASSES:
        issues.append(
            "campaignSummary projectedPasses is "
            f"{projected_passes}, expected {EXPECTED_CAMPAIGN_PROJECTED_PASSES}"
        )


def validate_expected_campaign_no_overlap(
    issues: list[str],
    campaign_summary: dict[str, Any],
    *,
    issue_prefix: str = "campaignSummary",
) -> None:
    if "duplicateTasks" not in campaign_summary:
        issues.append(f"{issue_prefix} duplicateTasks is missing")
    else:
        duplicate_tasks = int_value(campaign_summary.get("duplicateTasks"))
        if duplicate_tasks != EXPECTED_CAMPAIGN_DUPLICATE_TASKS:
            issues.append(
                f"{issue_prefix} duplicateTasks is "
                f"{duplicate_tasks}, expected {EXPECTED_CAMPAIGN_DUPLICATE_TASKS}"
            )
    duplicates = campaign_summary.get("duplicates")
    if not isinstance(duplicates, list):
        issues.append(f"{issue_prefix} duplicates are missing")
    elif duplicates:
        issues.append(f"{issue_prefix} duplicates are present")


def validate_expected_campaign_manifest_coverage(
    issues: list[str],
    campaign_summary: dict[str, Any],
    *,
    issue_prefix: str = "campaignSummary",
) -> None:
    campaigns = manifest_campaign_names(campaign_summary)
    expected = sorted(EXPECTED_CAMPAIGN_EXPECT_CAMPAIGNS)
    if sorted(campaigns) != expected:
        issues.append(f"{issue_prefix} campaigns mismatch")


def manifest_campaign_names(campaign_summary: dict[str, Any]) -> list[str]:
    names: list[str] = []
    for manifest in list_value(campaign_summary.get("manifests")):
        if not isinstance(manifest, dict):
            continue
        campaign = manifest.get("campaign")
        if isinstance(campaign, str) and campaign:
            names.append(campaign)
    return names


def validate_expected_campaign_expectations(
    issues: list[str],
    campaign_summary: dict[str, Any],
    *,
    issue_prefix: str = "campaignSummary",
) -> None:
    for field, expected in expected_campaign_expectation_values().items():
        actual = campaign_summary.get(field)
        if actual == expected:
            continue
        if isinstance(expected, list):
            issues.append(f"{issue_prefix} {field} mismatch")
        else:
            issues.append(
                f"{issue_prefix} {field} is {display_value(actual)}, expected {expected}"
            )


def expected_campaign_expectation_values() -> dict[str, Any]:
    return {
        "requireNoOverlap": True,
        "expectUniqueTasks": EXPECTED_CAMPAIGN_UNIQUE_TASKS,
        "expectProjectedPasses": EXPECTED_CAMPAIGN_PROJECTED_PASSES,
        "expectCampaigns": list(EXPECTED_CAMPAIGN_EXPECT_CAMPAIGNS),
        "expectRoutes": list(EXPECTED_CAMPAIGN_EXPECT_ROUTES),
        "expectTasks": list(EXPECTED_CAMPAIGN_EXPECT_TASKS),
        "expectOwners": list(EXPECTED_CAMPAIGN_EXPECT_OWNERS),
    }


def display_value(value: Any) -> str:
    return "<missing>" if value is None else str(value)


def resolve_manifest_path(manifest_path: str, summary_json: Path | str | None) -> Path:
    path = Path(manifest_path)
    if path.is_absolute() or path.exists() or summary_json is None:
        return path
    summary_path = Path(summary_json)
    return summary_path.parent / path


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def path_values_match(actual: Any, expected: Path | str) -> bool:
    if not isinstance(actual, str) or not actual:
        return False
    expected_text = str(expected)
    if actual == expected_text:
        return True
    return normalized_path(actual) == normalized_path(expected_text)


def normalized_path(value: str) -> Path:
    return Path(value).expanduser().resolve(strict=False)


def is_sha256_hex(value: Any) -> bool:
    return (
        isinstance(value, str)
        and len(value) == SHA256_HEX_LENGTH
        and all(char in "0123456789abcdefABCDEF" for char in value)
    )


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0
