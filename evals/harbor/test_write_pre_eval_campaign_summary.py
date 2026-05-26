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

from pre_eval_summary_test_helpers import build_summary, load_module


class PreEvalCampaignSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_summary_records_valid_combined_campaign_handoff(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(campaign_summary, validation_status="ok")
            expected_manifests = campaign_manifest_entries(root)
            expected_summary_sha = sha256(campaign_summary.read_bytes()).hexdigest()

            summary = build_summary(
                self.module,
                root,
                tbench_outcomes=["pass"],
                campaign_summary=campaign_summary,
            )

        self.assertEqual("ok", summary["status"])
        self.assertEqual([], summary["blockedChecks"])
        check = summary["checks"]["campaignSummary"]
        self.assertEqual("passed", check["status"])
        self.assertEqual(str(campaign_summary), check["summaryJson"])
        self.assertEqual(expected_summary_sha, check["summaryJsonSha256"])
        self.assertEqual("validated-plus-historical", check["preset"])
        self.assertEqual("ok", check["validationStatus"])
        self.assertEqual(18, check["uniqueTasks"])
        self.assertEqual(68, check["projectedPasses"])
        self.assertEqual(True, check["requireNoOverlap"])
        self.assertEqual(18, check["expectUniqueTasks"])
        self.assertEqual(68, check["expectProjectedPasses"])
        self.assertEqual(
            ["validated-conversions", "historical-wins"],
            check["expectCampaigns"],
        )
        self.assertEqual(
            [
                "validated-conversions/medium-validated",
                "validated-conversions/xhigh-validated",
                "validated-conversions/xhigh-plan-first",
                "historical-wins/policy-framed",
                "historical-wins/environment-targeted",
            ],
            check["expectRoutes"],
        )
        self.assertEqual(
            ["password-recovery", "qemu-startup", "vulnerable-secret"],
            check["expectTasks"],
        )
        self.assertEqual(
            [
                "password-recovery=historical-wins/policy-framed",
                "qemu-startup=historical-wins/environment-targeted",
                "vulnerable-secret=historical-wins/policy-framed",
            ],
            check["expectOwners"],
        )
        self.assertEqual(
            expected_manifests,
            check["manifests"],
        )

    def test_summary_blocks_failed_combined_campaign_handoff(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(
                campaign_summary,
                validation_status="blocked",
                issues=["missing expected routes: historical-wins/policy-framed"],
            )

            summary = build_summary(
                self.module,
                root,
                tbench_outcomes=["pass"],
                campaign_summary=campaign_summary,
            )

        self.assertEqual("blocked", summary["status"])
        self.assertEqual(["campaignSummary"], summary["blockedChecks"])
        check = summary["checks"]["campaignSummary"]
        self.assertEqual("failed", check["status"])
        self.assertEqual(
            ["missing expected routes: historical-wins/policy-framed"],
            check["issues"],
        )

    def test_summary_blocks_combined_campaign_without_manifest_hashes(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(campaign_summary, validation_status="ok")
            data = json.loads(campaign_summary.read_text())
            data["campaigns"][0].pop("manifestSha256")
            campaign_summary.write_text(json.dumps(data))

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
            "campaign manifest SHA-256 is missing",
            summary["checks"]["campaignSummary"]["issues"],
        )

    def test_summary_blocks_combined_campaign_with_malformed_manifest_hash(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(campaign_summary, validation_status="ok")
            data = json.loads(campaign_summary.read_text())
            data["campaigns"][0]["manifestSha256"] = "not-a-sha"
            campaign_summary.write_text(json.dumps(data))

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
            "campaign manifest SHA-256 is invalid",
            summary["checks"]["campaignSummary"]["issues"],
        )

    def test_summary_blocks_combined_campaign_with_stale_manifest_hash(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(campaign_summary, validation_status="ok")
            manifest = root / "validated-conversions-manifest.json"
            manifest.write_text('{"campaign":"changed"}\n')

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
            "campaign manifest SHA-256 mismatch",
            summary["checks"]["campaignSummary"]["issues"],
        )

    def test_summary_blocks_validated_plus_historical_with_wrong_unique_task_count(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(
                campaign_summary,
                validation_status="ok",
                unique_tasks=17,
            )

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
            "campaign uniqueTasks expected 18, got 17",
            summary["checks"]["campaignSummary"]["issues"],
        )

    def test_summary_blocks_validated_plus_historical_with_wrong_projected_passes(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(
                campaign_summary,
                validation_status="ok",
                projected_passes=67,
            )

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
            "campaign projectedPasses expected 68, got 67",
            summary["checks"]["campaignSummary"]["issues"],
        )

    def test_summary_blocks_combined_campaign_without_expected_preset(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(
                campaign_summary,
                validation_status="ok",
                preset=None,
            )

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
            "campaign preset is missing",
            summary["checks"]["campaignSummary"]["issues"],
        )

    def test_summary_blocks_combined_campaign_with_unexpected_preset(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(
                campaign_summary,
                validation_status="ok",
                preset="ad-hoc",
            )

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
            "campaign preset is ad-hoc, expected validated-plus-historical",
            summary["checks"]["campaignSummary"]["issues"],
        )

    def test_summary_blocks_validated_plus_historical_without_expectation_set(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(
                campaign_summary,
                validation_status="ok",
                include_expectations=False,
            )

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
            "campaign expectRoutes mismatch",
            summary["checks"]["campaignSummary"]["issues"],
        )

    def test_summary_blocks_validated_plus_historical_with_incomplete_expectation_set(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(
                campaign_summary,
                validation_status="ok",
                expect_routes=["validated-conversions/medium-validated"],
            )

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
            "campaign expectRoutes mismatch",
            summary["checks"]["campaignSummary"]["issues"],
        )

    def test_summary_blocks_validated_plus_historical_with_duplicate_tasks(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            campaign_summary = root / "combined-summary.json"
            write_campaign_summary(
                campaign_summary,
                validation_status="ok",
                duplicate_tasks=1,
                duplicates=[
                    {
                        "taskName": "qemu-startup",
                        "owners": [
                            "validated-conversions/xhigh-validated",
                            "historical-wins/environment-targeted",
                        ],
                    }
                ],
            )

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
            "campaign duplicateTasks expected 0, got 1",
            summary["checks"]["campaignSummary"]["issues"],
        )
        self.assertIn(
            "campaign duplicates are present",
            summary["checks"]["campaignSummary"]["issues"],
        )


def write_campaign_summary(
    path: Path,
    *,
    validation_status: str,
    issues: list[str] | None = None,
    preset: str | None = "validated-plus-historical",
    unique_tasks: int = 18,
    projected_passes: int = 68,
    include_expectations: bool = True,
    expect_routes: list[str] | None = None,
    duplicate_tasks: int = 0,
    duplicates: list[dict] | None = None,
    campaigns: list[str] | None = None,
) -> None:
    campaign_names = campaigns or ["validated-conversions", "historical-wins"]
    manifests = []
    for campaign in campaign_names:
        manifest = path.parent / f"{campaign}-manifest.json"
        manifest.write_text(json.dumps({"campaign": campaign}) + "\n")
        manifests.append(
            {
                "campaign": campaign,
                "manifest": str(manifest),
                "manifestSha256": sha256(manifest.read_bytes()).hexdigest(),
            }
        )
    validation = {
        "status": validation_status,
        "preset": preset,
        "issues": issues or [],
    }
    if include_expectations:
        validation.update(
            {
                "requireNoOverlap": True,
                "expectUniqueTasks": 18,
                "expectProjectedPasses": 68,
                "expectCampaigns": ["validated-conversions", "historical-wins"],
                "expectRoutes": expect_routes
                or [
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
            }
        )
    path.write_text(
        json.dumps(
            {
                "validation": validation,
                "summary": {
                    "uniqueTasks": unique_tasks,
                    "duplicateTasks": duplicate_tasks,
                },
                "scoreProjection": {
                    "projectedPassesIfAllRoutesPass": projected_passes,
                },
                "duplicates": duplicates or [],
                "campaigns": manifests,
            }
        )
    )


def campaign_manifest_entries(root: Path) -> list[dict]:
    entries = []
    for campaign in ("validated-conversions", "historical-wins"):
        manifest = root / f"{campaign}-manifest.json"
        entries.append(
            {
                "campaign": campaign,
                "manifest": str(manifest),
                "manifestSha256": sha256(manifest.read_bytes()).hexdigest(),
            }
        )
    return entries


if __name__ == "__main__":
    unittest.main()
