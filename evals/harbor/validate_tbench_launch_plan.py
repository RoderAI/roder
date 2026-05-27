#!/usr/bin/env python3
"""Validate a Harbor full-run launch-plan artifact."""

from __future__ import annotations

import argparse
import json
import sys
from datetime import datetime
from pathlib import Path
from typing import Any

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from pre_eval_live_checks import (  # noqa: E402
    combined_file_digest,
    file_sha256,
    validate_auth_file as validate_auth_file_summary,
    validate_harbor_harness_files,
    validate_prebuilt_file,
)
from pre_eval_harness_summary import DEFAULT_HARNESS_FILES  # noqa: E402
from pre_eval_image_preflight_validation import (  # noqa: E402
    validate_image_preflight_clean_details,
    validate_image_preflight_evidence,
    validate_image_manifest_file,
)
from launch_plan_dependency_validation import (  # noqa: E402
    validate_config_hashes_match,
    validate_required_dependency_snapshots,
    validate_required_sha256,
)
from launch_plan_campaign_summary_validation import validate_campaign_summary  # noqa: E402
from launch_plan_summary_copies import validate_plan_copies_match_summary  # noqa: E402
from tbench_deadline_policy import validate_deadline_policy  # noqa: E402
from validate_pre_eval_summary import validate_summary as validate_pre_eval_summary  # noqa: E402

VALID_STATUSES = {"dry_run", "ready", "blocked"}
REQUIRED_TEXT_FIELDS = (
    "harborConfig",
    "jobDir",
    "analysisJson",
    "analysisMarkdown",
    "preEvalSummary",
)
REQUIRED_SHA256_FIELDS = (
    "harborConfigSha256",
    "preEvalHarborConfigSha256",
    "preEvalSummarySha256",
)
VALID_IMAGE_PREFLIGHT_SOURCES = {"pre_eval_summary", "route_manifest"}


class ValidationResult:
    def __init__(self, *, ok: bool, issues: list[str]) -> None:
        self.ok = ok
        self.issues = issues


