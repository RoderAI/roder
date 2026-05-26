#!/usr/bin/env python3
"""Validate a Harbor analyzer report against a clean-run baseline."""

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

from analyze_tbench_run import analyze_job  # noqa: E402
from tbench_analysis_constants import HARNESS_ERROR_CLASSES  # noqa: E402


DEFAULT_BASELINE = Path("evals/harbor/tbench-clean-baseline.json")
CORE_METRICS = {
    "analysis_consistency",
    "clean",
    "harbor_n_errors",
    "harbor_n_trials",
    "harness_error_total",
    "passes",
    "scored_failures",
    "scored_trials",
    "task_dirs",
}
EXPECTED_TRIAL_METRICS = {"harbor_n_trials", "scored_trials", "task_dirs"}
KNOWN_ANALYSIS_CLASSES = HARNESS_ERROR_CLASSES | {
    "deadline_finalized",
    "internal_deadline_timeout",
    "pass",
    "provider_policy_block",
    "scored_fail",
    "soft_timeout",
    "soft_timeout_fail",
    "soft_timeout_pass",
    "unknown",
}


def load_json(path: Path) -> dict[str, Any]:
    value = json.loads(path.read_text())
    if not isinstance(value, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return value


def load_analysis(path: Path) -> dict[str, Any]:
    if path.is_dir():
        return analyze_job(path, group_scored_failures=True)
    data = load_json(path)
    if "stats" not in data:
        raise ValueError(f"{path} is neither a Harbor job dir nor analyzer JSON")
    return data


def compare_analysis_to_baseline(
    analysis: dict[str, Any],
    baseline: dict[str, Any],
    *,
    expected_trials: int | None = None,
) -> dict[str, Any]:
    metrics = analysis_metrics(analysis)
    rows = []
    status = "ok"
    for expectation in baseline_expectations(baseline, expected_trials=expected_trials):
        metric = str(expectation.get("metric") or "")
        current = metrics.get(metric, 0)
        if not is_known_metric(metric, metrics):
            rows.append(
                {
                    "metric": metric or "<missing>",
                    "current": current,
                    "status": "blocked",
                    "reason": "unknown_metric",
                }
            )
            status = "blocked"
            continue
        max_count = count_value(expectation.get("maxCount"))
        min_count = count_value(expectation.get("minCount"))
        row_status = "ok"
        if max_count is not None and current > max_count:
            row_status = "blocked"
        if min_count is not None and current < min_count:
            row_status = "blocked"
        if row_status == "blocked":
            status = "blocked"
        row = {
            "metric": metric,
            "current": current,
            "status": row_status,
        }
        if max_count is not None:
            row["maxCount"] = max_count
        if min_count is not None:
            row["minCount"] = min_count
        rows.append(row)
    return {
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "status": status,
        "rows": rows,
        "metrics": metrics,
    }


def is_known_metric(metric: str, metrics: dict[str, int]) -> bool:
    return (
        metric in metrics
        or metric in CORE_METRICS
        or metric in KNOWN_ANALYSIS_CLASSES
    )


def baseline_expectations(
    baseline: dict[str, Any],
    *,
    expected_trials: int | None = None,
) -> list[dict[str, Any]]:
    expectations = [
        expectation
        for expectation in baseline.get("expectations", [])
        if isinstance(expectation, dict)
    ]
    explicit_metrics = {str(expectation.get("metric") or "") for expectation in expectations}
    if "analysis_consistency" not in explicit_metrics:
        expectations.append({"metric": "analysis_consistency", "minCount": 1})
    if "clean" not in explicit_metrics:
        expectations.append({"metric": "clean", "minCount": 1})
    for metric in sorted(HARNESS_ERROR_CLASSES.difference(explicit_metrics)):
        expectations.append({"metric": metric, "maxCount": 0})
    if expected_trials is not None:
        expectations = [
            expected_trial_expectation(expectation, expected_trials)
            for expectation in expectations
        ]
    return expectations


def expected_trial_expectation(
    expectation: dict[str, Any],
    expected_trials: int,
) -> dict[str, Any]:
    metric = str(expectation.get("metric") or "")
    if metric not in EXPECTED_TRIAL_METRICS or "minCount" not in expectation:
        return expectation
    updated = dict(expectation)
    updated["minCount"] = expected_trials
    return updated


def analysis_metrics(analysis: dict[str, Any]) -> dict[str, int]:
    stats = analysis.get("stats") if isinstance(analysis.get("stats"), dict) else {}
    harbor = stats.get("harbor") if isinstance(stats.get("harbor"), dict) else {}
    classes = analysis.get("classes") if isinstance(analysis.get("classes"), dict) else {}
    harness_classes = (
        stats.get("harness_error_classes")
        if isinstance(stats.get("harness_error_classes"), dict)
        else {}
    )

    passes = as_int(stats.get("passes"))
    scored_failures = as_int(stats.get("scored_failures"))
    class_passes = class_count(classes, "pass")
    class_scored_failures = class_count(classes, "scored_fail")
    metrics: dict[str, int] = {
        "analysis_consistency": int(
            passes == class_passes and scored_failures == class_scored_failures
        ),
        "clean": 1 if analysis.get("clean") is True else 0,
        "harbor_n_errors": as_int(harbor.get("n_errors")),
        "harbor_n_trials": as_int(harbor.get("n_trials")),
        "task_dirs": as_int(stats.get("task_dirs")),
        "passes": passes,
        "scored_failures": scored_failures,
        "scored_trials": passes + scored_failures,
        "harness_error_total": sum(as_int(value) for value in harness_classes.values()),
    }

    for name, entries in classes.items():
        if isinstance(entries, list):
            metrics[str(name)] = len(entries)
    for name, value in harness_classes.items():
        metrics[str(name)] = as_int(value)
    return metrics


def class_count(classes: dict[str, Any], name: str) -> int:
    entries = classes.get(name)
    return len(entries) if isinstance(entries, list) else 0


def as_int(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0


def count_value(value: Any) -> int | None:
    if value is None:
        return None
    return as_int(value)


def render_markdown(comparison: dict[str, Any]) -> str:
    lines = [
        "# Harbor TBench Baseline Validation",
        "",
        f"- Status: `{comparison['status']}`",
        "",
        "| Metric | Current | Min | Max | Status |",
        "| --- | ---: | ---: | ---: | --- |",
    ]
    for row in comparison["rows"]:
        lines.append(
            "| `{}` | {} | {} | {} | `{}` |".format(
                row["metric"],
                row["current"],
                row.get("minCount", "-"),
                row.get("maxCount", "-"),
                row["status"],
            )
        )
    return "\n".join(lines).rstrip() + "\n"


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("analysis_or_job", type=Path)
    parser.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE)
    parser.add_argument("--expected-trials", type=int)
    parser.add_argument("--json", dest="json_path", type=Path)
    parser.add_argument("--markdown", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        analysis = load_analysis(args.analysis_or_job)
        baseline = load_json(args.baseline)
        comparison = compare_analysis_to_baseline(
            analysis,
            baseline,
            expected_trials=args.expected_trials,
        )
    except Exception as exc:
        print(f"validate_tbench_analysis: {exc}", file=sys.stderr)
        return 2

    if args.json_path:
        args.json_path.parent.mkdir(parents=True, exist_ok=True)
        args.json_path.write_text(json.dumps(comparison, indent=2) + "\n")

    markdown = render_markdown(comparison)
    if args.markdown:
        args.markdown.parent.mkdir(parents=True, exist_ok=True)
        args.markdown.write_text(markdown)

    if not args.json_path and not args.markdown:
        print(markdown, end="")

    return 0 if comparison["status"] == "ok" else 1


if __name__ == "__main__":
    raise SystemExit(main())
