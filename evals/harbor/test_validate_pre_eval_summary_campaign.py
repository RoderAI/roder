#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from hashlib import sha256
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


class ValidatePreEvalCampaignSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_validate_summary_accepts_clean_campaign_summary_when_required(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        summary["checks"]["campaignSummary"] = clean_campaign_summary_check()

        result = self.module.validate_summary(summary, require_campaign_summary=True)

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_validate_summary_blocks_missing_required_campaign_summary(self) -> None:
        summary = clean_summary()

        result = self.module.validate_summary(summary, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("required campaign summary did not run", result.issues)
        self.assertIn("campaignSummary check missing", result.issues)

    def test_validate_summary_blocks_present_campaign_summary_without_hashes(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        check = clean_campaign_summary_check()
        check["manifests"][0].pop("manifestSha256")
        summary["checks"]["campaignSummary"] = check

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary manifestSha256 is missing", result.issues)

    def test_validate_summary_blocks_present_campaign_summary_with_malformed_hash(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        check = clean_campaign_summary_check()
        check["manifests"][0]["manifestSha256"] = "not-a-sha"
        summary["checks"]["campaignSummary"] = check

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary manifestSha256 is invalid", result.issues)

    def test_validate_summary_blocks_unexpected_campaign_summary_path(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        summary["checks"]["campaignSummary"] = clean_campaign_summary_check()

        result = self.module.validate_summary(
            summary,
            expected_campaign_summary=Path("other-summary.json"),
        )

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary option path mismatch", result.issues)
        self.assertIn("campaignSummary summaryJson path mismatch", result.issues)

    def test_validate_summary_blocks_internal_campaign_summary_path_mismatch(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        check = clean_campaign_summary_check()
        check["summaryJson"] = "other-summary.json"
        summary["checks"]["campaignSummary"] = check

        result = self.module.validate_summary(summary, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary option and summaryJson mismatch", result.issues)

    def test_validate_summary_blocks_expected_campaign_with_stale_manifest_hash(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            campaign_summary.write_text("{}\n")
            manifest = root / "manifest.json"
            manifest.write_text('{"campaign":"before"}\n')
            summary = clean_summary()
            summary["options"]["campaignSummary"] = str(campaign_summary)
            summary["checks"]["campaignSummary"] = clean_campaign_summary_check(
                summary_json=str(campaign_summary),
                manifest=str(manifest),
                manifest_sha=sha256(manifest.read_bytes()).hexdigest(),
            )
            manifest.write_text('{"campaign":"after"}\n')

            result = self.module.validate_summary(
                summary,
                expected_campaign_summary=campaign_summary,
            )

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary manifestSha256 mismatch", result.issues)

    def test_validate_summary_requires_campaign_summary_json_hash(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        check = clean_campaign_summary_check()
        check.pop("summaryJsonSha256")
        summary["checks"]["campaignSummary"] = check

        result = self.module.validate_summary(summary, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary summaryJsonSha256 is missing", result.issues)

    def test_validate_summary_blocks_stale_campaign_summary_json_hash(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            campaign_summary.write_text('{"before":true}\n')
            manifest = root / "manifest.json"
            manifest.write_text('{"campaign":"ok"}\n')
            summary = clean_summary()
            summary["options"]["campaignSummary"] = str(campaign_summary)
            summary["checks"]["campaignSummary"] = clean_campaign_summary_check(
                summary_json=str(campaign_summary),
                summary_sha=sha256(campaign_summary.read_bytes()).hexdigest(),
                manifest=str(manifest),
                manifest_sha=sha256(manifest.read_bytes()).hexdigest(),
            )
            campaign_summary.write_text('{"after":true}\n')

            result = self.module.validate_summary(
                summary,
                expected_campaign_summary=campaign_summary,
            )

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary summaryJsonSha256 mismatch", result.issues)

    def test_validate_summary_blocks_missing_expected_campaign_summary_json(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            missing_campaign_summary = root / "missing-combined-summary.json"
            campaign_summary.write_text('{"before":true}\n')
            manifest = root / "manifest.json"
            manifest.write_text('{"campaign":"ok"}\n')
            summary = clean_summary()
            summary["options"]["campaignSummary"] = str(missing_campaign_summary)
            summary["checks"]["campaignSummary"] = clean_campaign_summary_check(
                summary_json=str(missing_campaign_summary),
                summary_sha=sha256(campaign_summary.read_bytes()).hexdigest(),
                manifest=str(manifest),
                manifest_sha=sha256(manifest.read_bytes()).hexdigest(),
            )

            result = self.module.validate_summary(
                summary,
                expected_campaign_summary=missing_campaign_summary,
            )

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary summaryJson file is missing", result.issues)

    def test_validate_summary_requires_validated_plus_historical_preset(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        check = clean_campaign_summary_check(preset="ad-hoc")
        summary["checks"]["campaignSummary"] = check

        result = self.module.validate_summary(summary, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn(
            "campaignSummary preset is ad-hoc, expected validated-plus-historical",
            result.issues,
        )

    def test_validate_summary_requires_expected_campaign_unique_task_count(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        check = clean_campaign_summary_check()
        check["uniqueTasks"] = 17
        summary["checks"]["campaignSummary"] = check

        result = self.module.validate_summary(summary, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn(
            "campaignSummary uniqueTasks is 17, expected 18",
            result.issues,
        )

    def test_validate_summary_requires_expected_campaign_projected_passes(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        check = clean_campaign_summary_check()
        check["projectedPasses"] = 67
        summary["checks"]["campaignSummary"] = check

        result = self.module.validate_summary(summary, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn(
            "campaignSummary projectedPasses is 67, expected 68",
            result.issues,
        )

    def test_validate_summary_requires_expected_campaign_expectation_set(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        check = clean_campaign_summary_check()
        check["expectRoutes"] = ["validated-conversions/medium-validated"]
        summary["checks"]["campaignSummary"] = check

        result = self.module.validate_summary(summary, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary expectRoutes mismatch", result.issues)

    def test_validate_summary_blocks_duplicate_campaign_tasks(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        check = clean_campaign_summary_check()
        check["duplicateTasks"] = 1
        check["duplicates"] = [
            {
                "taskName": "qemu-startup",
                "owners": [
                    "validated-conversions/xhigh-validated",
                    "historical-wins/environment-targeted",
                ],
            }
        ]
        summary["checks"]["campaignSummary"] = check

        result = self.module.validate_summary(summary, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary duplicateTasks is 1, expected 0", result.issues)
        self.assertIn("campaignSummary duplicates are present", result.issues)

    def test_validate_summary_requires_expected_campaign_manifest_coverage(self) -> None:
        summary = clean_summary()
        summary["options"]["campaignSummary"] = "combined-summary.json"
        check = clean_campaign_summary_check(campaigns=("validated-conversions",))
        summary["checks"]["campaignSummary"] = check

        result = self.module.validate_summary(summary, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary campaigns mismatch", result.issues)


def clean_campaign_summary_check(
    *,
    summary_json: str = "combined-summary.json",
    summary_sha: str = "b" * 64,
    preset: str = "validated-plus-historical",
    manifest: str = "validated-conversions-manifest.json",
    manifest_sha: str = "a" * 64,
    campaigns: tuple[str, ...] = ("validated-conversions", "historical-wins"),
) -> dict:
    manifests = []
    for campaign in campaigns:
        manifest_path = (
            manifest
            if campaign == "validated-conversions"
            else f"{campaign}-manifest.json"
        )
        manifests.append(
            {
                "campaign": campaign,
                "manifest": manifest_path,
                "manifestSha256": (
                    manifest_sha if campaign == "validated-conversions" else "c" * 64
                ),
            }
        )
    return {
        "status": "passed",
        "summaryJson": summary_json,
        "summaryJsonSha256": summary_sha,
        "preset": preset,
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
