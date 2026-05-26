#!/usr/bin/env python3
"""Write the local Harbor pre-eval diagnostic summary envelope."""

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

from pre_eval_file_summary import (
    auth_is_blocked,
    auth_summary,
    prebuilt_is_blocked,
    prebuilt_summary,
)
from pre_eval_campaign_summary import campaign_summary_check
from pre_eval_harness_summary import harness_summary
from pre_eval_config_summary import DEFAULT_CONFIGS, harbor_config_summary
from pre_eval_run_summary import eval_run_summary, tbench_eval_run_summary
from pre_eval_git_summary import git_summary

DEFAULT_PREBUILT = Path("evals/harbor/artifacts/roder-linux-amd64")
DEFAULT_AUTH = Path("~/.roder/auth/codex.json")
NON_BLOCKING_CHECK_STATUSES = {"ok", "passed", "skipped"}


def build_summary(
    *,
    output_root: Path,
    tbench_dir: Path,
    speed_dir: Path | None,
    analysis_dir: Path | None,
    run_tests: bool,
    include_speed: bool,
    require_prebuilt: bool,
    preflight_images: bool,
    offline_images: bool,
    pull_images: bool,
    image_config: str,
    analysis_target: str,
    analysis_baseline: str,
    prebuilt_binary: Path,
    auth_file: Path,
    require_auth: bool,
    image_manifest: Path | None,
    config_paths: tuple[Path, ...] | list[Path] | None = None,
    campaign_summary: Path | None = None,
    harbor_readiness_status: str = "passed",
    harbor_harness_tests_status: str | None = None,
    roder_evals_status: str | None = None,
    failure_step: str = "",
    failure_exit_code: int | None = None,
) -> dict[str, Any]:
    if harbor_harness_tests_status is None:
        harbor_harness_tests_status = "passed" if run_tests else "skipped"
    if roder_evals_status is None:
        roder_evals_status = "passed" if run_tests else "skipped"
    summary: dict[str, Any] = {
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "outputDir": str(output_root),
        "git": git_summary(),
        "options": {
            "runTests": run_tests,
            "includeSpeed": include_speed,
            "requirePrebuilt": require_prebuilt,
            "requireAuth": require_auth,
            "preflightImages": preflight_images,
            "offlineImages": offline_images,
            "pullImages": pull_images,
            "imageConfig": image_config if preflight_images else None,
            "analysisTarget": analysis_target or None,
            "analysisBaseline": analysis_baseline if analysis_target else None,
            "campaignSummary": str(campaign_summary) if campaign_summary else None,
        },
        "prebuiltBinary": prebuilt_summary(prebuilt_binary, require_prebuilt),
        "authFile": auth_summary(auth_file, require_auth),
        "checks": {
            "preEvalOptions": pre_eval_options_summary(
                preflight_images=preflight_images,
                offline_images=offline_images,
                pull_images=pull_images,
            ),
            "harborReadiness": {"status": harbor_readiness_status},
            "harborConfigs": harbor_config_summary(config_paths),
            "harborHarness": harness_summary(),
            "harborHarnessTests": {"status": harbor_harness_tests_status},
            "roderEvalsLib": {"status": roder_evals_status},
            "tbenchDiagnostics": {
                "status": "passed",
                **(tbench_eval_run_summary(tbench_dir) or {}),
            },
        },
    }

    if failure_step or failure_exit_code is not None:
        failure: dict[str, Any] = {}
        if failure_step:
            failure["step"] = failure_step
        if failure_exit_code is not None:
            failure["exitCode"] = failure_exit_code
        summary["failure"] = failure

    if include_speed:
        summary["checks"]["speedPolicy"] = {
            "status": "passed",
            **(eval_run_summary(speed_dir) or {}),
        }

    if analysis_target and analysis_dir is not None:
        summary["checks"]["harborAnalysisBaseline"] = analysis_validation_summary(
            analysis_dir,
            analysis_target,
            analysis_baseline,
        )

    if image_manifest is not None:
        image_preflight = image_preflight_summary(image_manifest)
        apply_image_preflight_option_requirements(
            image_preflight,
            offline_images=offline_images,
        )
        summary["checks"]["imagePreflight"] = image_preflight

    if campaign_summary is not None:
        summary["checks"]["campaignSummary"] = campaign_summary_check(campaign_summary)

    status, blocked_checks = overall_status(summary)
    summary["status"] = status
    summary["blockedChecks"] = blocked_checks
    return summary


