#!/usr/bin/env python3
"""Write a Harbor Terminal-Bench launch-plan artifact."""

from __future__ import annotations

import argparse
import hashlib
import json
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def load_summary(path: Path) -> tuple[dict[str, Any], str | None]:
    try:
        summary_bytes = path.read_bytes()
        summary = json.loads(summary_bytes)
        summary_sha256 = hashlib.sha256(summary_bytes).hexdigest()
        if not isinstance(summary, dict):
            return {}, summary_sha256
        return summary, summary_sha256
    except Exception:
        return {}, None


def file_sha256(path: Path) -> str | None:
    try:
        return hashlib.sha256(path.read_bytes()).hexdigest()
    except OSError:
        return None


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def summary_check(summary: dict[str, Any], name: str) -> dict[str, Any]:
    checks = dict_value(summary.get("checks"))
    return dict_value(checks.get(name))


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def copied_fields(source: dict[str, Any], fields: tuple[str, ...]) -> dict[str, Any]:
    return {key: source[key] for key in fields if key in source}


def pre_eval_harbor_config_sha256(
    summary: dict[str, Any],
    harbor_config: str,
) -> str | None:
    harbor_configs = summary_check(summary, "harborConfigs")
    entries = harbor_configs.get("entries")
    if not isinstance(entries, list):
        return None
    for entry in entries:
        if not isinstance(entry, dict):
            continue
        if entry.get("path") == harbor_config and isinstance(entry.get("sha256"), str):
            return entry["sha256"]
    return None


def pre_eval_summary_status(summary: dict[str, Any]) -> dict[str, Any]:
    git = dict_value(summary.get("git"))
    status = {
        "status": summary.get("status"),
        "blockedChecks": summary.get("blockedChecks")
        if isinstance(summary.get("blockedChecks"), list)
        else [],
    }
    if summary.get("generatedAt"):
        status["generatedAt"] = summary.get("generatedAt")
    if git.get("head"):
        status["gitHead"] = git.get("head")
    return status


def effective_bool(summary: dict[str, Any], option: str, fallback: bool) -> bool:
    options = dict_value(summary.get("options"))
    value = options.get(option)
    return value if isinstance(value, bool) else fallback


def effective_output_dir(summary: dict[str, Any], fallback: str) -> str:
    output_dir = summary.get("outputDir")
    return output_dir if isinstance(output_dir, str) and output_dir else fallback


def image_preflight_from_manifest(path: Path | None) -> dict[str, Any]:
    if path is None:
        return {}
    try:
        manifest = json.loads(path.read_text())
    except Exception as exc:
        return {
            "status": "failed",
            "source": "route_manifest",
            "manifest": str(path),
            "error": str(exc),
        }
    if not isinstance(manifest, dict):
        return {
            "status": "failed",
            "source": "route_manifest",
            "manifest": str(path),
            "error": "manifest must be a JSON object",
        }
    summary = dict_value(manifest.get("summary"))
    return {
        "status": "passed" if manifest.get("clean") is True else "failed",
        "source": "route_manifest",
        "config": manifest.get("config"),
        "manifest": str(path),
        "tasks": int_value(summary.get("tasks")),
        "uniqueImages": int_value(summary.get("unique_images")),
        "present": int_value(summary.get("present")),
        "missing": int_value(summary.get("missing")),
        "unresolved": int_value(summary.get("unresolved")),
        "pullFailed": int_value(summary.get("pull_failed")),
        "offline": manifest.get("offline") is True,
        "selectionErrors": selection_errors(manifest),
        "blockedTasks": blocked_image_tasks(manifest),
    }


def selection_errors(manifest: dict[str, Any]) -> list[str]:
    return [str(item) for item in list_value(manifest.get("selection_errors"))]


