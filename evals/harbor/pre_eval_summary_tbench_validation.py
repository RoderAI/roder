"""TBench diagnostic checks for Harbor pre-eval summary validation."""

from __future__ import annotations

from typing import Any

from tbench_diagnostic_contract import (
    EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS,
    EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES,
    EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS,
)


def validate_tbench_diagnostics(issues: list[str], check: Any) -> None:
    if not isinstance(check, dict):
        issues.append("tbenchDiagnostics check missing")
        return
    if check.get("status") != "passed":
        issues.append(f"tbenchDiagnostics status is {check.get('status') or '<missing>'}")
    fixture_ids = list_value(check.get("fixtureIds"))
    if not fixture_ids:
        issues.append("TBench diagnostic fixtureIds are missing")
    else:
        duplicate_fixture_ids = sorted(
            {
                str(fixture)
                for fixture in fixture_ids
                if fixture_ids.count(fixture) > 1
            }
        )
        if duplicate_fixture_ids:
            issues.append(
                "duplicate TBench diagnostic fixture IDs: "
                + ", ".join(duplicate_fixture_ids)
            )
        fixture_id_set = {str(fixture) for fixture in fixture_ids}
        missing_fixture_ids = [
            fixture
            for fixture in EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES
            if fixture not in fixture_id_set
        ]
        if missing_fixture_ids:
            issues.append(
                "missing TBench diagnostic fixture IDs: "
                + ", ".join(missing_fixture_ids)
            )
        unexpected_fixture_ids = sorted(
            fixture_id_set.difference(EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES)
        )
        if unexpected_fixture_ids:
            issues.append(
                "unexpected TBench diagnostic fixture IDs: "
                + ", ".join(unexpected_fixture_ids)
            )
    validate_tbench_diagnostic_counts(issues, check, fixture_ids)
    validate_tbench_diagnostic_fixture_fields(issues, check)
    validate_tbench_diagnostic_command_checks(issues, check)
    validate_tbench_command_check_fixtures(issues, check)
    validate_tbench_task_ledger_checkpoints(issues, check)
    if int_value(check.get("missingVerification")) > 0:
        issues.append(f"TBench diagnostics missing verification: {check.get('missingVerification')}")
    if int_value(check.get("unknownReliabilityErrors")) > 0:
        issues.append(
            "TBench diagnostics unknown reliability errors: "
            f"{check.get('unknownReliabilityErrors')}"
        )


def validate_tbench_diagnostic_counts(
    issues: list[str],
    check: dict[str, Any],
    fixture_ids: list[Any],
) -> None:
    if any(field not in check for field in ("fixtures", "passed", "failed")):
        issues.append("TBench diagnostics count fields are missing")
        return
    fixture_count = int_value(check.get("fixtures"))
    passed_count = int_value(check.get("passed"))
    failed_count = int_value(check.get("failed"))
    if fixture_ids and fixture_count != len(fixture_ids):
        issues.append(
            f"TBench diagnostics fixture count mismatch: {fixture_count} != {len(fixture_ids)}"
        )
    if fixture_count < len(EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES):
        issues.append(
            "TBench diagnostics fixture count below expected: "
            f"{fixture_count}/{len(EXPECTED_TBENCH_DIAGNOSTIC_FIXTURES)}"
        )
    if passed_count != fixture_count:
        issues.append(
            f"TBench diagnostics passed count mismatch: {passed_count}/{fixture_count}"
        )
    if failed_count != 0:
        issues.append(f"TBench diagnostics failed count is not zero: {failed_count}")


def validate_tbench_diagnostic_fixture_fields(
    issues: list[str],
    check: dict[str, Any],
) -> None:
    missing = list_value(check.get("missingExpectedFixtures"))
    if missing:
        issues.append("missing TBench diagnostic fixtures: " + ", ".join(str(item) for item in missing))
    for field, label in (
        ("unexpectedFixtures", "unexpected"),
        ("duplicateFixtures", "duplicate"),
    ):
        if field not in check:
            issues.append(f"TBench diagnostics {field} field is missing")
            continue
        stale = list_value(check.get(field))
        if stale:
            issues.append(f"{label} TBench diagnostic fixtures: " + ", ".join(str(item) for item in stale))


