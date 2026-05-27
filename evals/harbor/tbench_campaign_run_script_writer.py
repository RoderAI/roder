"""Run-script writer for generated Harbor Terminal-Bench campaigns."""

from __future__ import annotations

import shlex
from pathlib import Path
from typing import Any


def write_run_script(
    *,
    path: Path,
    repo_root: Path,
    manifest_path: Path,
    output_dir: Path,
    routes: list[dict[str, Any]],
) -> None:
    lines = [
        "#!/usr/bin/env bash",
        "set -euo pipefail",
        "",
        f"REPO_ROOT=${{RODER_REPO_ROOT:-{shlex.quote(str(repo_root))}}}",
        'cd "$REPO_ROOT"',
        'export PYTHONPATH="$PWD/evals/harbor${PYTHONPATH:+:$PYTHONPATH}"',
        "",
        f"MANIFEST={shlex.quote(str(manifest_path))}",
        f"PREFLIGHT_DIR={shlex.quote(str(output_dir))}",
        'dry_run="${RODER_HARBOR_DRY_RUN:-0}"',
        "",
        'python3 evals/harbor/validate_tbench_campaign.py "$MANIFEST"',
        "",
        'if [[ "${RODER_HARBOR_PREFLIGHT_PULL:-}" == "1" ]]; then',
        "  preflight_args=(--pull)",
        "else",
        "  preflight_args=(--offline)",
        "fi",
        "",
    ]
    for route in routes:
        config = str(route["config"])
        image_manifest = str(route["imageManifest"])
        lines.append(
            f"python3 evals/harbor/preflight_tbench_images.py --config {shlex.quote(config)} "
            f'"${{preflight_args[@]}}" --manifest {shlex.quote(image_manifest)}'
        )
    lines.extend(
        [
            "",
            'python3 evals/harbor/validate_tbench_campaign.py "$MANIFEST" '
            '--require-image-preflight --preflight-dir "$PREFLIGHT_DIR"',
            "",
            'if [[ "$dry_run" != "1" && "${RODER_HARBOR_LIVE_TBENCH:-}" != "1" ]]; then',
            "  echo \"Campaign preflight complete. Set RODER_HARBOR_LIVE_TBENCH=1 to run Harbor routes.\"",
            "  exit 0",
            "fi",
            "",
            'pre_eval_max_age_seconds="${RODER_HARBOR_PRE_EVAL_MAX_AGE_SECONDS:-7200}"',
            'pre_eval_summary="${RODER_HARBOR_PRE_EVAL_SUMMARY:-}"',
            'pre_eval_campaign_summary="${RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY:-}"',
            'pre_eval_output_dir="${RODER_HARBOR_PRE_EVAL_OUTPUT_DIR:-$PREFLIGHT_DIR/pre-eval}"',
            "pre_eval_ran_here=0",
            'if [[ -z "$pre_eval_summary" ]]; then',
            '  pre_eval_args=(--require-prebuilt --require-auth --output-dir "$pre_eval_output_dir")',
            *[
                f"  pre_eval_args+=(--config {shlex.quote(str(route['config']))})"
                for route in routes
            ],
            '  if [[ -n "${RODER_HARBOR_PRE_EVAL_ANALYSIS:-}" ]]; then',
            '    pre_eval_args+=(--analysis "$RODER_HARBOR_PRE_EVAL_ANALYSIS")',
            '    if [[ -n "${RODER_HARBOR_PRE_EVAL_ANALYSIS_BASELINE:-}" ]]; then',
            '      pre_eval_args+=(--analysis-baseline "$RODER_HARBOR_PRE_EVAL_ANALYSIS_BASELINE")',
            "    fi",
            "  fi",
            '  if [[ -n "$pre_eval_campaign_summary" ]]; then',
            '    pre_eval_args+=(--campaign-summary "$pre_eval_campaign_summary")',
            "  fi",
            '  evals/harbor/run-roder-pre-eval-diagnostics.sh "${pre_eval_args[@]}"',
            "  pre_eval_ran_here=1",
            '  pre_eval_summary="$pre_eval_output_dir/pre-eval-summary.json"',
            "fi",
            "summary_validation_args=(",
            '  "$pre_eval_summary"',
            "  --require-prebuilt",
            "  --require-auth",
            "  --require-tests",
            "  --verify-harbor-configs",
            "  --verify-harness-files",
            "  --verify-prebuilt-binary",
            "  --verify-auth-file",
            '  --max-age-seconds "$pre_eval_max_age_seconds"',
            ")",
            *[
                f"summary_validation_args+=(--require-config {shlex.quote(str(route['config']))})"
                for route in routes
            ],
            'if [[ -n "${RODER_HARBOR_PRE_EVAL_ANALYSIS:-}" || "${RODER_HARBOR_PRE_EVAL_REQUIRE_ANALYSIS:-}" == "1" ]]; then',
            "  summary_validation_args+=(--require-analysis)",
            "fi",
            'if [[ -n "$pre_eval_campaign_summary" ]]; then',
            "  summary_validation_args+=(--require-campaign-summary)",
            '  summary_validation_args+=(--campaign-summary "$pre_eval_campaign_summary")',
            "fi",
            'python3 evals/harbor/validate_pre_eval_summary.py "${summary_validation_args[@]}"',
            "",
            "launch_plan_campaign_args=()",
            "launch_plan_validation_campaign_args=()",
            "launch_plan_run_context_args=()",
            'if [[ "$pre_eval_ran_here" == "1" ]]; then',
            "  launch_plan_run_context_args=(--pre-eval-ran-here)",
            "fi",
            'if [[ -n "$pre_eval_campaign_summary" ]]; then',
            '  launch_plan_campaign_args=(--campaign-summary "$pre_eval_campaign_summary")',
            "  launch_plan_validation_campaign_args=(--require-campaign-summary)",
            "fi",
            "",
            'if [[ "$dry_run" == "1" ]]; then',
            *dry_run_launch_plan_lines(routes),
            '  python3 evals/harbor/validate_tbench_campaign.py "$MANIFEST" '
            '--require-image-preflight --require-launch-plans '
            '--allow-dry-run-launch-plans --preflight-dir "$PREFLIGHT_DIR"',
            '  echo "Campaign dry-run complete. Pre-eval summary validated: $pre_eval_summary"',
            "  exit 0",
            "fi",
            "",
            "if ! command -v harbor >/dev/null 2>&1; then",
            "  echo \"harbor is not on PATH. Install it with: uv tool install harbor\" >&2",
            "  exit 1",
            "fi",
            "",
            "route_job_dirs=(",
            *[f"  {shlex.quote(str(route['jobDir']))}" for route in routes],
            ")",
            'if [[ "${RODER_HARBOR_REPLACE_JOB:-}" != "1" ]]; then',
            '  for job_dir in "${route_job_dirs[@]}"; do',
            '    if [[ -e "$job_dir" ]]; then',
            '      echo "$job_dir already exists. Set RODER_HARBOR_REPLACE_JOB=1 to replace it." >&2',
            "      exit 2",
            "    fi",
            "  done",
            "else",
            '  for job_dir in "${route_job_dirs[@]}"; do',
            '    rm -rf "$job_dir"',
            "  done",
            "fi",
            "",
        ]
    )
    for route in routes:
        lines.extend(ready_launch_plan_lines(route))
        lines.extend(
            [
                f"harbor run --config {shlex.quote(str(route['config']))}",
                (
                    "python3 evals/harbor/analyze_tbench_run.py "
                    f"{shlex.quote(str(route['jobDir']))} "
                    "--require-clean "
                    f"--json {shlex.quote(str(route['analysisJson']))} "
                    f"--markdown {shlex.quote(str(route['analysisMarkdown']))} "
                    f"--manifest-dir {shlex.quote(str(route['analysisManifestDir']))} "
                    "--group-scored-failures"
                ),
                (
                    "python3 evals/harbor/validate_tbench_analysis.py "
                    f"{shlex.quote(str(route['analysisJson']))} "
                    "--baseline evals/harbor/tbench-clean-baseline.json "
                    f"--expected-trials {int(route['taskCount'])}"
                ),
            ]
        )
    lines.extend(
        [
            "",
            'python3 evals/harbor/validate_tbench_campaign.py "$MANIFEST" '
            '--require-image-preflight --require-analysis --require-launch-plans '
            '--preflight-dir "$PREFLIGHT_DIR"',
        ]
    )
    path.write_text("\n".join(lines).rstrip() + "\n")
    path.chmod(path.stat().st_mode | 0o755)


