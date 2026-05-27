#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
MODULE_PATH = ROOT / "evals/harbor/validate_tbench_launch_plan.py"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from launch_plan_test_helpers import ready_plan, write_clean_summary_fixture  # noqa: E402


def load_module():
    spec = importlib.util.spec_from_file_location(
        "validate_tbench_launch_plan",
        MODULE_PATH,
    )
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class LaunchPlanCampaignSummaryCopyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_rejects_copied_campaign_expectation_set_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            campaign_summary = clean_campaign_summary_check(
                temp_path / "combined-summary.json"
            )
            summary_data["options"]["campaignSummary"] = str(
                temp_path / "combined-summary.json"
            )
            summary_data["checks"]["campaignSummary"] = campaign_summary
            summary.write_text(json.dumps(summary_data) + "\n")

            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["campaignSummary"] = dict(campaign_summary)
            plan["campaignSummary"]["expectOwners"] = ["injected drift"]

            result = self.module.validate_plan(
                plan,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("campaign summary expectOwners mismatch", result.issues)

    def test_rejects_copied_campaign_duplicates_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            campaign_summary = clean_campaign_summary_check(
                temp_path / "combined-summary.json"
            )
            summary_data["options"]["campaignSummary"] = str(
                temp_path / "combined-summary.json"
            )
            summary_data["checks"]["campaignSummary"] = campaign_summary
            summary.write_text(json.dumps(summary_data) + "\n")

            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["campaignSummary"] = dict(campaign_summary)
            plan["campaignSummary"]["duplicates"] = [{"taskName": "qemu-startup"}]

            result = self.module.validate_plan(
                plan,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("campaign summary duplicates mismatch", result.issues)


def clean_campaign_summary_check(path: Path) -> dict:
    manifests = []
    for campaign in ("validated-conversions", "historical-wins"):
        manifest = path.parent / f"{campaign}-manifest.json"
        manifest.write_text(json.dumps({"campaign": campaign}) + "\n")
        manifests.append(
            {
                "campaign": campaign,
                "manifest": str(manifest),
                "manifestSha256": sha256(manifest.read_bytes()).hexdigest(),
            }
        )
    path.write_text('{"validation":{"status":"ok"}}\n')
    return {
        "status": "passed",
        "summaryJson": str(path),
        "summaryJsonSha256": sha256(path.read_bytes()).hexdigest(),
        "preset": "validated-plus-historical",
        "validationStatus": "ok",
        "issues": [],
        "uniqueTasks": 18,
        "projectedPasses": 68,
        "duplicateTasks": 0,
        "duplicates": [],
        "requireNoOverlap": True,
        "expectUniqueTasks": 18,
        "expectProjectedPasses": 68,
        "expectCampaigns": ["validated-conversions", "historical-wins"],
        "expectRoutes": [
            "validated-conversions/medium-validated",
            "validated-conversions/xhigh-validated",
            "validated-conversions/xhigh-plan-first",
            "historical-wins/policy-framed",
            "historical-wins/environment-targeted",
        ],
        "expectTasks": [
            "password-recovery",
            "qemu-startup",
            "vulnerable-secret",
        ],
        "expectOwners": [
            "password-recovery=historical-wins/policy-framed",
            "qemu-startup=historical-wins/environment-targeted",
            "vulnerable-secret=historical-wins/policy-framed",
        ],
        "manifests": manifests,
    }


if __name__ == "__main__":
    unittest.main()