def validate_tbench_diagnostic_command_checks(
    issues: list[str],
    check: dict[str, Any],
) -> None:
    missing_command_checks = list_value(check.get("missingCommandChecks"))
    if missing_command_checks:
        issues.append(
            "missing TBench diagnostic command checks: "
            + ", ".join(str(item) for item in missing_command_checks)
        )
    if (
        "verifierCommandChecksRequired" not in check
        or "verifierCommandChecksCompleted" not in check
    ):
        issues.append("TBench diagnostics verifier command check totals are missing")
        return
    command_checks_required = int_value(check.get("verifierCommandChecksRequired"))
    command_checks_completed = int_value(check.get("verifierCommandChecksCompleted"))
    expected_command_checks = sum(EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS.values())
    if command_checks_required != expected_command_checks:
        issues.append(
            "TBench diagnostics verifier command checks required mismatch: "
            f"{command_checks_required} != {expected_command_checks}"
        )
    if command_checks_completed < expected_command_checks:
        issues.append(
            "TBench diagnostics verifier command checks below expected: "
            f"{command_checks_completed}/{expected_command_checks}"
        )
    if command_checks_completed < command_checks_required:
        issues.append(
            "TBench diagnostics verifier command checks incomplete: "
            f"{command_checks_completed}/{command_checks_required}"
        )


def validate_tbench_command_check_fixtures(
    issues: list[str],
    check: dict[str, Any],
) -> None:
    entries = check.get("verifierCommandCheckFixtures")
    if not isinstance(entries, dict):
        issues.append("TBench diagnostics verifier command check fixture map is missing")
        return
    fixture_required_total = 0
    fixture_completed_total = 0
    for entry in entries.values():
        if isinstance(entry, dict):
            fixture_required_total += int_value(entry.get("required"))
            fixture_completed_total += int_value(entry.get("completed"))
    for fixture, expected_count in EXPECTED_TBENCH_DIAGNOSTIC_COMMAND_CHECKS.items():
        entry = entries.get(fixture)
        if not isinstance(entry, dict):
            issues.append(
                f"TBench diagnostics verifier command check fixture missing: {fixture}"
            )
            continue
        required = int_value(entry.get("required"))
        completed = int_value(entry.get("completed"))
        if required != expected_count:
            issues.append(
                "TBench diagnostics verifier command check fixture required mismatch: "
                f"{fixture} {required} != {expected_count}"
            )
        if completed < expected_count:
            issues.append(
                "TBench diagnostics verifier command check fixture incomplete: "
                f"{fixture} {completed}/{expected_count}"
            )
    if "verifierCommandChecksRequired" in check:
        command_checks_required = int_value(check.get("verifierCommandChecksRequired"))
        if fixture_required_total != command_checks_required:
            issues.append(
                "TBench diagnostics verifier command check fixture required total mismatch: "
                f"{fixture_required_total} != {command_checks_required}"
            )
    if "verifierCommandChecksCompleted" in check:
        command_checks_completed = int_value(check.get("verifierCommandChecksCompleted"))
        if fixture_completed_total != command_checks_completed:
            issues.append(
                "TBench diagnostics verifier command check fixture completed total mismatch: "
                f"{fixture_completed_total} != {command_checks_completed}"
            )


def validate_tbench_task_ledger_checkpoints(
    issues: list[str],
    check: dict[str, Any],
) -> None:
    missing_checkpoints = list_value(check.get("missingTaskLedgerCheckpoints"))
    if missing_checkpoints:
        issues.append(
            "missing TBench diagnostic task ledger checkpoints: "
            + ", ".join(str(item) for item in missing_checkpoints)
        )
    entries = check.get("taskLedgerCheckpointFixtures")
    if not isinstance(entries, dict):
        issues.append("TBench diagnostics task ledger checkpoint fixture map is missing")
        return
    unexpected = sorted(
        str(fixture)
        for fixture in entries
        if fixture not in EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS
    )
    if unexpected:
        issues.append(
            "unexpected TBench diagnostic task ledger checkpoint fixtures: "
            + ", ".join(unexpected)
        )
    for fixture, expected in EXPECTED_TBENCH_DIAGNOSTIC_TASK_LEDGER_CHECKPOINTS.items():
        entry = entries.get(fixture)
        if not isinstance(entry, dict):
            issues.append(
                f"TBench diagnostics task ledger checkpoint fixture missing: {fixture}"
            )
            continue
        updates = int_value(entry.get("updates"))
        completed = int_value(entry.get("completed"))
        expected_updates = int_value(expected.get("updates"))
        expected_completed = int_value(expected.get("completed"))
        if updates < expected_updates or completed < expected_completed:
            issues.append(
                "TBench diagnostics task ledger checkpoint incomplete: "
                f"{fixture} updates {updates}/{expected_updates} "
                f"completed {completed}/{expected_completed}"
            )


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []


def int_value(value: Any) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return 0
