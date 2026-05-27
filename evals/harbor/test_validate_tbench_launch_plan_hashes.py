#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
MODULE_PATH = ROOT / "evals/harbor/validate_tbench_launch_plan.py"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from launch_plan_test_helpers import ready_plan  # noqa: E402


def load_module():
    spec = importlib.util.spec_from_file_location(
        "validate_tbench_launch_plan",
        MODULE_PATH,
    )
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class LaunchPlanHashTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_ready_plan_requires_sha256_anchors(self) -> None:
        plan = ready_plan()
        for field in (
            "harborConfigSha256",
            "preEvalHarborConfigSha256",
            "preEvalSummarySha256",
        ):
            plan.pop(field, None)

        result = self.module.validate_plan(plan, require_ready=True)

        self.assertFalse(result.ok)
        self.assertIn("harborConfigSha256 is missing", result.issues)
        self.assertIn("preEvalHarborConfigSha256 is missing", result.issues)
        self.assertIn("preEvalSummarySha256 is missing", result.issues)

    def test_ready_plan_rejects_config_hash_mismatch(self) -> None:
        plan = ready_plan()
        plan["harborConfigSha256"] = "a" * 64
        plan["preEvalHarborConfigSha256"] = "b" * 64

        result = self.module.validate_plan(plan, require_ready=True)

        self.assertFalse(result.ok)
        self.assertIn("pre-eval Harbor config SHA-256 mismatch", result.issues)


if __name__ == "__main__":
    unittest.main()