def load_json(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text())
    if not isinstance(data, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return data


def validate_plan(
    plan: dict[str, Any],
    *,
    require_ready: bool = False,
    allow_dry_run: bool = False,
    require_image_preflight: bool = False,
    verify_pre_eval_summary: bool = False,
    verify_harbor_config: bool = False,
    verify_prebuilt_binary: bool = False,
    verify_auth_file: bool = False,
    verify_harness_files: bool = False,
    verify_image_manifest: bool = False,
    require_campaign_summary: bool = False,
    max_pre_eval_age_seconds: int | None = None,
    now: datetime | None = None,
) -> ValidationResult:
    issues: list[str] = []
    status = str(plan.get("launchStatus") or "")
    blocked_reasons = list_value(plan.get("blockedReasons"))

    if status not in VALID_STATUSES:
        issues.append(f"launchStatus is {status or '<missing>'}")
    if require_ready and status != "ready":
        issues.append(f"launchStatus is {status or '<missing>'}, expected ready")
    if status == "dry_run" and not allow_dry_run and not require_ready:
        issues.append("launchStatus is dry_run; pass --allow-dry-run to accept it")
    if status == "dry_run" and plan.get("dryRun") is not True:
        issues.append("dry_run launch plan dryRun is not true")
    if status == "dry_run" and plan.get("wouldRunHarbor") is True:
        issues.append("dry_run launch plan would run Harbor")
    if blocked_reasons:
        issues.append("launch blocked: " + ", ".join(str(item) for item in blocked_reasons))
    if status == "blocked" and not blocked_reasons:
        issues.append("blocked launch plan has no blockedReasons")
    if status == "ready" and plan.get("wouldRunHarbor") is not True:
        issues.append("ready launch plan would not run Harbor")
    if status == "blocked" and plan.get("wouldRunHarbor") is True:
        issues.append("blocked launch plan would still run Harbor")
    validate_deadline_policy(
        issues,
        plan.get("deadlinePolicy"),
        issue_prefix="deadlinePolicy",
    )
    if require_image_preflight and plan.get("requireImagePreflight") is not True:
        issues.append("required image preflight is not enabled")
    validate_image_preflight_source(issues, plan)
    if require_image_preflight:
        validate_image_preflight(
            issues,
            plan,
            verify_manifest=verify_image_manifest,
        )
    validate_campaign_summary(
        issues,
        plan,
        required=require_campaign_summary,
    )
    validate_pre_eval_summary_status(issues, plan.get("preEvalSummaryStatus"))
    if verify_pre_eval_summary:
        validate_pre_eval_summary_file(
            issues,
            plan,
            max_age_seconds=effective_max_pre_eval_age_seconds(
                issues,
                plan,
                max_pre_eval_age_seconds,
            ),
            verify_image_manifest=verify_image_manifest,
            now=now,
        )
    if verify_harbor_config:
        validate_harbor_config_file(issues, plan)
    if verify_prebuilt_binary:
        validate_prebuilt_binary_file(issues, plan)
    if verify_auth_file:
        validate_auth_file(issues, plan)
    if verify_harness_files:
        validate_harness_files(issues, plan)
    for field in REQUIRED_TEXT_FIELDS:
        if not isinstance(plan.get(field), str) or not plan.get(field):
            issues.append(f"{field} is missing")
    for field in REQUIRED_SHA256_FIELDS:
        validate_required_sha256(issues, plan, field)
    validate_config_hashes_match(issues, plan)
    validate_required_dependency_snapshots(issues, plan)

    return ValidationResult(ok=not issues, issues=issues)


def validate_pre_eval_summary_status(issues: list[str], value: Any) -> None:
    if not isinstance(value, dict):
        issues.append("preEvalSummaryStatus is missing")
        return
    status = str(value.get("status") or "")
    if status != "ok":
        issues.append(f"pre-eval summary status is {status or '<missing>'}")
    blocked_checks = list_value(value.get("blockedChecks"))
    if blocked_checks:
        issues.append(
            "pre-eval summary blocked checks: "
            + ", ".join(str(item) for item in blocked_checks)
        )


def effective_max_pre_eval_age_seconds(
    issues: list[str],
    plan: dict[str, Any],
    override: int | None,
) -> int | None:
    if override is not None:
        return override
    value = plan.get("maxPreEvalAgeSeconds")
    if value is None:
        return None
    try:
        seconds = int(value)
    except (TypeError, ValueError):
        issues.append("maxPreEvalAgeSeconds is invalid")
        return None
    if seconds < 0:
        issues.append("maxPreEvalAgeSeconds is invalid")
        return None
    return seconds


def validate_pre_eval_summary_file(
    issues: list[str],
    plan: dict[str, Any],
    *,
    max_age_seconds: int | None = None,
    verify_image_manifest: bool = False,
    now: datetime | None = None,
) -> None:
    expected = plan.get("preEvalSummarySha256")
    if not isinstance(expected, str) or not expected:
        issues.append("preEvalSummarySha256 is missing")
        return
    summary_path = plan.get("preEvalSummary")
    if not isinstance(summary_path, str) or not summary_path:
        issues.append("preEvalSummary is missing")
        return
    try:
        actual = file_sha256(Path(summary_path))
    except OSError as exc:
        issues.append(f"pre-eval summary file cannot be read: {exc}")
        return
    if actual != expected:
        issues.append("pre-eval summary SHA-256 mismatch")
        return
    try:
        summary = load_json(Path(summary_path))
    except Exception as exc:
        issues.append(f"pre-eval summary file is invalid: {exc}")
        return
    result = validate_pre_eval_summary(
        summary,
        require_prebuilt=True,
        require_auth=True,
        require_tests=True,
        require_image_preflight=summary_image_preflight_required(plan),
        require_analysis=plan.get("requireAnalysis") is True,
        verify_harbor_configs=True,
        verify_harness_files=True,
        verify_prebuilt_binary=True,
        verify_auth_file=True,
        verify_image_manifest=verify_image_manifest,
        require_campaign_summary=plan.get("requireCampaignSummary") is True,
        expected_campaign_summary=campaign_summary_json_path(plan),
        max_age_seconds=max_age_seconds,
        now=now,
    )
    for issue in result.issues:
        issues.append(f"pre-eval summary validation: {issue}")
    validate_embedded_summary_status_matches_summary(
        issues,
        plan.get("preEvalSummaryStatus"),
        summary,
    )
    validate_pre_eval_output_dir_matches_summary(issues, plan, summary)
    validate_pre_eval_options_match_summary(issues, plan, summary)
    validate_plan_copies_match_summary(issues, plan, summary)


def validate_embedded_summary_status_matches_summary(
    issues: list[str],
    embedded: Any,
    summary: dict[str, Any],
) -> None:
    if not isinstance(embedded, dict):
        return
    if embedded.get("status") != summary.get("status"):
        issues.append("pre-eval summary status mismatch")
    embedded_blocked = list_value(embedded.get("blockedChecks"))
    summary_blocked = list_value(summary.get("blockedChecks"))
    if embedded_blocked != summary_blocked:
        issues.append("pre-eval summary blockedChecks mismatch")
    if "generatedAt" in embedded and embedded.get("generatedAt") != summary.get("generatedAt"):
        issues.append("pre-eval summary generatedAt mismatch")
    git = summary.get("git") if isinstance(summary.get("git"), dict) else {}
    if "gitHead" in embedded and embedded.get("gitHead") != git.get("head"):
        issues.append("pre-eval summary gitHead mismatch")


def validate_pre_eval_output_dir_matches_summary(
    issues: list[str],
    plan: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    summary_output_dir = summary.get("outputDir")
    if not isinstance(summary_output_dir, str) or not summary_output_dir:
        return
    plan_output_dir = plan.get("preEvalOutputDir")
    if not isinstance(plan_output_dir, str) or not plan_output_dir:
        issues.append("preEvalOutputDir is missing")
    elif plan_output_dir != summary_output_dir:
        issues.append("pre-eval summary outputDir mismatch")


def validate_pre_eval_options_match_summary(
    issues: list[str],
    plan: dict[str, Any],
    summary: dict[str, Any],
) -> None:
    options = summary.get("options") if isinstance(summary.get("options"), dict) else {}
    if (
        image_preflight_source(plan) != "route_manifest"
        and isinstance(options.get("preflightImages"), bool)
    ):
        if plan.get("requireImagePreflight") is not options["preflightImages"]:
            issues.append("pre-eval summary preflightImages mismatch")
    if isinstance(options.get("pullImages"), bool):
        if plan.get("pullPreflight") is not options["pullImages"]:
            issues.append("pre-eval summary pullImages mismatch")
    if (
        image_preflight_source(plan) != "route_manifest"
        and isinstance(options.get("offlineImages"), bool)
    ):
        if plan.get("offlinePreflight") is not options["offlineImages"]:
            issues.append("pre-eval summary offlineImages mismatch")


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def image_preflight_source(plan: dict[str, Any]) -> str:
    source = plan.get("imagePreflightSource")
    return source if isinstance(source, str) and source else "pre_eval_summary"


def validate_image_preflight_source(issues: list[str], plan: dict[str, Any]) -> None:
    source = image_preflight_source(plan)
    if source not in VALID_IMAGE_PREFLIGHT_SOURCES:
        issues.append(f"imagePreflightSource is {source}")
        return
    image_preflight = plan.get("imagePreflight")
    if not isinstance(image_preflight, dict):
        return
    embedded_source = image_preflight.get("source")
    if isinstance(embedded_source, str) and embedded_source and embedded_source != source:
        issues.append("imagePreflight.source mismatch")


def summary_image_preflight_required(plan: dict[str, Any]) -> bool:
    return (
        plan.get("requireImagePreflight") is True
        and image_preflight_source(plan) != "route_manifest"
    )


def campaign_summary_json_path(plan: dict[str, Any]) -> Path | None:
    campaign_summary = dict_value(plan.get("campaignSummary"))
    summary_json = campaign_summary.get("summaryJson")
    if not isinstance(summary_json, str) or not summary_json:
        return None
    return Path(summary_json)


def validate_harbor_config_file(issues: list[str], plan: dict[str, Any]) -> None:
    expected = plan.get("harborConfigSha256")
    if not isinstance(expected, str) or not expected:
        issues.append("harborConfigSha256 is missing")
        return
    pre_eval_expected = plan.get("preEvalHarborConfigSha256")
    if not isinstance(pre_eval_expected, str) or not pre_eval_expected:
        issues.append("preEvalHarborConfigSha256 is missing")
    elif pre_eval_expected != expected:
        issues.append("pre-eval Harbor config SHA-256 mismatch")
    config_path = plan.get("harborConfig")
    if not isinstance(config_path, str) or not config_path:
        issues.append("harborConfig is missing")
        return
    try:
        actual = file_sha256(Path(config_path))
    except OSError as exc:
        issues.append(f"Harbor config file cannot be read: {exc}")
        return
    if actual != expected:
        issues.append("Harbor config SHA-256 mismatch")


def validate_prebuilt_binary_file(issues: list[str], plan: dict[str, Any]) -> None:
    if not isinstance(plan.get("prebuiltBinary"), dict):
        issues.append("prebuiltBinary is missing")
        return
    validate_prebuilt_file(issues, plan.get("prebuiltBinary"))


def validate_auth_file(issues: list[str], plan: dict[str, Any]) -> None:
    if not isinstance(plan.get("authFile"), dict):
        issues.append("authFile is missing")
        return
    validate_auth_file_summary(issues, plan.get("authFile"))


def validate_image_preflight(
    issues: list[str],
    plan: dict[str, Any],
    *,
    verify_manifest: bool = False,
) -> None:
    image_preflight = plan.get("imagePreflight")
    if not isinstance(image_preflight, dict):
        issues.append("imagePreflight is missing")
        return
    status = str(image_preflight.get("status") or "")
    if status != "passed":
        issues.append(f"imagePreflight status is {status or '<missing>'}")
    preflight_config = image_preflight.get("config")
    harbor_config = plan.get("harborConfig")
    if isinstance(preflight_config, str) and isinstance(harbor_config, str):
        if preflight_config != harbor_config:
            issues.append("image preflight config does not match Harbor config")
    else:
        issues.append("imagePreflight.config is missing")
    validate_image_preflight_evidence(issues, image_preflight)
    validate_image_preflight_clean_details(issues, image_preflight)
    if verify_manifest:
        validate_image_manifest_file(issues, image_preflight)


def validate_harness_files(issues: list[str], plan: dict[str, Any]) -> None:
    harness = plan.get("harborHarness")
    if not isinstance(harness, dict):
        issues.append("harborHarness is missing")
        return
    expected_combined = harness.get("combinedSha256")
    if not isinstance(expected_combined, str) or not expected_combined:
        issues.append("harborHarness.combinedSha256 is missing")
        return
    entries = harness.get("entries")
    if not isinstance(entries, list) or not entries:
        issues.append("harborHarness.entries is missing")
        return
    file_count = optional_int_value(harness.get("files"))
    if file_count is not None:
        if file_count <= 0:
            issues.append("harborHarness.files is not positive")
        elif file_count != len(entries):
            issues.append(
                f"harborHarness files count mismatch: expected {len(entries)}, got {file_count}"
            )

    validate_harbor_harness_files(
        issues,
        entries,
        expected_combined,
        required_paths=DEFAULT_HARNESS_FILES,
    )


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def optional_int_value(value: Any) -> int | None:
    try:
        return int(value)
    except (TypeError, ValueError):
        return None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("plan", type=Path)
    parser.add_argument("--require-ready", action="store_true")
    parser.add_argument("--allow-dry-run", action="store_true")
    parser.add_argument("--require-image-preflight", action="store_true")
    parser.add_argument("--verify-pre-eval-summary", action="store_true")
    parser.add_argument("--verify-harbor-config", action="store_true")
    parser.add_argument("--verify-prebuilt-binary", action="store_true")
    parser.add_argument("--verify-auth-file", action="store_true")
    parser.add_argument("--verify-harness-files", action="store_true")
    parser.add_argument("--verify-image-manifest", action="store_true")
    parser.add_argument("--require-campaign-summary", action="store_true")
    parser.add_argument("--max-pre-eval-age-seconds", type=int)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        plan = load_json(args.plan)
    except Exception as exc:
        print(f"validate_tbench_launch_plan: {exc}", file=sys.stderr)
        return 2

    result = validate_plan(
        plan,
        require_ready=args.require_ready,
        allow_dry_run=args.allow_dry_run,
        require_image_preflight=args.require_image_preflight,
        verify_pre_eval_summary=args.verify_pre_eval_summary,
        verify_harbor_config=args.verify_harbor_config,
        verify_prebuilt_binary=args.verify_prebuilt_binary,
        verify_auth_file=args.verify_auth_file,
        verify_harness_files=args.verify_harness_files,
        verify_image_manifest=args.verify_image_manifest,
        require_campaign_summary=args.require_campaign_summary,
        max_pre_eval_age_seconds=args.max_pre_eval_age_seconds,
    )
    if result.ok:
        print("TBench launch plan validation passed")
        return 0
    print("TBench launch plan validation failed", file=sys.stderr)
    for issue in result.issues:
        print(f"- {issue}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
