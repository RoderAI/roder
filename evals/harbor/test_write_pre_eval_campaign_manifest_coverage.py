#!/usr/bin/env python3

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path


HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from pre_eval_summary_test_helpers import build_summary, load_module  # noqa: E402


class PreEvalCampaignManifestCoverageTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_summary_blocks_validated_plus_historical_missing_expected_campaign(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(campaign_summary, campaigns=["validated-conversions"])

            summary = build_summary(
                self.module,
                root,
                tbench_outcomes=["pass"],
                campaign_summary=campaign_summary,
            )

        self.assertEqual("blocked", summary["status"])
        self.assertEqual(["campaignSummary"], summary["blockedChecks"])
        self.assertEqual("failed", summary["checks"]["campaignSummary"]["status"])
        self.assertIn(
            "campaign campaigns mismatch",
            summary["checks"]["campaignSummary"]["issues"],
        )


def write_campaign_summary(path: Path, *, campaigns: list[str]) -> None:
    manifests = []
    for campaign in campaigns:
        manifest = path.parent / f"{campaign}-manifest.json"
        manifest.write_text(json.dumps({"campaign": campaign}) + "\n")
        manifests.append(
            {
                "campaign": campaign,
                "manifest": str(manifest),
                "manifestSha256": sha256(manifest.read_bytes()).hexdigest(),
            }
        )
    path.write_text(
        json.dumps(
            {
                "validation": {
                    "status": "ok",
                    "preset": "validated-plus-historical",
                    "issues": [],
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
                },
                "summary": {
                    "uniqueTasks": 18,
                    "duplicateTasks": 0,
                },
                "scoreProjection": {
                    "projectedPassesIfAllRoutesPass": 68,
                },
                "duplicates": [],
                "campaigns": manifests,
            }
        )
    )


if __name__ == "__main__":
    unittest.main()
