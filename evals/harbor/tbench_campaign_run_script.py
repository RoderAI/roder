"""Run-script validation for generated Harbor Terminal-Bench campaigns."""

from __future__ import annotations

import os
import shlex
from pathlib import Path
from typing import Any

from tbench_campaign_script_commands import (
    array_literal_values,
    array_append_flag_values,
    analysis_command_tuples,
    baseline_validation_command_tuples,
    campaign_validation_command_tuples,
    command_flag_values,
    expected_analysis_tuples,
    expected_baseline_validation_tuples,
    expected_campaign_validation_tuples,
    expected_image_preflight_tuples,
    expected_route_job_dirs,
    format_tuple,
    has_flag_value,
    image_preflight_command_tuples,
    int_value,
    route_job_dir_values,
    script_flag_values,
    validate_final_campaign_validation_order,
    validate_route_command_order,
)


REQUIRED_RUN_SCRIPT_GUARDS = (
    "RODER_HARBOR_LIVE_TBENCH",
    "RODER_HARBOR_DRY_RUN",
    "preflight_tbench_images.py",
    "run-roder-pre-eval-diagnostics.sh",
    "validate_pre_eval_summary.py",
    "RODER_HARBOR_PRE_EVAL_SUMMARY",
    "RODER_HARBOR_REPLACE_JOB",
    "route_job_dirs=(",
    "harbor run --config",
    "analyze_tbench_run.py",
    "validate_tbench_analysis.py",
    "--require-analysis",
)
REQUIRED_RUN_SCRIPT_ROUTE_FIELDS = (
    ("config", "config path"),
    ("jobDir", "job directory"),
    ("analysisJson", "analysis JSON path"),
    ("analysisMarkdown", "analysis Markdown path"),
    ("analysisManifestDir", "analysis manifest directory"),
    ("imageManifest", "image manifest path"),
)
REQUIRED_SUMMARY_VALIDATION_FLAGS = (
    "--require-prebuilt",
    "--require-auth",
    "--require-tests",
    "--verify-harbor-configs",
    "--verify-harness-files",
    "--verify-prebuilt-binary",
    "--verify-auth-file",
    "--max-age-seconds",
)
FORBIDDEN_PRE_EVAL_FLAGS = ("--skip-tests",)
REQUIRED_RUN_SCRIPT_LINES = (
    ('dry_run="${RODER_HARBOR_DRY_RUN:-0}"', "runScript dry-run guard mismatch"),
    (
        'pre_eval_summary="${RODER_HARBOR_PRE_EVAL_SUMMARY:-}"',
        "runScript pre-eval summary guard mismatch",
    ),
    (
        'if [[ "$dry_run" != "1" && "${RODER_HARBOR_LIVE_TBENCH:-}" != "1" ]]; then',
        "runScript live-run guard mismatch",
    ),
    (
        'if [[ "${RODER_HARBOR_REPLACE_JOB:-}" != "1" ]]; then',
        "runScript replace-job guard mismatch",
    ),
)


def validate_run_script(result: Any, value: Any, *, routes: list[Any]) -> None:
    if not isinstance(value, str) or not value:
        result.add("runScript is missing")
        return
    path = Path(value)
    if not path.is_file() or not os.access(path, os.X_OK):
        result.add("runScript cannot be executed")
        return
    try:
        text = path.read_text()
    except OSError as exc:
        result.add(f"runScript cannot be read: {exc}")
        return
    for guard in REQUIRED_RUN_SCRIPT_GUARDS:
        if guard not in text:
            result.add(f"runScript missing required guard: {guard}")
    try:
        tokens = shlex.split(text, comments=True)
    except ValueError as exc:
        result.add(f"runScript cannot be tokenized: {exc}")
        return
    validate_run_script_routes(result, tokens, text, routes)


