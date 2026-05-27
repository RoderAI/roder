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


class ValidatePreEvalSummaryImageCountsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_required_clean_image_preflight_requires_present_tasks(self) -> None:
        summary = clean_summary()
        summary["checks"]["imagePreflight"]["tasks"] = 2
        summary["checks"]["imagePreflight"]["uniqueImages"] = 1
        summary["checks"]["imagePreflight"]["present"] = 1
        summary["checks"]["imagePreflight"]["missing"] = 0
        summary["checks"]["imagePreflight"]["unresolved"] = 0
        summary["checks"]["imagePreflight"]["pullFailed"] = 0

        result = self.module.validate_summary(
            summary,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight present does not cover all tasks", result.issues)


if __name__ == "__main__":
    unittest.main()
