"""Harbor harness file attestation for pre-eval summaries."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from pre_eval_live_checks import combined_file_digest, file_sha256


DEFAULT_HARNESS_FILES = (
    Path("evals/harbor/roder_harbor_agent.py"),
    Path("evals/harbor/roder_harbor_agent_config.py"),
    Path("evals/harbor/roder_benchmark_guidance.py"),
    Path("evals/harbor/roder_config_shell.py"),
    Path("evals/harbor/roder_exec_shell.py"),
    Path("evals/harbor/roder_plan_first.py"),
    Path("evals/harbor/roder_run_summary_fragment.py"),
    Path("evals/harbor/install-roder.sh.j2"),
    Path("evals/harbor/build-prebuilt-roder.sh"),
    Path("evals/harbor/run-roder-tbench-full.sh"),
    Path("evals/harbor/run-roder-tbench-smoke.sh"),
    Path("evals/harbor/run-roder-pre-eval-diagnostics.sh"),
    Path("evals/harbor/preflight_tbench_images.py"),
    Path("evals/harbor/analyze_tbench_run.py"),
    Path("evals/harbor/compare_tbench_runs.py"),
    Path("evals/harbor/rerun_tbench_subset.py"),
    Path("evals/harbor/suggest_tbench_campaign.py"),
    Path("evals/harbor/summarize_tbench_campaigns.py"),
    Path("evals/harbor/generate_tbench_campaign.py"),
    Path("evals/harbor/validate_tbench_campaign.py"),
    Path("evals/harbor/tbench_campaign_handoff.py"),
    Path("evals/harbor/tbench_campaign_score_projection.py"),
    Path("evals/harbor/tbench_campaign_run_script.py"),
    Path("evals/harbor/tbench_campaign_script_commands.py"),
    Path("evals/harbor/validate_harbor_readiness.py"),
    Path("evals/harbor/validate_pre_eval_summary.py"),
    Path("evals/harbor/validate_pre_eval_tbench_diagnostics.py"),
    Path("evals/harbor/validate_tbench_analysis.py"),
    Path("evals/harbor/validate_tbench_launch_plan.py"),
    Path("evals/harbor/launch_plan_dependency_validation.py"),
    Path("evals/harbor/write_pre_eval_summary.py"),
    Path("evals/harbor/pre_eval_config_summary.py"),
    Path("evals/harbor/pre_eval_file_summary.py"),
    Path("evals/harbor/pre_eval_git_summary.py"),
    Path("evals/harbor/pre_eval_harness_summary.py"),
    Path("evals/harbor/pre_eval_image_preflight_validation.py"),
    Path("evals/harbor/pre_eval_live_checks.py"),
    Path("evals/harbor/pre_eval_run_summary.py"),
    Path("evals/harbor/pre_eval_summary_tbench_validation.py"),
    Path("evals/harbor/tbench_analysis_constants.py"),
    Path("evals/harbor/tbench_campaign_image_preflight.py"),
    Path("evals/harbor/tbench_diagnostic_contract.py"),
    Path("evals/harbor/test_suggest_tbench_campaign.py"),
    Path("evals/harbor/test_summarize_tbench_campaigns.py"),
    Path("evals/harbor/test_run_roder_pre_eval_diagnostics_args.py"),
    Path("evals/harbor/test_validate_tbench_campaign_run_script.py"),
    Path("evals/harbor/test_validate_tbench_campaign_run_script_guards.py"),
    Path("evals/harbor/test_validate_tbench_campaign_run_script_summary.py"),
    Path("evals/harbor/test_validate_tbench_campaign_handoff.py"),
    Path("evals/harbor/test_validate_tbench_campaign_score_projection.py"),
    Path("evals/harbor/test_run_roder_tbench_full_gate.py"),
    Path("evals/harbor/test_validate_pre_eval_summary_configs.py"),
    Path("evals/harbor/test_validate_pre_eval_summary_harness.py"),
    Path("evals/harbor/test_validate_tbench_launch_plan_harness.py"),
    Path("evals/harbor/test_validate_tbench_launch_plan_summary_copies.py"),
)


def harness_summary(paths: tuple[Path, ...] | list[Path] | None = None) -> dict[str, Any]:
    entries: list[dict[str, Any]] = []
    issues: list[str] = []
    for path in paths or DEFAULT_HARNESS_FILES:
        entry = file_digest_entry(path)
        entries.append(entry)
        if "error" in entry:
            issues.append(f"{entry['path']}: {entry['error']}")
    return {
        "status": "failed" if issues else "passed",
        "files": len(entries),
        "issues": issues,
        "combinedSha256": combined_file_digest(entries) if not issues else None,
        "entries": entries,
    }


def file_digest_entry(path: Path) -> dict[str, Any]:
    entry: dict[str, Any] = {"path": str(path)}
    try:
        size_bytes = len(path.read_bytes())
    except OSError as exc:
        return {**entry, "error": str(exc)}
    entry["sha256"] = file_sha256(path)
    entry["sizeBytes"] = size_bytes
    return entry
