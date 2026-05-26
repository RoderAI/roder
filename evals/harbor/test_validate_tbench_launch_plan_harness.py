#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sys
import unittest
from hashlib import sha256
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


class ValidateTbenchLaunchPlanHarnessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_verify_harness_files_blocks_files_count_mismatch(self) -> None:
        entries = [
            {"path": str(path), "sha256": sha256(path.read_bytes()).hexdigest()}
            for path in self.module.DEFAULT_HARNESS_FILES
        ]
        plan = ready_plan()
        plan["harborHarness"] = {
            "status": "passed",
            "files": len(entries) + 1,
            "entries": entries,
            "combinedSha256": self.module.combined_file_digest(entries),
        }

        result = self.module.validate_plan(
            plan,
            require_ready=True,
            verify_harness_files=True,
        )

        self.assertFalse(result.ok)
        self.assertIn(
            f"harborHarness files count mismatch: expected {len(entries)}, got {len(entries) + 1}",
            result.issues,
        )


if __name__ == "__main__":
    unittest.main()
