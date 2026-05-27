"""Shared test data for local Terminal-Bench diagnostic fixtures."""

from __future__ import annotations

from copy import deepcopy
from typing import Any

from tbench_diagnostic_contract import (
    EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS,
    EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES,
    EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS,
)


def diagnostic_fixture_ids() -> list[str]:
    return list(EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES)


def diagnostic_command_check_fixture_ids() -> set[str]:
    return set(EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS)


def default_metrics_for_fixture(fixture_id: str) -> list[dict[str, Any]]:
    metrics: list[dict[str, Any]] = [
        {"name": "verification_completed", "value": 1.0},
        {"name": "reliability_unknown_errors", "value": 0.0},
    ]
    command_checks = EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS.get(fixture_id)
    if command_checks is not None:
        metrics.extend(
            [
                {
                    "name": "verifier_command_checks_required",
                    "value": float(command_checks),
                },
                {
                    "name": "verifier_command_checks_completed",
                    "value": float(command_checks),
                },
            ]
        )
    checkpoint = EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS.get(fixture_id)
    if checkpoint is not None:
        metrics.extend(
            [
                {"name": "task_ledger_updates", "value": float(checkpoint["updates"])},
                {
                    "name": "task_ledger_completed",
                    "value": float(checkpoint["completed"]),
                },
            ]
        )
    return metrics


def diagnostic_eval_result(
    fixture_id: str,
    *,
    outcome: str = "pass",
    metrics: list[dict[str, Any]] | None = None,
) -> dict[str, Any]:
    return {
        "fixtureId": fixture_id,
        "report": {
            "outcome": outcome,
            "metrics": metrics if metrics is not None else default_metrics_for_fixture(fixture_id),
        },
    }


def passing_diagnostic_results() -> list[dict[str, Any]]:
    return [diagnostic_eval_result(fixture_id) for fixture_id in diagnostic_fixture_ids()]


def command_check_fixture_summary() -> dict[str, dict[str, int]]:
    return {
        fixture_id: {
            "required": expected_count,
            "completed": expected_count,
        }
        for fixture_id, expected_count in EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS.items()
    }


def task_ledger_checkpoint_summary() -> dict[str, dict[str, int]]:
    return deepcopy(EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS)


def passing_tbench_diagnostics_summary() -> dict[str, Any]:
    fixtures = diagnostic_fixture_ids()
    command_checks = command_check_fixture_summary()
    return {
        "status": "passed",
        "fixtures": len(fixtures),
        "passed": len(fixtures),
        "failed": 0,
        "fixtureIds": fixtures,
        "missingExpectedFixtures": [],
        "unexpectedFixtures": [],
        "duplicateFixtures": [],
        "missingCommandChecks": [],
        "missingVerification": 0,
        "verifierCommandChecksRequired": sum(
            EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS.values()
        ),
        "verifierCommandChecksCompleted": sum(
            item["completed"] for item in command_checks.values()
        ),
        "verifierCommandCheckFixtures": command_checks,
        "missingTaskLedgerCheckpoints": [],
        "taskLedgerCheckpointFixtures": task_ledger_checkpoint_summary(),
        "unknownReliabilityErrors": 0,
    }