def validate_run_script_routes(
    result: Any,
    tokens: list[str],
    text: str,
    routes: list[Any],
) -> None:
    expected_configs = sorted(
        str(route.get("config"))
        for route in routes
        if isinstance(route, dict) and isinstance(route.get("config"), str)
    )
    actual_configs = sorted(command_flag_values(tokens, "harbor", "run", "--config"))
    if actual_configs != expected_configs:
        result.add(
            "runScript harbor configs mismatch: expected "
            + ", ".join(expected_configs)
            + "; got "
            + ", ".join(actual_configs)
        )
    actual_preflight_configs = sorted(
        script_flag_values(tokens, "evals/harbor/preflight_tbench_images.py", "--config")
    )
    if actual_preflight_configs != expected_configs:
        result.add(
            "runScript image preflight configs mismatch: expected "
            + ", ".join(expected_configs)
            + "; got "
            + ", ".join(actual_preflight_configs)
        )
    expected_preflight = sorted(expected_image_preflight_tuples(routes))
    actual_preflight = sorted(image_preflight_command_tuples(tokens))
    if actual_preflight != expected_preflight:
        result.add(
            "runScript image preflight commands mismatch: expected "
            + "; ".join(format_tuple(item) for item in expected_preflight)
            + "; got "
            + "; ".join(format_tuple(item) for item in actual_preflight)
        )
    expected_job_dirs = sorted(expected_route_job_dirs(routes))
    actual_job_dirs = sorted(route_job_dir_values(text))
    if actual_job_dirs != expected_job_dirs:
        result.add(
            "runScript route job dirs mismatch: expected "
            + ", ".join(expected_job_dirs)
            + "; got "
            + ", ".join(actual_job_dirs)
        )
    actual_pre_eval_configs = sorted(
        array_append_flag_values(text, "pre_eval_args", "--config")
    )
    if actual_pre_eval_configs != expected_configs:
        result.add(
            "runScript pre-eval configs mismatch: expected "
            + ", ".join(expected_configs)
            + "; got "
            + ", ".join(actual_pre_eval_configs)
        )
    actual_summary_configs = sorted(
        array_append_flag_values(text, "summary_validation_args", "--require-config")
    )
    if actual_summary_configs != expected_configs:
        result.add(
            "runScript summary required configs mismatch: expected "
            + ", ".join(expected_configs)
            + "; got "
            + ", ".join(actual_summary_configs)
        )
    validate_required_invocations(result, text)
    expected_analysis = sorted(expected_analysis_tuples(routes))
    actual_analysis = sorted(analysis_command_tuples(tokens))
    if actual_analysis != expected_analysis:
        result.add(
            "runScript analysis commands mismatch: expected "
            + "; ".join(format_tuple(item) for item in expected_analysis)
            + "; got "
            + "; ".join(format_tuple(item) for item in actual_analysis)
        )
    expected_baseline = sorted(expected_baseline_validation_tuples(routes))
    actual_baseline = sorted(baseline_validation_command_tuples(tokens))
    if actual_baseline != expected_baseline:
        result.add(
            "runScript baseline validation commands mismatch: expected "
            + "; ".join(format_tuple(item) for item in expected_baseline)
            + "; got "
            + "; ".join(format_tuple(item) for item in actual_baseline)
        )
    validate_route_command_order(result, tokens, routes)
    validate_final_campaign_validation_order(result, tokens, routes)
    expected_campaign_validation = sorted(expected_campaign_validation_tuples())
    actual_campaign_validation = sorted(campaign_validation_command_tuples(tokens))
    if actual_campaign_validation != expected_campaign_validation:
        result.add(
            "runScript campaign validation commands mismatch: expected "
            + "; ".join(format_tuple(item) for item in expected_campaign_validation)
            + "; got "
            + "; ".join(format_tuple(item) for item in actual_campaign_validation)
        )
    for route in routes:
        if not isinstance(route, dict):
            continue
        name = str(route.get("name") or "<missing>")
        for field, label in REQUIRED_RUN_SCRIPT_ROUTE_FIELDS:
            value = route.get(field)
            if not isinstance(value, str) or not value:
                continue
            if value not in tokens:
                result.add(f"runScript missing route {name} {label}")
        task_count = int_value(route.get("taskCount"))
        if task_count and not has_flag_value(tokens, "--expected-trials", str(task_count)):
            result.add(f"runScript missing route {name} expected-trials gate")


def validate_required_invocations(result: Any, text: str) -> None:
    lines = {line.strip() for line in text.splitlines()}
    for line, issue in REQUIRED_RUN_SCRIPT_LINES:
        if line not in lines:
            result.add(issue)
    if 'evals/harbor/run-roder-pre-eval-diagnostics.sh "${pre_eval_args[@]}"' not in lines:
        result.add("runScript pre-eval diagnostics invocation mismatch")
    if (
        'python3 evals/harbor/validate_pre_eval_summary.py "${summary_validation_args[@]}"'
        not in lines
    ):
        result.add("runScript summary validation invocation mismatch")
    summary_args = set(array_literal_values(text, "summary_validation_args"))
    for flag in REQUIRED_SUMMARY_VALIDATION_FLAGS:
        if flag not in summary_args:
            result.add(f"runScript summary validation args missing: {flag}")
    pre_eval_args = set(array_literal_values(text, "pre_eval_args"))
    for flag in FORBIDDEN_PRE_EVAL_FLAGS:
        if flag in pre_eval_args:
            result.add("runScript pre-eval tests cannot be skipped")
