#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from validate_pre_eval_summary_test_helpers import clean_summary  # noqa: E402

MODULE_PATH = ROOT / "evals/harbor/validate_pre_eval_summary.py"


def load_module():
    spec = importlib.util.spec_from_file_location("validate_pre_eval_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class ValidatePreEvalSummaryTBenchTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_tbench_diagnostics_missing_fixture_id_list_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"].pop("fixtureIds")

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("TBench diagnostic fixtureIds are missing", result.issues)

    def test_tbench_diagnostics_missing_fixed_fixture_id_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["fixtureIds"] = [
            fixture
            for fixture in summary["checks"]["tbenchDiagnostics"]["fixtureIds"]
            if fixture != "tbench-output-directory-hygiene"
        ]

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "missing TBench diagnostic fixture IDs: tbench-output-directory-hygiene",
            result.issues,
        )

    def test_tbench_diagnostics_unexpected_fixture_id_blocks(self) -> None:
        summary = clean_summary()
        diagnostics = summary["checks"]["tbenchDiagnostics"]
        diagnostics["fixtureIds"].append("tbench-extra-contract")
        diagnostics["fixtures"] = 7
        diagnostics["passed"] = 7

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "unexpected TBench diagnostic fixture IDs: tbench-extra-contract",
            result.issues,
        )

    def test_tbench_diagnostics_duplicate_fixture_id_blocks(self) -> None:
        summary = clean_summary()
        diagnostics = summary["checks"]["tbenchDiagnostics"]
        diagnostics["fixtureIds"].append("tbench-exact-output-file")
        diagnostics["fixtures"] = 7
        diagnostics["passed"] = 7

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "duplicate TBench diagnostic fixture IDs: tbench-exact-output-file",
            result.issues,
        )

    def test_tbench_diagnostics_unexpected_fixture_field_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["unexpectedFixtures"] = [
            "tbench-extra-contract"
        ]

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "unexpected TBench diagnostic fixtures: tbench-extra-contract",
            result.issues,
        )

    def test_tbench_diagnostics_missing_unexpected_fixture_field_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"].pop("unexpectedFixtures", None)

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics unexpectedFixtures field is missing",
            result.issues,
        )

    def test_tbench_diagnostics_duplicate_fixture_field_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["duplicateFixtures"] = [
            "tbench-exact-output-file"
        ]

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "duplicate TBench diagnostic fixtures: tbench-exact-output-file",
            result.issues,
        )

    def test_tbench_diagnostics_missing_duplicate_fixture_field_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"].pop("duplicateFixtures", None)

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics duplicateFixtures field is missing",
            result.issues,
        )

    def test_tbench_diagnostics_missing_count_fields_block(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"].pop("fixtures")

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("TBench diagnostics count fields are missing", result.issues)

    def test_tbench_diagnostics_fixture_count_mismatch_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["fixtures"] = 4

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("TBench diagnostics fixture count mismatch: 4 != 9", result.issues)

    def test_tbench_diagnostics_failed_count_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["passed"] = 4
        summary["checks"]["tbenchDiagnostics"]["failed"] = 1

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("TBench diagnostics passed count mismatch: 4/9", result.issues)
        self.assertIn("TBench diagnostics failed count is not zero: 1", result.issues)

    def test_tbench_diagnostics_missing_command_checks_block(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["missingCommandChecks"] = [
            "tbench-output-directory-hygiene"
        ]

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "missing TBench diagnostic command checks: tbench-output-directory-hygiene",
            result.issues,
        )

    def test_tbench_diagnostics_incomplete_command_check_totals_block(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["verifierCommandChecksCompleted"] = 1

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics verifier command checks incomplete: 1/6",
            result.issues,
        )

    def test_tbench_diagnostics_command_check_required_total_spoof_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["verifierCommandChecksRequired"] = 1
        summary["checks"]["tbenchDiagnostics"]["verifierCommandChecksCompleted"] = 1

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics verifier command checks required mismatch: 1 != 6",
            result.issues,
        )

    def test_tbench_diagnostics_missing_command_check_totals_block(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"].pop("verifierCommandChecksCompleted")

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics verifier command check totals are missing",
            result.issues,
        )

    def test_tbench_diagnostics_missing_command_check_fixture_map_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"].pop("verifierCommandCheckFixtures")

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics verifier command check fixture map is missing",
            result.issues,
        )

    def test_tbench_diagnostics_missing_command_check_fixture_entry_blocks(self) -> None:
        summary = clean_summary()
        del summary["checks"]["tbenchDiagnostics"]["verifierCommandCheckFixtures"][
            "tbench-output-directory-hygiene"
        ]

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics verifier command check fixture missing: "
            "tbench-output-directory-hygiene",
            result.issues,
        )

    def test_tbench_diagnostics_incomplete_command_check_fixture_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["verifierCommandCheckFixtures"][
            "tbench-output-directory-hygiene"
        ]["completed"] = 0

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics verifier command check fixture incomplete: "
            "tbench-output-directory-hygiene 0/1",
            result.issues,
        )

    def test_tbench_diagnostics_extra_command_check_fixture_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["verifierCommandCheckFixtures"][
            "stale-extra-fixture"
        ] = {
            "required": 1,
            "completed": 1,
        }

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics verifier command check fixture required total mismatch: 7 != 6",
            result.issues,
        )
        self.assertIn(
            "TBench diagnostics verifier command check fixture completed total mismatch: 7 != 6",
            result.issues,
        )

    def test_tbench_diagnostics_missing_task_ledger_checkpoint_fixture_map_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"].pop("taskLedgerCheckpointFixtures")

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics task ledger checkpoint fixture map is missing",
            result.issues,
        )

    def test_tbench_diagnostics_incomplete_task_ledger_checkpoint_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["taskLedgerCheckpointFixtures"][
            "tbench-artifact-checkpoint"
        ]["updates"] = 1

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "TBench diagnostics task ledger checkpoint incomplete: "
            "tbench-artifact-checkpoint updates 1/2 completed 2/2",
            result.issues,
        )

    def test_tbench_diagnostics_missing_task_ledger_checkpoint_field_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["tbenchDiagnostics"]["missingTaskLedgerCheckpoints"] = [
            "tbench-artifact-checkpoint"
        ]

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "missing TBench diagnostic task ledger checkpoints: "
            "tbench-artifact-checkpoint",
            result.issues,
        )


if __name__ == "__main__":
    unittest.main()