def dry_run_launch_plan_lines(routes: list[dict[str, Any]]) -> list[str]:
    lines: list[str] = []
    for route in routes:
        lines.extend(route_launch_plan_lines(route, dry_run=True))
    return ["  " + line if line else line for line in lines]


def ready_launch_plan_lines(route: dict[str, Any]) -> list[str]:
    return route_launch_plan_lines(route, dry_run=False)


def route_launch_plan_lines(route: dict[str, Any], *, dry_run: bool) -> list[str]:
    mode_flag = "--dry-run" if dry_run else ""
    validation_mode = "--allow-dry-run" if dry_run else "--require-ready"
    return [
        (
            "python3 evals/harbor/write_tbench_launch_plan.py "
            f"--output {shlex.quote(str(route['launchPlan']))} "
            '--pre-eval-summary "$pre_eval_summary" '
            '--pre-eval-output-dir "$pre_eval_output_dir" '
            f"--job-dir {shlex.quote(str(route['jobDir']))} "
            f"--harbor-config {shlex.quote(str(route['config']))} "
            f"--analysis-json {shlex.quote(str(route['analysisJson']))} "
            f"--analysis-markdown {shlex.quote(str(route['analysisMarkdown']))} "
            '--max-pre-eval-age-seconds "$pre_eval_max_age_seconds" '
            f"--image-preflight-manifest {shlex.quote(str(route['imageManifest']))} "
            "--require-image-preflight "
            f"{mode_flag} "
            '${launch_plan_run_context_args[@]+"${launch_plan_run_context_args[@]}"} '
            '${launch_plan_campaign_args[@]+"${launch_plan_campaign_args[@]}"}'
        ).rstrip(),
        (
            "python3 evals/harbor/validate_tbench_launch_plan.py "
            f"{shlex.quote(str(route['launchPlan']))} "
            f"{validation_mode} "
            "--verify-pre-eval-summary "
            "--verify-harbor-config "
            "--verify-prebuilt-binary "
            "--verify-auth-file "
            "--verify-harness-files "
            "--require-image-preflight "
            "--verify-image-manifest "
            '--max-pre-eval-age-seconds "$pre_eval_max_age_seconds" '
            '${launch_plan_validation_campaign_args[@]+"${launch_plan_validation_campaign_args[@]}"}'
        ),
    ]
