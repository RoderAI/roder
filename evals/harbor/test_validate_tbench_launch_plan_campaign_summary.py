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


class ValidateTbenchLaunchPlanCampaignSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_requires_campaign_summary_when_requested(self) -> None:
        plan = ready_plan()

        result = self.module.validate_plan(plan, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("required campaign summary is not enabled", result.issues)
        self.assertIn("campaignSummary is missing", result.issues)

    def test_blocks_malformed_campaign_manifest_hash(self) -> None:
        plan = ready_plan()
        plan["requireCampaignSummary"] = True
        plan["campaignSummary"] = clean_campaign_summary_check(
            Path("combined-summary.json")
        )
        plan["campaignSummary"]["manifests"][0]["manifestSha256"] = "not-a-sha"

        result = self.module.validate_plan(plan, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary manifestSha256 is invalid", result.issues)

    def test_blocks_missing_campaign_summary_json_hash(self) -> None:
        plan = ready_plan()
        plan["requireCampaignSummary"] = True
        plan["campaignSummary"] = clean_campaign_summary_check(
            Path("combined-summary.json")
        )
        plan["campaignSummary"].pop("summaryJsonSha256")

        result = self.module.validate_plan(plan, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary summaryJsonSha256 is missing", result.issues)

    def test_requires_validated_plus_historical_preset(self) -> None:
        plan = ready_plan()
        plan["requireCampaignSummary"] = True
        plan["campaignSummary"] = clean_campaign_summary_check(
            Path("combined-summary.json"),
            preset="ad-hoc",
        )

        result = self.module.validate_plan(plan, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn(
            "campaignSummary preset is ad-hoc, expected validated-plus-historical",
            result.issues,
        )

    def test_requires_expected_campaign_unique_task_count(self) -> None:
        plan = ready_plan()
        plan["requireCampaignSummary"] = True
        plan["campaignSummary"] = clean_campaign_summary_check(
            Path("combined-summary.json")
        )
        plan["campaignSummary"]["uniqueTasks"] = 17

        result = self.module.validate_plan(plan, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn(
            "campaignSummary uniqueTasks is 17, expected 18",
            result.issues,
        )

    def test_requires_expected_campaign_projected_passes(self) -> None:
        plan = ready_plan()
        plan["requireCampaignSummary"] = True
        plan["campaignSummary"] = clean_campaign_summary_check(
            Path("combined-summary.json")
        )
        plan["campaignSummary"]["projectedPasses"] = 67

        result = self.module.validate_plan(plan, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn(
            "campaignSummary projectedPasses is 67, expected 68",
            result.issues,
        )

    def test_requires_expected_campaign_expectation_set(self) -> None:
        plan = ready_plan()
        plan["requireCampaignSummary"] = True
        plan["campaignSummary"] = clean_campaign_summary_check(
            Path("combined-summary.json")
        )
        plan["campaignSummary"]["expectOwners"] = [
            "password-recovery=historical-wins/policy-framed",
        ]

        result = self.module.validate_plan(plan, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary expectOwners mismatch", result.issues)

    def test_blocks_duplicate_campaign_tasks(self) -> None:
        plan = ready_plan()
        plan["requireCampaignSummary"] = True
        plan["campaignSummary"] = clean_campaign_summary_check(
            Path("combined-summary.json")
        )
        plan["campaignSummary"]["duplicateTasks"] = 1
        plan["campaignSummary"]["duplicates"] = [
            {
                "taskName": "qemu-startup",
                "owners": [
                    "validated-conversions/xhigh-validated",
                    "historical-wins/environment-targeted",
                ],
            }
        ]

        result = self.module.validate_plan(plan, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary duplicateTasks is 1, expected 0", result.issues)
        self.assertIn("campaignSummary duplicates are present", result.issues)

    def test_requires_expected_campaign_manifest_coverage(self) -> None:
        plan = ready_plan()
        plan["requireCampaignSummary"] = True
        plan["campaignSummary"] = clean_campaign_summary_check(
            Path("combined-summary.json"),
            campaigns=("validated-conversions",),
        )

        result = self.module.validate_plan(plan, require_campaign_summary=True)

        self.assertFalse(result.ok)
        self.assertIn("campaignSummary campaigns mismatch", result.issues)

    def test_verifies_campaign_summary_copy_from_pre_eval_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            summary_data["options"]["campaignSummary"] = str(
                temp_path / "combined-summary.json"
            )
            summary_data["checks"]["campaignSummary"] = clean_campaign_summary_check(
                temp_path / "combined-summary.json"
            )
            summary.write_text(json.dumps(summary_data) + "\n")
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["requireCampaignSummary"] = True
            plan["campaignSummary"] = dict(summary_data["checks"]["campaignSummary"])
            plan["campaignSummary"]["projectedPasses"] = 67

            result = self.module.validate_plan(
                plan,
                require_campaign_summary=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("campaign summary projectedPasses mismatch", result.issues)

    def test_verify_pre_eval_summary_blocks_stale_campaign_manifest_hash(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            campaign_summary = temp_path / "combined-summary.json"
            campaign_summary.write_text("{}\n")
            manifest = temp_path / "manifest.json"
            manifest.write_text('{"campaign":"before"}\n')
            summary_data = json.loads(summary.read_text())
            summary_data["options"]["campaignSummary"] = str(campaign_summary)
            summary_data["checks"]["campaignSummary"] = clean_campaign_summary_check(
                campaign_summary,
                manifest=str(manifest),
                manifest_sha=sha256(manifest.read_bytes()).hexdigest(),
            )
            summary.write_text(json.dumps(summary_data) + "\n")
            manifest.write_text('{"campaign":"after"}\n')
            plan = ready_plan()
            config = temp_path / "tbench.json"
            config_sha = sha256(config.read_bytes()).hexdigest()
            plan["harborConfig"] = str(config)
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["prebuiltBinary"] = summary_data["prebuiltBinary"]
            plan["authFile"] = summary_data["authFile"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["requireCampaignSummary"] = True
            plan["campaignSummary"] = dict(summary_data["checks"]["campaignSummary"])

            result = self.module.validate_plan(
                plan,
                require_campaign_summary=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "pre-eval summary validation: campaignSummary manifestSha256 mismatch",
            result.issues,
        )

    def test_verify_pre_eval_summary_blocks_stale_campaign_summary_json_hash(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            campaign_summary = temp_path / "combined-summary.json"
            campaign_summary.write_text('{"before":true}\n')
            manifest = temp_path / "manifest.json"
            manifest.write_text('{"campaign":"ok"}\n')
            summary_data = json.loads(summary.read_text())
            summary_data["options"]["campaignSummary"] = str(campaign_summary)
            summary_data["checks"]["campaignSummary"] = clean_campaign_summary_check(
                campaign_summary,
                summary_sha=sha256(campaign_summary.read_bytes()).hexdigest(),
                manifest=str(manifest),
                manifest_sha=sha256(manifest.read_bytes()).hexdigest(),
            )
            summary.write_text(json.dumps(summary_data) + "\n")
            campaign_summary.write_text('{"after":true}\n')
            plan = ready_plan()
            config = temp_path / "tbench.json"
            config_sha = sha256(config.read_bytes()).hexdigest()
            plan["harborConfig"] = str(config)
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["prebuiltBinary"] = summary_data["prebuiltBinary"]
            plan["authFile"] = summary_data["authFile"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["requireCampaignSummary"] = True
            plan["campaignSummary"] = dict(summary_data["checks"]["campaignSummary"])

            result = self.module.validate_plan(
                plan,
                require_campaign_summary=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "pre-eval summary validation: campaignSummary summaryJsonSha256 mismatch",
            result.issues,
        )


def clean_campaign_summary_check(
    path: Path,
    *,
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
        "summaryJson": str(path),
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