def blocked_image_tasks(manifest: dict[str, Any]) -> list[dict[str, Any]]:
    blocked: list[dict[str, Any]] = []
    for task in list_value(manifest.get("tasks")):
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


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def build_launch_plan(args: argparse.Namespace) -> dict[str, Any]:
    summary, summary_sha256 = load_summary(args.pre_eval_summary)
    is_dry_run = args.dry_run
    job_dir_exists = args.job_dir.exists()
    job_dir_blocks_launch = (not is_dry_run) and job_dir_exists and not args.replace_job
    blocked_reasons = ["existing_job_dir"] if job_dir_blocks_launch else []
    launch_status = "dry_run" if is_dry_run else "blocked" if blocked_reasons else "ready"

    route_image_preflight = image_preflight_from_manifest(args.image_preflight_manifest)
    effective_require_image = bool(route_image_preflight) or effective_bool(
        summary,
        "preflightImages",
        args.require_image_preflight,
    )
    effective_pull_preflight = effective_bool(summary, "pullImages", args.pull_preflight)
    effective_offline_preflight = (
        route_image_preflight.get("offline") is True
        if route_image_preflight
        else effective_bool(summary, "offlineImages", False)
    )
    campaign_summary = summary_check(summary, "campaignSummary")
    require_campaign_summary = bool(args.campaign_summary)
    options = dict_value(summary.get("options"))
    if isinstance(options.get("campaignSummary"), str) and options["campaignSummary"]:
        require_campaign_summary = True
    require_analysis = bool(args.analysis_target) or args.require_analysis
    if isinstance(options.get("analysisTarget"), str) and options["analysisTarget"]:
        require_analysis = True

    harbor_config = args.harbor_config.as_posix()
    prebuilt = dict_value(summary.get("prebuiltBinary"))
    auth = dict_value(summary.get("authFile"))

    return {
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "launchStatus": launch_status,
        "blockedReasons": blocked_reasons,
        "dryRun": is_dry_run,
        "wouldRunHarbor": (not is_dry_run) and not job_dir_blocks_launch,
        "harborConfig": harbor_config,
        "harborConfigSha256": file_sha256(args.harbor_config),
        "preEvalHarborConfigSha256": pre_eval_harbor_config_sha256(
            summary,
            harbor_config,
        ),
        "jobDir": args.job_dir.as_posix(),
        "jobDirExists": job_dir_exists,
        "jobDirBlocksLaunch": job_dir_blocks_launch,
        "blockedBeforeHarbor": blocked_reasons[0] if blocked_reasons else None,
        "analysisJson": args.analysis_json.as_posix(),
        "analysisMarkdown": args.analysis_markdown.as_posix(),
        "preEvalSummary": str(args.pre_eval_summary),
        "preEvalSummarySha256": summary_sha256,
        "preEvalSummaryStatus": pre_eval_summary_status(summary),
        "prebuiltBinary": copied_fields(
            prebuilt,
            (
                "path",
                "sha256",
                "sizeBytes",
                "modifiedAt",
                "fileType",
                "linuxX8664Elf",
                "executable",
            ),
        ),
        "authFile": copied_fields(
            auth,
            (
                "path",
                "sizeBytes",
                "modifiedAt",
                "validJson",
                "jsonFields",
            ),
        ),
        "deadlinePolicy": summary_check(summary, "harborConfigs").get(
            "deadlinePolicy",
            {},
        ),
        "imagePreflight": route_image_preflight or summary_check(summary, "imagePreflight"),
        "imagePreflightSource": "route_manifest"
        if route_image_preflight
        else "pre_eval_summary",
        "harborHarness": summary_check(summary, "harborHarness"),
        "harborHarnessTests": summary_check(summary, "harborHarnessTests"),
        "campaignSummary": campaign_summary,
        "preEvalOutputDir": effective_output_dir(summary, args.pre_eval_output_dir),
        "preEvalRanHere": args.pre_eval_ran_here,
        "requireImagePreflight": effective_require_image,
        "requireCampaignSummary": require_campaign_summary,
        "requireAnalysis": require_analysis,
        "maxPreEvalAgeSeconds": args.max_pre_eval_age_seconds,
        "skipPreflight": args.skip_preflight,
        "pullPreflight": effective_pull_preflight,
        "offlinePreflight": effective_offline_preflight,
        "replaceJob": args.replace_job,
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--pre-eval-summary", type=Path, required=True)
    parser.add_argument("--pre-eval-output-dir", required=True)
    parser.add_argument("--pre-eval-ran-here", action="store_true")
    parser.add_argument("--require-image-preflight", action="store_true")
    parser.add_argument("--image-preflight-manifest", type=Path)
    parser.add_argument("--require-analysis", action="store_true")
    parser.add_argument("--analysis-target", default="")
    parser.add_argument("--skip-preflight", action="store_true")
    parser.add_argument("--pull-preflight", action="store_true")
    parser.add_argument("--replace-job", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--job-dir", type=Path, required=True)
    parser.add_argument("--harbor-config", type=Path, required=True)
    parser.add_argument("--analysis-json", type=Path, required=True)
    parser.add_argument("--analysis-markdown", type=Path, required=True)
    parser.add_argument("--max-pre-eval-age-seconds", type=int, required=True)
    parser.add_argument("--campaign-summary", default="")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    plan = build_launch_plan(args)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(plan, indent=2) + "\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
