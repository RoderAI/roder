"""Handoff metadata for generated Harbor Terminal-Bench campaigns."""

from __future__ import annotations

from pathlib import Path
from typing import Any


def pre_eval_handoff(output_dir: Path) -> dict[str, str]:
    pre_eval_dir = output_dir / "pre-eval"
    return {
        "outputDir": str(pre_eval_dir),
        "summary": str(pre_eval_dir / "pre-eval-summary.json"),
    }


def validate_pre_eval_handoff(
    result: Any,
    value: Any,
    *,
    manifest_path: Path,
    run_script: Any = None,
) -> None:
    expected = pre_eval_handoff(manifest_path.parent)
    if not isinstance(value, dict):
        result.add("preEval is missing")
        return
    for field, expected_value in expected.items():
        if value.get(field) != expected_value:
            result.add(f"preEval {field} mismatch")
    validate_pre_eval_run_script(result, run_script)


def validate_pre_eval_run_script(result: Any, run_script: Any) -> None:
    if not isinstance(run_script, str) or not run_script:
        return
    try:
        text = Path(run_script).read_text()
    except OSError:
        return
    required_lines = {
        'pre_eval_output_dir="${RODER_HARBOR_PRE_EVAL_OUTPUT_DIR:-$PREFLIGHT_DIR/pre-eval}"': (
            "runScript pre-eval output dir mismatch"
        ),
        'pre_eval_summary="$pre_eval_output_dir/pre-eval-summary.json"': (
            "runScript pre-eval summary path mismatch"
        ),
    }
    lines = {line.strip() for line in text.splitlines()}
    for line, issue in required_lines.items():
        if line not in lines:
            result.add(issue)
