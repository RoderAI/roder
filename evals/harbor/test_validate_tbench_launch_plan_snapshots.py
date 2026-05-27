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


class LaunchPlanSnapshotTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_ready_plan_requires_dependency_snapshots(self) -> None:
        plan = ready_plan()
        for field in ("prebuiltBinary", "authFile", "harborHarness"):
            plan.pop(field, None)

        result = self.module.validate_plan(plan, require_ready=True)

        self.assertFalse(result.ok)
        self.assertIn("prebuiltBinary is missing", result.issues)
        self.assertIn("authFile is missing", result.issues)
        self.assertIn("harborHarness is missing", result.issues)


if __name__ == "__main__":
    unittest.main()
