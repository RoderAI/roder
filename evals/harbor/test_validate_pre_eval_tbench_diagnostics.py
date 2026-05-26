#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stderr
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from tbench_diagnostic_test_data import (  # noqa: E402
    default_metrics_for_fixture,
    diagnostic_fixture_ids,
    passing_diagnostic_results,
)

MODULE_PATH = ROOT / "evals/harbor/validate_pre_eval_tbench_diagnostics.py"


def load_module():
    spec = importlib.util.spec_from_file_location(
        "validate_pre_eval_tbench_diagnostics",
        MODULE_PATH,
    )
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def result(fixture_id: str, outcome: str, metrics: dict[str, float]) -> dict:
    return {
        "fixtureId": fixture_id,
        "report": {
            "outcome": outcome,
            "metrics": [
                {"name": name, "value": value} for name, value in metrics.items()
            ],
        },
    }


def passing_result(fixture_id: str, *, include_command_metrics: bool = True) -> dict:
    metrics = default_metrics_for_fixture(fixture_id)
    if not include_command_metrics:
        metrics = [
            metric
            for metric in metrics
            if not str(metric["name"]).startswith("verifier_command_checks_")
        ]
    return {
        "fixtureId": fixture_id,
        "report": {
            "outcome": "pass",
            "metrics": metrics,
        },
    }


class TBenchDiagnosticsValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_clean_diagnostics_pass(self) -> None:
        run = {"results": passing_diagnostic_results()}

        summary = self.module.validate_run(run)

        self.assertTrue(summary.ok)
        self.assertEqual(9, summary.fixtures)
        self.assertEqual([], summary.failed_fixtures)
        self.assertEqual(6, summary.command_checks_required)
        self.assertEqual(6, summary.command_checks_completed)
        self.assertEqual([], summary.missing_task_ledger_checkpoints)

    def test_unexpected_diagnostic_fixture_blocks(self) -> None:
        run = {
            "results": passing_diagnostic_results()
            + [passing_result("tbench-extra-contract")]
        }

        summary = self.module.validate_run(run)

        self.assertFalse(summary.ok)
        self.assertEqual(["tbench-extra-contract"], summary.unexpected_fixtures)

    def test_duplicate_diagnostic_fixture_blocks(self) -> None:
        run = {
            "results": passing_diagnostic_results()
            + [passing_result("tbench-exact-output-file")]
        }

        summary = self.module.validate_run(run)

        self.assertFalse(summary.ok)
        self.assertEqual(["tbench-exact-output-file"], summary.duplicate_fixtures)

    def test_missing_expected_diagnostic_fixture_blocks(self) -> None:
        run = {
            "results": [
                passing_result(fixture_id)
                for fixture_id in diagnostic_fixture_ids()
                if fixture_id != "tbench-numeric-tolerance-output"
            ]
        }

        summary = self.module.validate_run(run)

        self.assertFalse(summary.ok)
        self.assertEqual(
            ["tbench-numeric-tolerance-output"],
            summary.missing_fixtures,
        )

    def test_failed_fixture_blocks(self) -> None:
        run = {
            "results": [
                result(
                    "json-array-output",
                    "fail",
                    {
                        "verification_completed": 1,
                        "reliability_unknown_errors": 0,
                    },
                )
            ]
        }

        summary = self.module.validate_run(run)

        self.assertFalse(summary.ok)
        self.assertEqual(["json-array-output"], summary.failed_fixtures)

    def test_missing_verification_blocks(self) -> None:
        run = {
            "results": [
                result(
                    "sequence-output",
                    "pass",
                    {
                        "verification_completed": 0,
                        "reliability_unknown_errors": 0,
                    },
                )
            ]
        }

        summary = self.module.validate_run(run)

        self.assertFalse(summary.ok)
        self.assertEqual(["sequence-output"], summary.missing_verification)

    def test_missing_expected_command_check_blocks(self) -> None:
        run = {
            "results": [
                passing_result("tbench-exact-output-file"),
                passing_result("tbench-json-array-output"),
                passing_result(
                    "tbench-numeric-tolerance-output",
                    include_command_metrics=False,
                ),
                passing_result(
                    "tbench-output-directory-hygiene",
                    include_command_metrics=False,
                ),
                passing_result(
                    "tbench-visible-verifier-contract",
                    include_command_metrics=False,
                ),
                passing_result(
                    "tbench-artifact-checkpoint",
                    include_command_metrics=False,
                ),
                passing_result(
                    "tbench-service-target-sanity",
                    include_command_metrics=False,
                ),
                passing_result(
                    "tbench-verifier-dependency-parity",
                    include_command_metrics=False,
                ),
                passing_result("tbench-sequence-output"),
            ]
        }

        summary = self.module.validate_run(run)

        self.assertFalse(summary.ok)
        self.assertEqual(
                [
                    "tbench-numeric-tolerance-output",
                    "tbench-output-directory-hygiene",
                    "tbench-visible-verifier-contract",
                    "tbench-artifact-checkpoint",
                    "tbench-service-target-sanity",
                    "tbench-verifier-dependency-parity",
                ],
                summary.missing_command_checks,
            )
        self.assertEqual(6, summary.command_checks_required)
        self.assertEqual(0, summary.command_checks_completed)

    def test_command_check_completed_without_required_metric_blocks(self) -> None:
        run = {
            "results": [
                passing_result("tbench-exact-output-file"),
                passing_result("tbench-json-array-output"),
                result(
                    "tbench-numeric-tolerance-output",
                    "pass",
                    {
                        "verification_completed": 1,
                        "reliability_unknown_errors": 0,
                        "verifier_command_checks_completed": 1,
                    },
                ),
                passing_result("tbench-output-directory-hygiene"),
                passing_result("tbench-visible-verifier-contract"),
                passing_result("tbench-artifact-checkpoint"),
                passing_result("tbench-service-target-sanity"),
                passing_result("tbench-verifier-dependency-parity"),
                passing_result("tbench-sequence-output"),
            ]
        }

        summary = self.module.validate_run(run)

        self.assertFalse(summary.ok)
        self.assertEqual(
            ["tbench-numeric-tolerance-output"],
            summary.missing_command_checks,
        )

    def test_missing_artifact_checkpoint_ledger_metrics_block(self) -> None:
        run = {
            "results": [
                passing_result("tbench-exact-output-file"),
                passing_result("tbench-json-array-output"),
                passing_result("tbench-numeric-tolerance-output"),
                passing_result("tbench-output-directory-hygiene"),
                passing_result("tbench-sequence-output"),
                passing_result("tbench-visible-verifier-contract"),
                passing_result("tbench-service-target-sanity"),
                passing_result("tbench-verifier-dependency-parity"),
                result(
                    "tbench-artifact-checkpoint",
                    "pass",
                    {
                        "verification_completed": 1,
                        "reliability_unknown_errors": 0,
                        "verifier_command_checks_required": 1,
                        "verifier_command_checks_completed": 1,
                        "task_ledger_updates": 1,
                        "task_ledger_completed": 1,
                    },
                ),
            ]
        }

        summary = self.module.validate_run(run)

        self.assertFalse(summary.ok)
        self.assertEqual(
            ["tbench-artifact-checkpoint"],
            summary.missing_task_ledger_checkpoints,
        )

    def test_unknown_reliability_errors_block(self) -> None:
        run = {
            "results": [
                result(
                    "numeric-tolerance-output",
                    "pass",
                    {
                        "verification_completed": 1,
                        "reliability_unknown_errors": 2,
                    },
                )
            ]
        }

        summary = self.module.validate_run(run)

        self.assertFalse(summary.ok)
        self.assertEqual(2, summary.unknown_errors)

    def test_main_returns_nonzero_for_blocked_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "eval-run.json"
            path.write_text(
                json.dumps(
                    {
                        "results": [
                            result(
                                "exact-output-file",
                                "pass",
                                {"verification_completed": 0},
                            )
                        ]
                    }
                )
            )

            stderr = io.StringIO()
            with redirect_stderr(stderr):
                exit_code = self.module.main([str(path)])

        self.assertEqual(1, exit_code)
        self.assertIn("missing verification", stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
