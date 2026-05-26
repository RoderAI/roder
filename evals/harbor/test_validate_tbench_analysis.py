#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "evals/harbor/validate_tbench_analysis.py"


def load_module():
    spec = importlib.util.spec_from_file_location("validate_tbench_analysis", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class TBenchAnalysisBaselineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def baseline(self) -> dict:
        return {
            "version": 1,
            "expectations": [
                {"metric": "harbor_n_errors", "maxCount": 0},
                {"metric": "harness_error_total", "maxCount": 0},
                {"metric": "unknown_error", "maxCount": 0},
                {"metric": "task_dirs", "minCount": 89},
                {"metric": "scored_trials", "minCount": 89},
            ],
        }

    def test_clean_scored_failures_pass_the_baseline(self) -> None:
        analysis = {
            "clean": True,
            "stats": {
                "harbor": {"n_errors": 0},
                "task_dirs": 89,
                "passes": 43,
                "scored_failures": 46,
                "harness_error_classes": {},
            },
            "classes": {"pass": [{}] * 43, "scored_fail": [{}] * 46},
        }

        comparison = self.module.compare_analysis_to_baseline(
            analysis,
            self.baseline(),
        )

        self.assertEqual("ok", comparison["status"])
        self.assertEqual([], [row for row in comparison["rows"] if row["status"] != "ok"])

    def test_harness_errors_block_even_when_scored_failures_are_present(self) -> None:
        analysis = {
            "clean": False,
            "stats": {
                "harbor": {"n_errors": 0},
                "task_dirs": 89,
                "passes": 42,
                "scored_failures": 47,
                "harness_error_classes": {"unknown_error": 1},
            },
            "classes": {
                "pass": [{}] * 42,
                "scored_fail": [{}] * 47,
                "unknown_error": [{}],
            },
        }

        comparison = self.module.compare_analysis_to_baseline(
            analysis,
            self.baseline(),
        )

        blocked = [row for row in comparison["rows"] if row["status"] == "blocked"]
        self.assertEqual("blocked", comparison["status"])
        self.assertIn("harness_error_total", {row["metric"] for row in blocked})
        self.assertIn("unknown_error", {row["metric"] for row in blocked})

    def test_checked_in_baseline_blocks_provider_runtime_errors(self) -> None:
        analysis = {
            "clean": True,
            "stats": {
                "harbor": {"n_errors": 0},
                "task_dirs": 89,
                "passes": 85,
                "scored_failures": 4,
                "harness_error_classes": {},
            },
            "classes": {
                "pass": [{}] * 85,
                "scored_fail": [{}] * 4,
                "provider_api_invalid_tool_name": [{}],
                "provider_stream_decode_error": [{}],
                "provider_stream_incomplete": [{}],
                "roder_exec_error_status": [{}],
            },
        }

        comparison = self.module.compare_analysis_to_baseline(
            analysis,
            self.module.load_json(ROOT / "evals/harbor/tbench-clean-baseline.json"),
        )

        blocked_metrics = {
            row["metric"]
            for row in comparison["rows"]
            if row["status"] == "blocked"
        }
        self.assertEqual("blocked", comparison["status"])
        self.assertIn("provider_api_invalid_tool_name", blocked_metrics)
        self.assertIn("provider_stream_decode_error", blocked_metrics)
        self.assertIn("provider_stream_incomplete", blocked_metrics)
        self.assertIn("roder_exec_error_status", blocked_metrics)

    def test_harness_error_classes_block_when_baseline_omits_metric(self) -> None:
        analysis = {
            "clean": True,
            "stats": {
                "harbor": {"n_errors": 0},
                "task_dirs": 89,
                "passes": 88,
                "scored_failures": 1,
                "harness_error_classes": {},
            },
            "classes": {
                "pass": [{}] * 88,
                "scored_fail": [{}],
                "provider_stream_incomplete": [{}],
            },
        }

        comparison = self.module.compare_analysis_to_baseline(
            analysis,
            self.baseline(),
        )

        blocked = [row for row in comparison["rows"] if row["status"] == "blocked"]
        self.assertEqual("blocked", comparison["status"])
        self.assertIn(
            {
                "metric": "provider_stream_incomplete",
                "current": 1,
                "status": "blocked",
                "maxCount": 0,
            },
            blocked,
        )

    def test_unclean_analysis_blocks_when_baseline_omits_clean_metric(self) -> None:
        analysis = {
            "clean": False,
            "stats": {
                "harbor": {"n_errors": 0},
                "task_dirs": 89,
                "passes": 89,
                "scored_failures": 0,
                "harness_error_classes": {},
            },
            "classes": {"pass": [{}] * 89},
        }

        comparison = self.module.compare_analysis_to_baseline(
            analysis,
            self.baseline(),
        )

        blocked = [row for row in comparison["rows"] if row["status"] == "blocked"]
        self.assertEqual("blocked", comparison["status"])
        self.assertIn(
            {
                "metric": "clean",
                "current": 0,
                "status": "blocked",
                "minCount": 1,
            },
            blocked,
        )

    def test_unknown_baseline_metric_blocks(self) -> None:
        analysis = {
            "clean": True,
            "stats": {
                "harbor": {"n_errors": 0},
                "task_dirs": 89,
                "passes": 89,
                "scored_failures": 0,
                "harness_error_classes": {},
            },
            "classes": {"pass": [{}] * 89},
        }
        baseline = {
            "version": 1,
            "expectations": [
                {"metric": "provider_stream_incomplete", "maxCount": 0},
                {"metric": "provider_stream_incomplet", "maxCount": 0},
            ],
        }

        comparison = self.module.compare_analysis_to_baseline(analysis, baseline)

        blocked = [row for row in comparison["rows"] if row["status"] == "blocked"]
        self.assertEqual("blocked", comparison["status"])
        self.assertIn(
            {
                "metric": "provider_stream_incomplet",
                "current": 0,
                "status": "blocked",
                "reason": "unknown_metric",
            },
            blocked,
        )

    def test_stats_class_count_mismatch_blocks_even_when_baseline_omits_metric(self) -> None:
        analysis = {
            "clean": True,
            "stats": {
                "harbor": {"n_errors": 0},
                "task_dirs": 89,
                "passes": 89,
                "scored_failures": 0,
                "harness_error_classes": {},
            },
            "classes": {"pass": [{}] * 88, "scored_fail": [{}]},
        }

        comparison = self.module.compare_analysis_to_baseline(
            analysis,
            self.baseline(),
        )

        blocked = [row for row in comparison["rows"] if row["status"] == "blocked"]
        self.assertEqual("blocked", comparison["status"])
        self.assertIn(
            {
                "metric": "analysis_consistency",
                "current": 0,
                "status": "blocked",
                "minCount": 1,
            },
            blocked,
        )

    def test_expected_trials_allows_clean_subset_analysis(self) -> None:
        analysis = {
            "clean": True,
            "stats": {
                "harbor": {"n_errors": 0, "n_trials": 7},
                "task_dirs": 7,
                "passes": 3,
                "scored_failures": 4,
                "harness_error_classes": {},
            },
            "classes": {"pass": [{}] * 3, "scored_fail": [{}] * 4},
        }

        comparison = self.module.compare_analysis_to_baseline(
            analysis,
            self.baseline(),
            expected_trials=7,
        )

        self.assertEqual("ok", comparison["status"])
        trial_rows = {
            row["metric"]: row
            for row in comparison["rows"]
            if row["metric"] in {"task_dirs", "scored_trials"}
        }
        self.assertEqual(7, trial_rows["task_dirs"]["minCount"])
        self.assertEqual(7, trial_rows["scored_trials"]["minCount"])


if __name__ == "__main__":
    unittest.main()