def pre_eval_options_summary(
    *,
    preflight_images: bool,
    offline_images: bool,
    pull_images: bool,
) -> dict[str, Any]:
    issues: list[str] = []
    if offline_images and not preflight_images:
        issues.append("offlineImages requires preflightImages")
    if pull_images and not preflight_images:
        issues.append("pullImages requires preflightImages")
    if offline_images and pull_images:
        issues.append("offlineImages cannot be combined with pullImages")
    return {
        "status": "failed" if issues else "passed",
        "issues": issues,
    }


def apply_image_preflight_option_requirements(
    image_preflight: dict[str, Any],
    *,
    offline_images: bool,
) -> None:
    issues: list[str] = []
    existing_issues = image_preflight.get("issues")
    if isinstance(existing_issues, list):
        issues.extend(str(issue) for issue in existing_issues)
    if offline_images and image_preflight.get("offline") is not True:
        issues.append("image preflight did not run in offline mode")
    if issues:
        image_preflight["status"] = "failed"
        image_preflight["issues"] = issues


def overall_status(summary: dict[str, Any]) -> tuple[str, list[str]]:
    blocked_checks: list[str] = []

    checks = summary.get("checks")
    if isinstance(checks, dict):
        for name, check in checks.items():
            if check_is_blocked(check):
                blocked_checks.append(str(name))
    else:
        blocked_checks.append("checks")

    if prebuilt_is_blocked(summary.get("prebuiltBinary")):
        blocked_checks.append("prebuiltBinary")
    if auth_is_blocked(summary.get("authFile")):
        blocked_checks.append("authFile")

    return ("blocked" if blocked_checks else "ok", blocked_checks)


def check_is_blocked(check: Any) -> bool:
    if not isinstance(check, dict):
        return True
    if str(check.get("status") or "") not in NON_BLOCKING_CHECK_STATUSES:
        return True
    return int_value(check.get("failed")) > 0


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def analysis_validation_summary(
    directory: Path,
    analysis_target: str,
    analysis_baseline: str,
) -> dict[str, Any]:
    validation_path = directory / "validation.json"
    summary = {
        "validationJson": str(validation_path),
        "validationMarkdown": str(directory / "validation.md"),
        "analysisTarget": analysis_target,
        "baseline": analysis_baseline,
    }
    if not validation_path.exists():
        return {"status": "missing", **summary}
    try:
        validation = json.loads(validation_path.read_text())
    except Exception as exc:
        return {"status": "failed", "error": str(exc), **summary}
    if not isinstance(validation, dict):
        return {
            "status": "failed",
            "error": "validation must be a JSON object",
            **summary,
        }
    return {
        "status": validation.get("status"),
        "blockedMetrics": blocked_validation_rows(validation),
        "metrics": validation.get("metrics") if isinstance(validation.get("metrics"), dict) else {},
        **summary,
    }


def blocked_validation_rows(validation: dict[str, Any]) -> list[dict[str, Any]]:
    rows = validation.get("rows")
    if not isinstance(rows, list):
        return []
    return [
        row
        for row in rows
        if isinstance(row, dict) and str(row.get("status") or "") == "blocked"
    ]


