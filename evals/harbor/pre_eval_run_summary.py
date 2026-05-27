"""Eval-run summary helpers for Harbor pre-eval handoff artifacts."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from tbench_diagnostic_contract import (
    EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS,
    EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES,
    EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS,
)


def eval_run_summary(
    directory: Path | None,
    *,
    expected_fixtures: tuple[str, ...] = (),
) -> dict[str, Any] | None:
    summary: dict[str, Any] = {
        "fixtures": 0,
        "passed": 0,
        "failed": 0,
    }
    if directory is None:
        return {"status": "missing", **summary}
    path = directory / "eval-run.json"
    summary.update(
        {
            "evalRun": str(path),
            "evalReport": str(directory / "eval-report.md"),
        }
    )
    if not path.exists():
        return {"status": "missing", **summary}
    try:
        data = json.loads(path.read_text())
    except Exception as exc:
        return {"status": "failed", "error": str(exc), **summary}
    results = data.get("results", [])
    if not isinstance(results, list):
        return {"status": "failed", "error": "results must be a list", **summary}

    fixture_id_list = [fixture_id(result) for result in results]
    fixture_ids = sorted(fixture_id_list)
    fixture_id_set = set(fixture_id_list)
    duplicate_fixtures = sorted(
        fixture
        for fixture in fixture_id_set
        if fixture_id_list.count(fixture) > 1
    )
    missing_expected = [
        fixture for fixture in expected_fixtures if fixture not in fixture_id_set
    ]
    unexpected_fixtures = (
        sorted(fixture_id_set.difference(expected_fixtures))
        if expected_fixtures
        else []
    )
    passed = sum(
        1 for result in results if result.get("report", {}).get("outcome") == "pass"
    )
    failed = len(results) - passed
    missing_verification = sum(
        1
        for result in results
        if metric_value(result, "verification_completed") < 1
    )
    results_by_fixture = {fixture_id(result): result for result in results}
    verifier_command_checks_required = sum(
        EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS.values()
    )
    verifier_command_checks_completed = sum(
        int(metric_value(results_by_fixture[fixture], "verifier_command_checks_completed"))
        for fixture in EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS
        if fixture in results_by_fixture
    )
    verifier_command_check_fixtures = {
        fixture: {
            "required": int(
                metric_value(
                    results_by_fixture[fixture],
                    "verifier_command_checks_required",
                )
            )
            if fixture in results_by_fixture
            else 0,
            "completed": int(
                metric_value(
                    results_by_fixture[fixture],
                    "verifier_command_checks_completed",
                )
            )
            if fixture in results_by_fixture
            else 0,
        }
        for fixture, expected_count in EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS.items()
    }
    missing_command_checks = [
        fixture
        for fixture, expected_count in EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS.items()
        if fixture in results_by_fixture
        and (
            metric_value(results_by_fixture[fixture], "verifier_command_checks_required")
            != expected_count
            or metric_value(
                results_by_fixture[fixture], "verifier_command_checks_completed"
            )
            < expected_count
        )
    ]
    task_ledger_checkpoint_fixtures = {
        fixture: {
            "updates": int(
                metric_value(
                    results_by_fixture[fixture],
                    "task_ledger_updates",
                )
            )
            if fixture in results_by_fixture
            else 0,
            "completed": int(
                metric_value(
                    results_by_fixture[fixture],
                    "task_ledger_completed",
                )
            )
            if fixture in results_by_fixture
            else 0,
        }
        for fixture in EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS
    }
    missing_task_ledger_checkpoints = [
        fixture
        for fixture, expected in EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS.items()
        if fixture in results_by_fixture
        and (
            metric_value(results_by_fixture[fixture], "task_ledger_updates")
            < expected["updates"]
            or metric_value(results_by_fixture[fixture], "task_ledger_completed")
            < expected["completed"]
        )
    ]
    unknown_errors = int(
        sum(metric_value(result, "reliability_unknown_errors") for result in results)
    )
    status = (
        "passed"
        if results
        and failed == 0
        and missing_verification == 0
        and not missing_command_checks
        and not missing_task_ledger_checkpoints
        and unknown_errors == 0
        and not missing_expected
        and not unexpected_fixtures
        and not duplicate_fixtures
        else "failed"
    )
    summary.update(
        {
            "status": status,
            "fixtures": len(results),
            "fixtureIds": fixture_ids,
            "missingExpectedFixtures": missing_expected,
            "unexpectedFixtures": unexpected_fixtures,
            "duplicateFixtures": duplicate_fixtures,
            "passed": passed,
            "failed": failed,
            "missingVerification": missing_verification,
            "missingCommandChecks": missing_command_checks,
            "verifierCommandChecksRequired": verifier_command_checks_required,
            "verifierCommandChecksCompleted": verifier_command_checks_completed,
            "verifierCommandCheckFixtures": verifier_command_check_fixtures,
            "missingTaskLedgerCheckpoints": missing_task_ledger_checkpoints,
            "taskLedgerCheckpointFixtures": task_ledger_checkpoint_fixtures,
            "unknownReliabilityErrors": unknown_errors,
        }
    )
    return summary


def tbench_eval_run_summary(directory: Path | None) -> dict[str, Any] | None:
    return eval_run_summary(
        directory,
        expected_fixtures=EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES,
    )


def fixture_id(result: Any) -> str:
    if isinstance(result, dict) and result.get("fixtureId"):
        return str(result["fixtureId"])
    return "<unknown>"


def metric_value(result: dict[str, Any], name: str) -> float:
    report = result.get("report") if isinstance(result.get("report"), dict) else {}
    metrics = report.get("metrics") if isinstance(report.get("metrics"), list) else []
    for metric in metrics:
        if not isinstance(metric, dict) or metric.get("name") != name:
            continue
        try:
            return float(metric.get("value") or 0)
        except (TypeError, ValueError):
            return 0.0
    return 0.0
