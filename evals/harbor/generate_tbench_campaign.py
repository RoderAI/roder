#!/usr/bin/env python3
"""Generate routed Harbor configs for the next Terminal-Bench score campaign."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shlex
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

from rerun_tbench_subset import build_subset_config
from tbench_campaign_handoff import pre_eval_handoff
from tbench_campaign_score_projection import score_projection_for_tasks


DEFAULT_BASE_CONFIG = Path("evals/harbor/tbench-full-gpt55-medium.json")


@dataclass(frozen=True)
class CampaignRoute:
    name: str
    description: str
    job_name: str
    tasks: tuple[str, ...]
    reasoning: str
    plan_first: bool = False
    plan_first_reasoning: str | None = None
    plan_first_soft_timeout_sec: int | None = None


VALIDATED_CONVERSION_ROUTES: tuple[CampaignRoute, ...] = (
    CampaignRoute(
        name="medium-validated",
        description="Focused medium-reasoning conversions that need reproducibility.",
        job_name="roder-tbench-validated-conversions-medium",
        reasoning="medium",
        tasks=(
            "financial-document-processor",
            "llm-inference-batching-scheduler",
            "mteb-leaderboard",
            "mteb-retrieve",
        ),
    ),
    CampaignRoute(
        name="xhigh-validated",
        description="Previously failing tasks converted by selective GPT-5.5 xhigh reruns.",
        job_name="roder-tbench-validated-conversions-xhigh",
        reasoning="xhigh",
        tasks=(
            "db-wal-recovery",
            "fix-code-vulnerability",
            "kv-store-grpc",
            "polyglot-c-py",
            "query-optimize",
            "torch-pipeline-parallelism",
            "tune-mjcf",
        ),
    ),
    CampaignRoute(
        name="xhigh-plan-first",
        description="Plan-first xhigh conversions from the remaining-failure rerun.",
        job_name="roder-tbench-validated-conversions-xhigh-plan-first",
        reasoning="xhigh",
        plan_first=True,
        plan_first_reasoning="medium",
        plan_first_soft_timeout_sec=360,
        tasks=(
            "git-leak-recovery",
            "model-extraction-relu-logits",
            "polyglot-rust-c",
            "regex-chess",
        ),
    ),
)


VERIFIER_CONTRACT_ROUTES: tuple[CampaignRoute, ...] = (
    CampaignRoute(
        name="near-misses",
        description="Near-miss tasks that need stricter final verifier-contract loops.",
        job_name="roder-tbench-verifier-contract-near-misses",
        reasoning="xhigh",
        tasks=(
            "dna-assembly",
            "dna-insert",
            "gcode-to-text",
            "protein-assembly",
            "sam-cell-seg",
            "torch-tensor-parallelism",
            "video-processing",
        ),
    ),
)


ENVIRONMENT_TARGET_ROUTES: tuple[CampaignRoute, ...] = (
    CampaignRoute(
        name="service-targets",
        description="Environment and service-target tasks that need endpoint parity checks.",
        job_name="roder-tbench-environment-service-targets",
        reasoning="xhigh",
        tasks=(
            "install-windows-3.11",
            "qemu-alpine-ssh",
            "qemu-startup",
            "train-fasttext",
        ),
    ),
)


HISTORICAL_WIN_ROUTES: tuple[CampaignRoute, ...] = (
    CampaignRoute(
        name="policy-framed",
        description="Historical policy-shaped wins missing from the current conversion campaign.",
        job_name="roder-tbench-historical-wins-policy-framed",
        reasoning="medium",
        tasks=(
            "password-recovery",
            "vulnerable-secret",
        ),
    ),
    CampaignRoute(
        name="environment-targeted",
        description="Historical environment-target win missing from the current conversion campaign.",
        job_name="roder-tbench-historical-wins-environment-targeted",
        reasoning="medium",
        tasks=(
            "qemu-startup",
        ),
    ),
)


CAMPAIGNS: dict[str, tuple[CampaignRoute, ...]] = {
    "environment-target": ENVIRONMENT_TARGET_ROUTES,
    "historical-wins": HISTORICAL_WIN_ROUTES,
    "validated-conversions": VALIDATED_CONVERSION_ROUTES,
    "verifier-contract": VERIFIER_CONTRACT_ROUTES,
}


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def selected_routes(campaign: str, route_names: list[str]) -> tuple[CampaignRoute, ...]:
    try:
        routes = CAMPAIGNS[campaign]
    except KeyError as exc:
        available = ", ".join(sorted(CAMPAIGNS))
        raise ValueError(f"unknown campaign {campaign!r}; available: {available}") from exc
    if not route_names:
        return routes
    by_name = {route.name: route for route in routes}
    unknown = sorted(set(route_names).difference(by_name))
    if unknown:
        available = ", ".join(sorted(by_name))
        raise ValueError(f"unknown route(s): {', '.join(unknown)}; available: {available}")
    return tuple(by_name[name] for name in route_names)


def assert_unique_tasks(routes: tuple[CampaignRoute, ...]) -> None:
    owners: dict[str, str] = {}
    duplicates: list[str] = []
    for route in routes:
        for task in route.tasks:
            if task in owners:
                duplicates.append(f"{task} ({owners[task]}, {route.name})")
            owners[task] = route.name
    if duplicates:
        raise ValueError("campaign routes overlap: " + "; ".join(sorted(duplicates)))


def build_route_config(
    *,
    base_config: dict[str, Any],
    route: CampaignRoute,
) -> dict[str, Any]:
    return build_subset_config(
        source_job=Path(route.job_name),
        base_config=base_config,
        task_names=list(route.tasks),
        class_name=route.name,
        job_name=route.job_name,
        jobs_dir=None,
        reasoning=route.reasoning,
        timeout_sec=None,
        soft_timeout_sec=None,
        eval_deadline_sec=None,
        plan_first=route.plan_first,
        plan_first_soft_timeout_sec=route.plan_first_soft_timeout_sec,
        plan_first_policy_mode=None,
        plan_first_reasoning=route.plan_first_reasoning,
    )


def route_config_path(output_dir: Path, campaign: str, route: CampaignRoute) -> Path:
    return output_dir / f"{campaign}-{route.name}.json"


def write_campaign(
    *,
    campaign: str,
    base_config_path: Path,
    output_dir: Path,
    route_names: list[str],
) -> dict[str, Any]:
    routes = selected_routes(campaign, route_names)
    assert_unique_tasks(routes)
    base_config = load_json(base_config_path)
    output_dir.mkdir(parents=True, exist_ok=True)

    route_entries: list[dict[str, Any]] = []
    for route in routes:
        config = build_route_config(base_config=base_config, route=route)
        config_path = route_config_path(output_dir, campaign, route)
        config_path.write_text(json.dumps(config, indent=2) + "\n")
        config_sha256 = hashlib.sha256(config_path.read_bytes()).hexdigest()
        analysis_json = output_dir / f"{route.name}-analysis.json"
        analysis_markdown = output_dir / f"{route.name}.md"
        analysis_manifest_dir = output_dir / "manifests" / route.name
        image_manifest = output_dir / f"{route.name}-images.json"
        route_entries.append(
            {
                "name": route.name,
                "description": route.description,
                "config": str(config_path),
                "configSha256": config_sha256,
                "jobName": route.job_name,
                "jobDir": str(Path(config.get("jobs_dir", "evals/harbor/jobs")) / route.job_name),
                "analysisJson": str(analysis_json),
                "analysisMarkdown": str(analysis_markdown),
                "analysisManifestDir": str(analysis_manifest_dir),
                "imageManifest": str(image_manifest),
                "reasoning": route.reasoning,
                "planFirst": route.plan_first,
                "planFirstReasoning": route.plan_first_reasoning,
                "planFirstSoftTimeoutSec": route.plan_first_soft_timeout_sec,
                "tasks": list(route.tasks),
                "taskCount": len(route.tasks),
            }
        )

    unique_tasks = sorted({task for route in routes for task in route.tasks})
    script_path = output_dir / f"run-{campaign}.sh"
    manifest = {
        "generatedAt": datetime.now(timezone.utc).isoformat(),
        "campaign": campaign,
        "baseConfig": str(base_config_path),
        "routes": route_entries,
        "runScript": str(script_path),
        "preEval": pre_eval_handoff(output_dir),
        "summary": {
            "routes": len(route_entries),
            "tasks": sum(route["taskCount"] for route in route_entries),
            "uniqueTasks": len(unique_tasks),
            "uniqueTaskNames": unique_tasks,
        },
        "scoreProjection": score_projection_for_tasks(unique_tasks),
    }
    manifest_path = output_dir / f"{campaign}-manifest.json"
    manifest["manifest"] = str(manifest_path)
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")
    write_run_script(
        path=script_path,
        repo_root=Path.cwd(),
        manifest_path=manifest_path,
        output_dir=output_dir,
        routes=route_entries,
    )
    return manifest


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
        lines.extend(
            [
                f"python3 evals/harbor/preflight_tbench_images.py --config {shlex.quote(config)} "
                f'"${{preflight_args[@]}}" --manifest {shlex.quote(image_manifest)}',
            ]
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
            'pre_eval_output_dir="${RODER_HARBOR_PRE_EVAL_OUTPUT_DIR:-$PREFLIGHT_DIR/pre-eval}"',
            'if [[ -z "$pre_eval_summary" ]]; then',
            "  pre_eval_args=(--require-prebuilt --require-auth --output-dir \"$pre_eval_output_dir\")",
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
            '  evals/harbor/run-roder-pre-eval-diagnostics.sh "${pre_eval_args[@]}"',
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
            'python3 evals/harbor/validate_pre_eval_summary.py "${summary_validation_args[@]}"',
            "",
            'if [[ "$dry_run" == "1" ]]; then',
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
            *[
                f"  {shlex.quote(str(route['jobDir']))}"
                for route in routes
            ],
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
            '--require-image-preflight --require-analysis --preflight-dir "$PREFLIGHT_DIR"',
        ]
    )
    path.write_text("\n".join(lines).rstrip() + "\n")
    path.chmod(path.stat().st_mode | 0o755)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--campaign", default="validated-conversions")
    parser.add_argument("--base-config", type=Path, default=DEFAULT_BASE_CONFIG)
    parser.add_argument("--output-dir", type=Path)
    parser.add_argument(
        "--route",
        action="append",
        default=[],
        help="Route name to generate. Repeat to generate multiple routes.",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        help="Print available campaigns and routes without writing configs.",
    )
    return parser.parse_args()


def render_available() -> str:
    lines: list[str] = []
    for campaign, routes in sorted(CAMPAIGNS.items()):
        lines.append(campaign)
        for route in routes:
            lines.append(f"  {route.name}: {len(route.tasks)} tasks")
    return "\n".join(lines) + "\n"


def main() -> int:
    args = parse_args()
    if args.list:
        print(render_available(), end="")
        return 0
    if args.output_dir is None:
        print("generate_tbench_campaign: --output-dir is required", file=sys.stderr)
        return 2
    try:
        manifest = write_campaign(
            campaign=args.campaign,
            base_config_path=args.base_config,
            output_dir=args.output_dir,
            route_names=args.route,
        )
    except Exception as exc:
        print(f"generate_tbench_campaign: {exc}", file=sys.stderr)
        return 2
    print(
        "Wrote {routes} route configs for {tasks} unique tasks: {manifest}".format(
            routes=manifest["summary"]["routes"],
            tasks=manifest["summary"]["uniqueTasks"],
            manifest=manifest["manifest"],
        )
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
