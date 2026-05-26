#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "evals/harbor/write_pre_eval_summary.py"


def load_module():
    spec = importlib.util.spec_from_file_location("write_pre_eval_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class PreEvalAnalysisSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_analysis_summary_records_blocked_metrics(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            validation_dir = root / "harbor-analysis-baseline"
            validation_dir.mkdir()
            (validation_dir / "validation.json").write_text(
                json.dumps(
                    {
                        "status": "blocked",
                        "rows": [
                            {
                                "metric": "unknown_error",
                                "current": 1,
                                "maxCount": 0,
                                "status": "blocked",
                            },
                            {
                                "metric": "scored_trials",
                                "current": 89,
                                "minCount": 89,
                                "status": "ok",
                            },
                        ],
                        "metrics": {"unknown_error": 1, "scored_trials": 89},
                    }
                )
            )

            summary = self.module.analysis_validation_summary(
                validation_dir,
                "evals/reports/harbor/full-analysis.json",
                "evals/harbor/tbench-clean-baseline.json",
            )

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(
                [
                    {
                        "metric": "unknown_error",
                        "current": 1,
                        "maxCount": 0,
                        "status": "blocked",
                    }
                ],
                summary["blockedMetrics"],
            )
            self.assertEqual(
                {"unknown_error": 1, "scored_trials": 89},
                summary["metrics"],
            )


if __name__ == "__main__":
    unittest.main()
