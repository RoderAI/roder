#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path


HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from run_roder_tbench_full_test_helpers import SCRIPT, ROOT, clean_summary


class RunRoderTbenchFullCampaignSummaryTests(unittest.TestCase):
    def test_dry_run_launch_plan_records_required_campaign_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            campaign_summary = temp_path / "combined-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            data = clean_summary(prebuilt_binary=prebuilt)
            data["options"]["campaignSummary"] = str(campaign_summary)
            expected_campaign_check = clean_campaign_summary_check(campaign_summary)
            data["checks"]["campaignSummary"] = expected_campaign_check
            summary.write_text(json.dumps(data, indent=2) + "\n")
            launch_plan = temp_path / "launch-plan.json"

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.update(
                {
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY": str(campaign_summary),
                    "RODER_HARBOR_LAUNCH_PLAN": str(launch_plan),
                    "RODER_HARBOR_SKIP_PREFLIGHT": "1",
                }
            )

            result = subprocess.run(
                [str(SCRIPT)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            plan = json.loads(launch_plan.read_text()) if launch_plan.exists() else {}

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertTrue(plan["requireCampaignSummary"])
        self.assertEqual(expected_campaign_check, plan["campaignSummary"])

    def test_reused_pre_eval_summary_must_match_requested_campaign_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            recorded_campaign_summary = temp_path / "combined-summary.json"
            requested_campaign_summary = temp_path / "new-combined-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            data = clean_summary(prebuilt_binary=prebuilt)
            data["options"]["campaignSummary"] = str(recorded_campaign_summary)
            data["checks"]["campaignSummary"] = clean_campaign_summary_check(
                recorded_campaign_summary
            )
            summary.write_text(json.dumps(data, indent=2) + "\n")
            launch_plan = temp_path / "launch-plan.json"

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.update(
                {
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY": str(
                        requested_campaign_summary
                    ),
                    "RODER_HARBOR_LAUNCH_PLAN": str(launch_plan),
                    "RODER_HARBOR_SKIP_PREFLIGHT": "1",
                }
            )

            result = subprocess.run(
                [str(SCRIPT)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn("campaignSummary summaryJson path mismatch", result.stderr)


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
