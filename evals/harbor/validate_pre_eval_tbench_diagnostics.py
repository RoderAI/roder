#!/usr/bin/env python3
"""Validate the local TBench diagnostic eval-run artifact."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from tbench_diagnostic_contract import (
    EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS,
    EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES,
    EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS,
)


class DiagnosticsSummary:
    def __init__(
        self,
        *,
        fixtures: int,
        missing_fixtures: list[str],
        unexpected_fixtures: list[str],
        duplicate_fixtures: list[str],
        failed_fixtures: list[str],
        missing_verification: list[str],
        missing_command_checks: list[str],
        missing_task_ledger_checkpoints: list[str],
        command_checks_required: int,
        command_checks_completed: int,
        unknown_errors: int,
    ) -> None:
        self.fixtures = fixtures
        self.missing_fixtures = missing_fixtures
        self.unexpected_fixtures = unexpected_fixtures
        self.duplicate_fixtures = duplicate_fixtures
        self.failed_fixtures = failed_fixtures
        self.missing_verification = missing_verification
        self.missing_command_checks = missing_command_checks
        self.missing_task_ledger_checkpoints = missing_task_ledger_checkpoints
        self.command_checks_required = command_checks_required
        self.command_checks_completed = command_checks_completed
        self.unknown_errors = unknown_errors

    @property
    def ok(self) -> bool:
        return not (
            self.missing_fixtures
            or self.unexpected_fixtures
            or self.duplicate_fixtures
            or self.failed_fixtures
            or self.missing_verification
            or self.missing_command_checks
            or self.missing_task_ledger_checkpoints
            or self.unknown_errors
        )


def load_json(path: Path) -> dict[str, Any]:
    data = json.loads(path.read_text())
    if not isinstance(data, dict):
        raise ValueError(f"{path} must contain a JSON object")
    return data


def validate_run(data: dict[str, Any]) -> DiagnosticsSummary:
    results = data.get("results")
    if not isinstance(results, list):
        results = []
    fixture_id_list = [fixture_id(result) for result in results]
    fixture_ids = set(fixture_id_list)
    duplicate_fixtures = sorted(
        fixture
        for fixture in fixture_ids
        if fixture_id_list.count(fixture) > 1
    )
    missing_fixtures = [
        fixture
        for fixture in EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES
        if fixture not in fixture_ids
    ]
    unexpected_fixtures = sorted(
        fixture_ids.difference(EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES)
    )
    failed = [
        fixture_id(result)
        for result in results
        if report(result).get("outcome") != "pass"
    ]
    missing_verification = [
        fixture_id(result)
        for result in results
        if metric(result, "verification_completed") < 1
    ]
    results_by_fixture = {fixture_id(result): result for result in results}
    missing_command_checks = []
    command_checks_required = sum(EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS.values())
    command_checks_completed = 0
    for fixture, expected_count in EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS.items():
        result = results_by_fixture.get(fixture)
        if result is None:
            continue
        required = int(metric(result, "verifier_command_checks_required"))
        completed = int(metric(result, "verifier_command_checks_completed"))
        command_checks_completed += completed
        if required != expected_count or completed < expected_count:
            missing_command_checks.append(fixture)
    missing_task_ledger_checkpoints = []
    for fixture, expected in EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS.items():
        result = results_by_fixture.get(fixture)
        if result is None:
            continue
        updates = int(metric(result, "task_ledger_updates"))
        completed = int(metric(result, "task_ledger_completed"))
        if updates < int(expected["updates"]) or completed < int(expected["completed"]):
            missing_task_ledger_checkpoints.append(fixture)
    unknown_errors = int(
        sum(metric(result, "reliability_unknown_errors") for result in results)
    )
    return DiagnosticsSummary(
        fixtures=len(results),
        missing_fixtures=missing_fixtures,
        unexpected_fixtures=unexpected_fixtures,
        duplicate_fixtures=duplicate_fixtures,
        failed_fixtures=failed,
        missing_verification=missing_verification,
        missing_command_checks=missing_command_checks,
        missing_task_ledger_checkpoints=missing_task_ledger_checkpoints,
        command_checks_required=command_checks_required,
        command_checks_completed=command_checks_completed,
        unknown_errors=unknown_errors,
    )


def fixture_id(result: Any) -> str:
    if isinstance(result, dict) and result.get("fixtureId"):
        return str(result["fixtureId"])
    return "<unknown>"


def report(result: Any) -> dict[str, Any]:
    if not isinstance(result, dict):
        return {}
    value = result.get("report")
    return value if isinstance(value, dict) else {}


def metric(result: Any, name: str) -> float:
    metrics = report(result).get("metrics")
    if not isinstance(metrics, list):
        return 0.0
    for item in metrics:
        if not isinstance(item, dict) or item.get("name") != name:
            continue
        try:
            return float(item.get("value") or 0)
        except (TypeError, ValueError):
            return 0.0
    return 0.0


def render_failure(summary: DiagnosticsSummary) -> str:
    lines = ["TBench diagnostics failed"]
    if summary.missing_fixtures:
        lines.append("missing fixtures: " + ", ".join(summary.missing_fixtures))
    if summary.unexpected_fixtures:
        lines.append(
            "unexpected fixtures: " + ", ".join(summary.unexpected_fixtures)
        )
    if summary.duplicate_fixtures:
        lines.append("duplicate fixtures: " + ", ".join(summary.duplicate_fixtures))
    if summary.failed_fixtures:
        lines.append("failed fixtures: " + ", ".join(summary.failed_fixtures))
    if summary.missing_verification:
        lines.append(
            "missing verification: " + ", ".join(summary.missing_verification)
        )
    if summary.missing_command_checks:
        lines.append(
            "missing command checks: " + ", ".join(summary.missing_command_checks)
        )
    if summary.missing_task_ledger_checkpoints:
        lines.append(
            "missing task ledger checkpoints: "
            + ", ".join(summary.missing_task_ledger_checkpoints)
        )
    if summary.unknown_errors:
        lines.append(f"unknown reliability errors: {summary.unknown_errors:g}")
    return "\n".join(lines)


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("eval_run", type=Path)
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        summary = validate_run(load_json(args.eval_run))
    except Exception as exc:
        print(f"validate_pre_eval_tbench_diagnostics: {exc}", file=sys.stderr)
        return 2
    if not summary.ok:
        print(render_failure(summary), file=sys.stderr)
        return 1
    print(f"TBench diagnostics passed: {summary.fixtures} fixtures")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
