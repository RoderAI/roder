#!/usr/bin/env python3

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from launch_plan_summary_copies import validate_plan_copies_match_summary  # noqa: E402
from launch_plan_test_helpers import ready_plan, write_clean_summary_fixture  # noqa: E402


def plan_bound_to_summary(summary: Path, temp_path: Path) -> dict:
    summary_data = json.loads(summary.read_text())
    plan = ready_plan()
    plan["preEvalSummary"] = str(summary)
    plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
    plan["harborConfig"] = str(temp_path / "tbench.json")
    config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
    plan["harborConfigSha256"] = config_sha
    plan["preEvalHarborConfigSha256"] = config_sha
    plan["prebuiltBinary"] = summary_data["prebuiltBinary"]
    plan["authFile"] = summary_data["authFile"]
    plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
    plan["harborHarness"] = dict(summary_data["checks"]["harborHarness"])
    return plan


class LaunchPlanHarnessCopyTests(unittest.TestCase):
    def test_rejects_copied_harness_status_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            plan = plan_bound_to_summary(summary, temp_path)
            plan["harborHarness"]["status"] = "failed"
            issues: list[str] = []

            validate_plan_copies_match_summary(
                issues,
                plan,
                summary_data,
            )

        self.assertIn("harbor harness summary status mismatch", issues)

    def test_rejects_copied_harness_entries_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            plan = plan_bound_to_summary(summary, temp_path)
            plan["harborHarness"]["entries"] = []
            issues: list[str] = []

            validate_plan_copies_match_summary(
                issues,
                plan,
                summary_data,
            )

        self.assertIn("harbor harness summary entries mismatch", issues)


if __name__ == "__main__":
    unittest.main()
