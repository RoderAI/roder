#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from tbench_diagnostic_test_data import (  # noqa: E402
    command_check_fixture_summary,
    default_metrics_for_fixture,
    diagnostic_fixture_ids,
    task_ledger_checkpoint_summary,
)

MODULE_PATH = ROOT / "evals/harbor/write_pre_eval_summary.py"


def load_module():
    spec = importlib.util.spec_from_file_location("write_pre_eval_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def result(fixture_id: str) -> dict:
    return {
        "fixtureId": fixture_id,
        "report": {
            "outcome": "pass",
            "metrics": default_metrics_for_fixture(fixture_id),
        },
    }


def result_without_command_checks(fixture_id: str) -> dict:
    entry = result(fixture_id)
    metrics = entry["report"]["metrics"]
    entry["report"]["metrics"] = [
        metric
        for metric in metrics
        if not str(metric["name"]).startswith("verifier_command_checks_")
    ]
    return entry


def result_without_required_command_check(fixture_id: str) -> dict:
    entry = result(fixture_id)
    entry["report"]["metrics"] = [
        metric
        for metric in entry["report"]["metrics"]
        if metric["name"] != "verifier_command_checks_required"
    ]
    return entry


def write_eval_run(
    directory: Path,
    fixture_ids: list[str],
    *,
    include_command_checks: bool = True,
) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    result_fn = result if include_command_checks else result_without_command_checks
    (directory / "eval-run.json").write_text(
        json.dumps({"results": [result_fn(fixture_id) for fixture_id in fixture_ids]})
    )


class PreEvalTBenchSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def build_summary(
        self,
        root: Path,
        fixture_ids: list[str],
        *,
        include_command_checks: bool = True,
    ) -> dict:
        tbench_dir = root / "tbench-diagnostics"
        write_eval_run(
            tbench_dir,
            fixture_ids,
            include_command_checks=include_command_checks,
        )
        return self.module.build_summary(
            output_root=root,
            tbench_dir=tbench_dir,
            speed_dir=None,
            analysis_dir=None,
            run_tests=False,
            include_speed=False,
            require_prebuilt=False,
            preflight_images=False,
            pull_images=False,
            image_config="",
            analysis_target="",
            analysis_baseline="",
            prebuilt_binary=root / "missing-roder",
            auth_file=root / "codex.json",
            require_auth=False,
            image_manifest=None,
        )

    def test_summary_blocks_missing_expected_tbench_diagnostic_fixture(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                [
                    "tbench-exact-output-file",
                    "tbench-json-array-output",
                    "tbench-sequence-output",
                ],
            )

            diagnostic = summary["checks"]["tbenchDiagnostics"]
            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["tbenchDiagnostics"], summary["blockedChecks"])
            self.assertEqual("failed", diagnostic["status"])
            self.assertEqual(
                [
                    "tbench-numeric-tolerance-output",
                    "tbench-output-directory-hygiene",
                    "tbench-visible-verifier-contract",
                    "tbench-artifact-checkpoint",
                    "tbench-service-target-sanity",
                    "tbench-verifier-dependency-parity",
                ],
                diagnostic["missingExpectedFixtures"],
            )

    def test_summary_blocks_unexpected_tbench_diagnostic_fixture(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                [
                    "tbench-exact-output-file",
                    "tbench-json-array-output",
                    "tbench-numeric-tolerance-output",
                    "tbench-output-directory-hygiene",
                    "tbench-sequence-output",
                    "tbench-visible-verifier-contract",
                    "tbench-artifact-checkpoint",
                    "tbench-service-target-sanity",
                    "tbench-verifier-dependency-parity",
                    "tbench-extra-contract",
                ],
            )

            diagnostic = summary["checks"]["tbenchDiagnostics"]
            self.assertEqual("blocked", summary["status"])
            self.assertEqual("failed", diagnostic["status"])
            self.assertEqual(["tbench-extra-contract"], diagnostic["unexpectedFixtures"])

    def test_summary_blocks_duplicate_tbench_diagnostic_fixture(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                [
                    "tbench-exact-output-file",
                    "tbench-json-array-output",
                    "tbench-numeric-tolerance-output",
                    "tbench-output-directory-hygiene",
                    "tbench-sequence-output",
                    "tbench-visible-verifier-contract",
                    "tbench-artifact-checkpoint",
                    "tbench-service-target-sanity",
                    "tbench-verifier-dependency-parity",
                    "tbench-exact-output-file",
                ],
            )

            diagnostic = summary["checks"]["tbenchDiagnostics"]
            self.assertEqual("blocked", summary["status"])
            self.assertEqual("failed", diagnostic["status"])
            self.assertEqual(["tbench-exact-output-file"], diagnostic["duplicateFixtures"])

    def test_summary_blocks_missing_expected_tbench_command_check(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                [
                    "tbench-exact-output-file",
                    "tbench-json-array-output",
                    "tbench-numeric-tolerance-output",
                    "tbench-output-directory-hygiene",
                    "tbench-sequence-output",
                    "tbench-visible-verifier-contract",
                    "tbench-artifact-checkpoint",
                    "tbench-service-target-sanity",
                    "tbench-verifier-dependency-parity",
                ],
                include_command_checks=False,
            )

            diagnostic = summary["checks"]["tbenchDiagnostics"]
            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["tbenchDiagnostics"], summary["blockedChecks"])
            self.assertEqual("failed", diagnostic["status"])
            self.assertEqual(
                [
                    "tbench-numeric-tolerance-output",
                    "tbench-output-directory-hygiene",
                    "tbench-visible-verifier-contract",
                    "tbench-artifact-checkpoint",
                    "tbench-service-target-sanity",
                    "tbench-verifier-dependency-parity",
                ],
                diagnostic["missingCommandChecks"],
            )

    def test_summary_blocks_command_check_completed_without_required_metric(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            tbench_dir = root / "tbench-diagnostics"
            tbench_dir.mkdir(parents=True)
            fixture_ids = [
                "tbench-exact-output-file",
                "tbench-json-array-output",
                "tbench-numeric-tolerance-output",
                "tbench-output-directory-hygiene",
                "tbench-sequence-output",
                "tbench-visible-verifier-contract",
                "tbench-artifact-checkpoint",
                "tbench-service-target-sanity",
                "tbench-verifier-dependency-parity",
            ]
            entries = [
                result_without_required_command_check(fixture_id)
                if fixture_id == "tbench-numeric-tolerance-output"
                else result(fixture_id)
                for fixture_id in fixture_ids
            ]
            (tbench_dir / "eval-run.json").write_text(json.dumps({"results": entries}))

            summary = self.module.build_summary(
                output_root=root,
                tbench_dir=tbench_dir,
                speed_dir=None,
                analysis_dir=None,
                run_tests=False,
                include_speed=False,
                require_prebuilt=False,
                preflight_images=False,
                pull_images=False,
                image_config="",
                analysis_target="",
                analysis_baseline="",
                prebuilt_binary=root / "missing-roder",
                auth_file=root / "codex.json",
                require_auth=False,
                image_manifest=None,
            )

            diagnostic = summary["checks"]["tbenchDiagnostics"]
            self.assertEqual("blocked", summary["status"])
            self.assertEqual("failed", diagnostic["status"])
            self.assertEqual(
                ["tbench-numeric-tolerance-output"],
                diagnostic["missingCommandChecks"],
            )
            self.assertEqual(
                0,
                diagnostic["verifierCommandCheckFixtures"][
                    "tbench-numeric-tolerance-output"
                ]["required"],
            )

    def test_summary_reports_expected_tbench_command_check_totals(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                [
                    "tbench-exact-output-file",
                    "tbench-json-array-output",
                    "tbench-numeric-tolerance-output",
                    "tbench-output-directory-hygiene",
                    "tbench-sequence-output",
                    "tbench-visible-verifier-contract",
                    "tbench-artifact-checkpoint",
                    "tbench-service-target-sanity",
                    "tbench-verifier-dependency-parity",
                ],
            )

            diagnostic = summary["checks"]["tbenchDiagnostics"]
            self.assertEqual("ok", summary["status"])
            self.assertEqual("passed", diagnostic["status"])
            self.assertEqual(6, diagnostic["verifierCommandChecksRequired"])
            self.assertEqual(6, diagnostic["verifierCommandChecksCompleted"])
            self.assertEqual(
                {
                    "tbench-artifact-checkpoint": {
                        "required": 1,
                        "completed": 1,
                    },
                    "tbench-numeric-tolerance-output": {
                        "required": 1,
                        "completed": 1,
                    },
                    "tbench-output-directory-hygiene": {
                        "required": 1,
                        "completed": 1,
                    },
                    "tbench-visible-verifier-contract": {
                        "required": 1,
                        "completed": 1,
                    },
                    "tbench-service-target-sanity": {
                        "required": 1,
                        "completed": 1,
                    },
                    "tbench-verifier-dependency-parity": {
                        "required": 1,
                        "completed": 1,
                    },
                },
                diagnostic["verifierCommandCheckFixtures"],
            )
            self.assertEqual(
                {
                    "tbench-artifact-checkpoint": {
                        "updates": 2,
                        "completed": 2,
                    },
                },
                diagnostic["taskLedgerCheckpointFixtures"],
            )

    def test_summary_blocks_missing_artifact_checkpoint_metrics(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            tbench_dir = root / "tbench-diagnostics"
            tbench_dir.mkdir(parents=True)
            fixture_ids = [
                "tbench-exact-output-file",
                "tbench-json-array-output",
                "tbench-numeric-tolerance-output",
                "tbench-output-directory-hygiene",
                "tbench-sequence-output",
                "tbench-visible-verifier-contract",
                "tbench-artifact-checkpoint",
                "tbench-service-target-sanity",
                "tbench-verifier-dependency-parity",
            ]
            entries = [result(fixture_id) for fixture_id in fixture_ids]
            artifact_entry = next(
                entry
                for entry in entries
                if entry["fixtureId"] == "tbench-artifact-checkpoint"
            )
            artifact_entry["report"]["metrics"] = [
                metric
                for metric in artifact_entry["report"]["metrics"]
                if not str(metric["name"]).startswith("task_ledger_")
            ]
            (tbench_dir / "eval-run.json").write_text(json.dumps({"results": entries}))

            summary = self.module.build_summary(
                output_root=root,
                tbench_dir=tbench_dir,
                speed_dir=None,
                analysis_dir=None,
                run_tests=False,
                include_speed=False,
                require_prebuilt=False,
                preflight_images=False,
                pull_images=False,
                image_config="",
                analysis_target="",
                analysis_baseline="",
                prebuilt_binary=root / "missing-roder",
                auth_file=root / "codex.json",
                require_auth=False,
                image_manifest=None,
            )

            diagnostic = summary["checks"]["tbenchDiagnostics"]
            self.assertEqual("blocked", summary["status"])
            self.assertEqual("failed", diagnostic["status"])
            self.assertEqual(
                ["tbench-artifact-checkpoint"],
                diagnostic["missingTaskLedgerCheckpoints"],
            )


if __name__ == "__main__":
    unittest.main()