def image_preflight_summary(path: Path) -> dict[str, Any]:
    base = {
        "manifest": str(path),
        "config": None,
        "tasks": 0,
        "uniqueImages": 0,
        "present": 0,
        "missing": 0,
        "unresolved": 0,
        "pullFailed": 0,
        "offline": False,
        "selectionErrors": [],
        "blockedTasks": [],
    }
    if not path.exists():
        return {"status": "missing", **base}
    try:
        manifest = json.loads(path.read_text())
    except Exception as exc:
        return {"status": "failed", "error": str(exc), **base}
    if not isinstance(manifest, dict):
        return {"status": "failed", "error": "manifest must be a JSON object", **base}
    summary = manifest.get("summary") if isinstance(manifest.get("summary"), dict) else {}
    return {
        **base,
        "status": "passed" if manifest.get("clean") is True else "failed",
        "offline": manifest.get("offline") is True,
        "config": manifest.get("config") if isinstance(manifest.get("config"), str) else None,
        "tasks": int(summary.get("tasks") or 0),
        "uniqueImages": int(summary.get("unique_images") or 0),
        "present": int(summary.get("present") or 0),
        "missing": int(summary.get("missing") or 0),
        "unresolved": int(summary.get("unresolved") or 0),
        "pullFailed": int(summary.get("pull_failed") or 0),
        "selectionErrors": selection_errors(manifest),
        "blockedTasks": blocked_image_tasks(manifest),
    }


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


def optional_path(value: str | None) -> Path | None:
    return Path(value) if value else None


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--summary", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--tbench-dir", type=Path, required=True)
    parser.add_argument("--config", type=Path, action="append", default=[])
    parser.add_argument("--speed-dir")
    parser.add_argument("--analysis-dir")
    parser.add_argument("--run-tests", action="store_true")
    parser.add_argument("--include-speed", action="store_true")
    parser.add_argument("--require-prebuilt", action="store_true")
    parser.add_argument("--preflight-images", action="store_true")
    parser.add_argument("--offline-images", action="store_true")
    parser.add_argument("--pull-images", action="store_true")
    parser.add_argument("--image-config", default="")
    parser.add_argument("--analysis-target", default="")
    parser.add_argument("--analysis-baseline", default="")
    parser.add_argument("--prebuilt-binary", type=Path, default=DEFAULT_PREBUILT)
    parser.add_argument("--auth-file", type=Path, default=DEFAULT_AUTH)
    parser.add_argument("--require-auth", action="store_true")
    parser.add_argument("--image-manifest", type=Path)
    parser.add_argument("--campaign-summary", type=Path)
    parser.add_argument("--harbor-readiness-status", default="passed")
    parser.add_argument("--harbor-harness-tests-status")
    parser.add_argument("--roder-evals-status")
    parser.add_argument("--failure-step", default="")
    parser.add_argument("--failure-exit-code", type=int)
    parser.add_argument(
        "--require-ok",
        action="store_true",
        help="Exit non-zero after writing the summary if the top-level status is blocked.",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    summary = build_summary(
        output_root=args.output_dir,
        tbench_dir=args.tbench_dir,
        speed_dir=optional_path(args.speed_dir),
        analysis_dir=optional_path(args.analysis_dir),
        run_tests=args.run_tests,
        include_speed=args.include_speed,
        require_prebuilt=args.require_prebuilt,
        preflight_images=args.preflight_images,
        offline_images=args.offline_images,
        pull_images=args.pull_images,
        image_config=args.image_config,
        analysis_target=args.analysis_target,
        analysis_baseline=args.analysis_baseline,
        prebuilt_binary=args.prebuilt_binary,
        auth_file=args.auth_file,
        require_auth=args.require_auth,
        image_manifest=args.image_manifest,
        config_paths=tuple(args.config) or DEFAULT_CONFIGS,
        campaign_summary=args.campaign_summary,
        harbor_readiness_status=args.harbor_readiness_status,
        harbor_harness_tests_status=args.harbor_harness_tests_status,
        roder_evals_status=args.roder_evals_status,
        failure_step=args.failure_step,
        failure_exit_code=args.failure_exit_code,
    )
    args.summary.parent.mkdir(parents=True, exist_ok=True)
    args.summary.write_text(json.dumps(summary, indent=2) + "\n")
    print(f"Pre-eval summary written: {args.summary}")
    if args.require_ok and summary.get("status") != "ok":
        blocked = ", ".join(summary.get("blockedChecks") or ["<unknown>"])
        print(f"Pre-eval summary blocked: {blocked}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
