#!/usr/bin/env python3
"""Validate a Harbor pre-eval summary handoff artifact."""

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

from pre_eval_live_checks import (  # noqa: E402
    validate_auth_file,
    validate_harbor_config_files,
    validate_harbor_harness_files,
    validate_prebuilt_file,
)
from pre_eval_config_summary import DEFAULT_CONFIGS  # noqa: E402
from pre_eval_harness_summary import DEFAULT_HARNESS_FILES  # noqa: E402
from pre_eval_image_preflight_validation import validate_image_preflight  # noqa: E402
from pre_eval_summary_tbench_validation import validate_tbench_diagnostics  # noqa: E402

NON_BLOCKING_STATUSES = {"ok", "passed", "skipped"}


class ValidationResult:
    def __init__(self, *, ok: bool, issues: list[str]) -> None:
        self.ok = ok
        self.issues = issues


def load_json(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text())
    if not isinstance(data, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return data


def validate_summary(
    summary: dict[str, Any],
    *,
    require_prebuilt: bool = False,
    require_auth: bool = False,
    require_tests: bool = False,
    require_image_preflight: bool = False,
    require_analysis: bool = False,
    verify_harbor_configs: bool = False,
    verify_harness_files: bool = False,
    verify_prebuilt_binary: bool = False,
    verify_auth_file: bool = False,
    verify_image_manifest: bool = False,
    require_image_config: str | None = None,
    required_configs: list[Path] | tuple[Path, ...] | None = None,
    max_age_seconds: int | None = None,
    now: datetime | None = None,
) -> ValidationResult:
    issues: list[str] = []
    options = dict_value(summary.get("options"))
    checks = dict_value(summary.get("checks"))

    validate_freshness(issues, summary, max_age_seconds, now)
    if summary.get("status") != "ok":
        issues.append(f"summary status is {summary.get('status') or '<missing>'}")
    blocked_checks = list_value(summary.get("blockedChecks"))
    if blocked_checks:
        issues.append("blocked checks: " + ", ".join(str(item) for item in blocked_checks))

    required_config_paths = list(required_configs or [])
    image_config = options.get("imageConfig")
    if isinstance(image_config, str) and image_config:
        required_config_paths.append(Path(image_config))

    require_check_status(issues, checks, "harborReadiness", {"passed"})
    validate_harbor_configs(
        issues,
        checks.get("harborConfigs"),
        verify_files=verify_harbor_configs,
        required_configs=required_config_paths,
    )
    validate_harbor_harness(
        issues,
        checks.get("harborHarness"),
        verify_files=verify_harness_files,
    )
    validate_tbench_diagnostics(issues, checks.get("tbenchDiagnostics"))
    validate_harbor_harness_tests(
        issues,
        checks.get("harborHarnessTests"),
        require_tests,
    )
    validate_roder_evals(issues, checks.get("roderEvalsLib"), require_tests)
    validate_prebuilt(issues, summary, options, require_prebuilt)
    if verify_prebuilt_binary:
        validate_prebuilt_file(issues, summary.get("prebuiltBinary"))
    validate_auth(issues, summary, options, require_auth)
    if verify_auth_file:
        validate_auth_file(issues, summary.get("authFile"))
    validate_image_preflight(
        issues,
        checks,
        options,
        require_image_preflight,
        verify_manifest=verify_image_manifest,
        required_config=require_image_config,
    )
    validate_analysis(issues, checks, options, require_analysis)

    return ValidationResult(ok=not issues, issues=issues)


def validate_freshness(
    issues: list[str],
    summary: dict[str, Any],
    max_age_seconds: int | None,
    now: datetime | None,
) -> None:
    if max_age_seconds is None:
        return
    generated_at = summary.get("generatedAt")
    if not isinstance(generated_at, str) or not generated_at:
        issues.append("summary generatedAt is missing")
        return
    try:
        generated = parse_datetime(generated_at)
    except ValueError as exc:
        issues.append(f"summary generatedAt is invalid: {exc}")
        return
    current = now or datetime.now(timezone.utc)
    if current.tzinfo is None:
        current = current.replace(tzinfo=timezone.utc)
    age_seconds = int((current - generated).total_seconds())
    if age_seconds < 0:
        issues.append(f"summary generatedAt is in the future: {generated_at}")
    elif age_seconds > max_age_seconds:
        issues.append(
            f"summary is stale: age {age_seconds}s exceeds max {max_age_seconds}s"
        )


def parse_datetime(value: str) -> datetime:
    normalized = value.replace("Z", "+00:00")
    parsed = datetime.fromisoformat(normalized)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=timezone.utc)
    return parsed


def validate_harbor_configs(
    issues: list[str],
    check: Any,
    *,
    verify_files: bool = False,
    required_configs: list[Path] | tuple[Path, ...] | None = None,
) -> None:
    if not isinstance(check, dict):
        issues.append("harborConfigs check missing")
        return
    if check.get("status") != "passed":
        issues.append(f"harborConfigs status is {check.get('status') or '<missing>'}")
    config_issues = list_value(check.get("issues"))
    if config_issues:
        issues.append("harbor config issues: " + "; ".join(str(item) for item in config_issues))
    config_count = optional_int_value(check.get("configs"))
    entries = check.get("entries")
    if config_count is not None:
        if config_count <= 0:
            issues.append("harborConfigs configs is not positive")
        elif isinstance(entries, list) and config_count != len(entries):
            issues.append(
                f"harborConfigs configs count mismatch: expected {len(entries)}, got {config_count}"
            )
    if verify_files:
        validate_harbor_config_files(
            issues,
            entries,
            required_paths=tuple(DEFAULT_CONFIGS) + tuple(required_configs or ()),
        )


def validate_harbor_harness(
    issues: list[str],
    check: Any,
    *,
    verify_files: bool = False,
) -> None:
    if not isinstance(check, dict):
        issues.append("harborHarness check missing")
        return
    if check.get("status") != "passed":
        issues.append(f"harborHarness status is {check.get('status') or '<missing>'}")
    harness_issues = list_value(check.get("issues"))
    if harness_issues:
        issues.append(
            "harbor harness issues: " + "; ".join(str(item) for item in harness_issues)
        )
    file_count = int_value(check.get("files"))
    if file_count <= 0:
        issues.append("harborHarness files is not positive")
    if not isinstance(check.get("combinedSha256"), str) or not check.get("combinedSha256"):
        issues.append("harborHarness combinedSha256 is missing")
    entries = check.get("entries")
    if not isinstance(entries, list) or not entries:
        issues.append("harborHarness entries are missing")
        return
    if file_count > 0 and file_count != len(entries):
        issues.append(
            f"harborHarness files count mismatch: expected {len(entries)}, got {file_count}"
        )
    if verify_files:
        validate_harbor_harness_files(
            issues,
            entries,
            check.get("combinedSha256"),
            required_paths=DEFAULT_HARNESS_FILES,
        )


def validate_roder_evals(issues: list[str], check: Any, required: bool) -> None:
    if not isinstance(check, dict):
        issues.append("roderEvalsLib check missing")
        return
    status = str(check.get("status") or "")
    if required and status != "passed":
        issues.append(f"required roder-evals tests did not pass: {status or '<missing>'}")
    elif status not in NON_BLOCKING_STATUSES:
        issues.append(f"roderEvalsLib status is {status or '<missing>'}")


def validate_harbor_harness_tests(
    issues: list[str],
    check: Any,
    required: bool,
) -> None:
    if not isinstance(check, dict):
        issues.append("harborHarnessTests check missing")
        return
    status = str(check.get("status") or "")
    if required and status != "passed":
        issues.append(
            f"required Harbor harness tests did not pass: {status or '<missing>'}"
        )
    elif status not in NON_BLOCKING_STATUSES:
        issues.append(f"harborHarnessTests status is {status or '<missing>'}")


def validate_prebuilt(
    issues: list[str],
    summary: dict[str, Any],
    options: dict[str, Any],
    required: bool,
) -> None:
    if not required:
        return
    if options.get("requirePrebuilt") is not True:
        issues.append("required prebuilt gate did not run")
    prebuilt = dict_value(summary.get("prebuiltBinary"))
    if prebuilt.get("required") is not True:
        issues.append("prebuilt summary was not marked required")
    for field in ("exists", "executable", "linuxX8664Elf"):
        if prebuilt.get(field) is not True:
            issues.append(f"prebuilt binary {field} is not true")


def validate_auth(
    issues: list[str],
    summary: dict[str, Any],
    options: dict[str, Any],
    required: bool,
) -> None:
    if not required:
        return
    if options.get("requireAuth") is not True:
        issues.append("required auth gate did not run")
    auth = dict_value(summary.get("authFile"))
    if auth.get("required") is not True:
        issues.append("auth summary was not marked required")
    for field in ("exists", "validJson"):
        if auth.get(field) is not True:
            issues.append(f"auth file {field} is not true")


def validate_analysis(
    issues: list[str],
    checks: dict[str, Any],
    options: dict[str, Any],
    required: bool,
) -> None:
    if not required:
        return
    if not options.get("analysisTarget"):
        issues.append("required analysis baseline did not run")
    require_check_status(issues, checks, "harborAnalysisBaseline", {"ok", "passed"})


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


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


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
    parser.add_argument("summary", type=Path)
    parser.add_argument("--require-prebuilt", action="store_true")
    parser.add_argument("--require-auth", action="store_true")
    parser.add_argument("--require-tests", action="store_true")
    parser.add_argument("--require-image-preflight", action="store_true")
    parser.add_argument("--require-analysis", action="store_true")
    parser.add_argument("--verify-harbor-configs", action="store_true")
    parser.add_argument("--verify-harness-files", action="store_true")
    parser.add_argument("--verify-prebuilt-binary", action="store_true")
    parser.add_argument("--verify-auth-file", action="store_true")
    parser.add_argument("--verify-image-manifest", action="store_true")
    parser.add_argument("--require-image-config")
    parser.add_argument("--require-config", type=Path, action="append", default=[])
    parser.add_argument("--max-age-seconds", type=int)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        summary = load_json(args.summary)
    except Exception as exc:
        print(f"validate_pre_eval_summary: {exc}", file=sys.stderr)
        return 2

    result = validate_summary(
        summary,
        require_prebuilt=args.require_prebuilt,
        require_auth=args.require_auth,
        require_tests=args.require_tests,
        require_image_preflight=args.require_image_preflight,
        require_analysis=args.require_analysis,
        verify_harbor_configs=args.verify_harbor_configs,
        verify_harness_files=args.verify_harness_files,
        verify_prebuilt_binary=args.verify_prebuilt_binary,
        verify_auth_file=args.verify_auth_file,
        verify_image_manifest=args.verify_image_manifest,
        require_image_config=args.require_image_config,
        required_configs=tuple(args.require_config),
        max_age_seconds=args.max_age_seconds,
    )
    if result.ok:
        print("Pre-eval summary validation passed")
        return 0
    print("Pre-eval summary validation failed", file=sys.stderr)
    for issue in result.issues:
        print(f"- {issue}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
