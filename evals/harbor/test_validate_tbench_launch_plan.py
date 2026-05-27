#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from hashlib import sha256
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
MODULE_PATH = ROOT / "evals/harbor/validate_tbench_launch_plan.py"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from launch_plan_test_helpers import (  # noqa: E402
    combined_file_digest,
    ready_plan,
    write_clean_summary_fixture,
)


def load_module():
    spec = importlib.util.spec_from_file_location(
        "validate_tbench_launch_plan",
        MODULE_PATH,
    )
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class ValidateTbenchLaunchPlanTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_ready_plan_passes_require_ready_gate(self) -> None:
        result = self.module.validate_plan(ready_plan(), require_ready=True)

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_dry_run_plan_passes_when_allowed(self) -> None:
        plan = ready_plan()
        plan.update(
            {
                "launchStatus": "dry_run",
                "dryRun": True,
                "wouldRunHarbor": False,
            }
        )

        result = self.module.validate_plan(plan, allow_dry_run=True)

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_dry_run_plan_requires_dry_run_execution_flags(self) -> None:
        plan = ready_plan()
        plan.update(
            {
                "launchStatus": "dry_run",
                "dryRun": False,
                "wouldRunHarbor": True,
            }
        )

        result = self.module.validate_plan(plan, allow_dry_run=True)

        self.assertFalse(result.ok)
        self.assertIn("dry_run launch plan dryRun is not true", result.issues)
        self.assertIn("dry_run launch plan would run Harbor", result.issues)

    def test_blocked_plan_reports_blocked_reasons(self) -> None:
        plan = ready_plan()
        plan.update(
            {
                "launchStatus": "blocked",
                "blockedReasons": ["existing_job_dir"],
                "wouldRunHarbor": False,
                "jobDirBlocksLaunch": True,
                "blockedBeforeHarbor": "existing_job_dir",
            }
        )

        result = self.module.validate_plan(plan, require_ready=True)

        self.assertFalse(result.ok)
        self.assertIn("launch blocked: existing_job_dir", result.issues)
        self.assertIn("launchStatus is blocked, expected ready", result.issues)

    def test_ready_plan_requires_would_run_harbor(self) -> None:
        plan = ready_plan()
        plan["wouldRunHarbor"] = False

        result = self.module.validate_plan(plan, require_ready=True)

        self.assertFalse(result.ok)
        self.assertIn("ready launch plan would not run Harbor", result.issues)

    def test_ready_plan_requires_deadline_policy(self) -> None:
        plan = ready_plan()
        plan.pop("deadlinePolicy")

        result = self.module.validate_plan(plan, require_ready=True)

        self.assertFalse(result.ok)
        self.assertIn("deadlinePolicy is missing", result.issues)

    def test_ready_plan_rejects_deadline_policy_drift(self) -> None:
        plan = ready_plan()
        plan["deadlinePolicy"] = {
            "overrideTimeoutSec": 900,
            "softTimeoutSec": 890,
            "evalDeadlineSeconds": 870,
        }

        result = self.module.validate_plan(plan, require_ready=True)

        self.assertFalse(result.ok)
        text = "\n".join(result.issues)
        self.assertIn("deadlinePolicy overrideTimeoutSec", text)
        self.assertIn("deadlinePolicy softTimeoutSec", text)
        self.assertIn("deadlinePolicy evalDeadlineSeconds", text)

    def test_ready_plan_passes_image_preflight_requirement(self) -> None:
        result = self.module.validate_plan(
            ready_plan(),
            require_ready=True,
            require_image_preflight=True,
        )

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_ready_plan_fails_when_image_preflight_was_skipped(self) -> None:
        plan = ready_plan()
        plan["requireImagePreflight"] = False
        plan["skipPreflight"] = True

        result = self.module.validate_plan(
            plan,
            require_ready=True,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("required image preflight is not enabled", result.issues)

    def test_ready_plan_rejects_image_preflight_for_different_config(self) -> None:
        plan = ready_plan()
        plan["imagePreflight"]["config"] = "evals/harbor/tbench-smoke.json"

        result = self.module.validate_plan(
            plan,
            require_ready=True,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn(
            "image preflight config does not match Harbor config",
            result.issues,
        )

    def test_ready_plan_rejects_unknown_image_preflight_source(self) -> None:
        plan = ready_plan()
        plan["imagePreflightSource"] = "stale_manifest"

        result = self.module.validate_plan(
            plan,
            require_ready=True,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflightSource is stale_manifest", result.issues)

    def test_ready_plan_rejects_embedded_image_preflight_source_mismatch(self) -> None:
        plan = ready_plan()
        plan["imagePreflightSource"] = "route_manifest"
        plan["imagePreflight"]["source"] = "pre_eval_summary"

        result = self.module.validate_plan(
            plan,
            require_ready=True,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight.source mismatch", result.issues)

    def test_ready_plan_rejects_nonclean_image_preflight_details(self) -> None:
        plan = ready_plan()
        plan["imagePreflight"]["missing"] = 1
        plan["imagePreflight"]["unresolved"] = 1
        plan["imagePreflight"]["pullFailed"] = 1
        plan["imagePreflight"]["selectionErrors"] = ["unresolved task image"]
        plan["imagePreflight"]["blockedTasks"] = [
            {"taskName": "missing-image-task", "status": "missing"}
        ]

        result = self.module.validate_plan(
            plan,
            require_ready=True,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight missing is 1", result.issues)
        self.assertIn("imagePreflight unresolved is 1", result.issues)
        self.assertIn("imagePreflight pullFailed is 1", result.issues)
        self.assertIn("imagePreflight selectionErrors are non-empty", result.issues)
        self.assertIn("imagePreflight blockedTasks are non-empty", result.issues)

    def test_ready_plan_requires_image_preflight_manifest_and_counts(self) -> None:
        plan = ready_plan()
        for field in ("manifest", "tasks", "uniqueImages", "present"):
            plan["imagePreflight"].pop(field, None)

        result = self.module.validate_plan(
            plan,
            require_ready=True,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight manifest is missing", result.issues)
        self.assertIn("imagePreflight tasks is missing", result.issues)
        self.assertIn("imagePreflight uniqueImages is missing", result.issues)
        self.assertIn("imagePreflight present is missing", result.issues)

    def test_ready_plan_rejects_inconsistent_image_preflight_counts(self) -> None:
        plan = ready_plan()
        plan["imagePreflight"]["tasks"] = 0
        plan["imagePreflight"]["uniqueImages"] = 2
        plan["imagePreflight"]["present"] = 1

        result = self.module.validate_plan(
            plan,
            require_ready=True,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight tasks is not positive", result.issues)
        self.assertIn(
            "imagePreflight present exceeds tasks",
            result.issues,
        )
        self.assertIn("imagePreflight uniqueImages exceeds tasks", result.issues)

    def test_plan_fails_when_embedded_pre_eval_summary_is_blocked(self) -> None:
        plan = ready_plan()
        plan["preEvalSummaryStatus"] = {
            "status": "blocked",
            "blockedChecks": ["imagePreflight"],
        }

        result = self.module.validate_plan(plan, require_ready=True)

        self.assertFalse(result.ok)
        self.assertIn("pre-eval summary status is blocked", result.issues)
        self.assertIn(
            "pre-eval summary blocked checks: imagePreflight",
            result.issues,
        )

    def test_plan_requires_embedded_pre_eval_summary_status(self) -> None:
        plan = ready_plan()
        plan.pop("preEvalSummaryStatus")

        result = self.module.validate_plan(plan, require_ready=True)

        self.assertFalse(result.ok)
        self.assertIn("preEvalSummaryStatus is missing", result.issues)

    def test_plan_requires_analysis_output_paths(self) -> None:
        plan = ready_plan()
        plan["analysisJson"] = "evals/reports/harbor/full-analysis.json"
        plan["analysisMarkdown"] = "evals/reports/harbor/full-analysis.md"
        plan.pop("analysisJson")
        plan.pop("analysisMarkdown")

        result = self.module.validate_plan(plan, require_ready=True)

        self.assertFalse(result.ok)
        self.assertIn("analysisJson is missing", result.issues)
        self.assertIn("analysisMarkdown is missing", result.issues)

    def test_verify_harness_files_blocks_missing_required_file_set(self) -> None:
        harness = ROOT / "evals/harbor/roder_harbor_agent.py"
        digest = sha256(harness.read_bytes()).hexdigest()
        plan = ready_plan()
        plan["harborHarness"] = {
            "status": "passed",
            "combinedSha256": combined_file_digest(
                [{"path": str(harness), "sha256": digest}]
            ),
            "entries": [{"path": str(harness), "sha256": digest}],
        }

        result = self.module.validate_plan(
            plan,
            require_ready=True,
            verify_harness_files=True,
        )

        self.assertFalse(result.ok)
        self.assertIn(
            "harborHarness required file missing: "
            "evals/harbor/pre_eval_image_preflight_validation.py",
            result.issues,
        )

    def test_verify_pre_eval_summary_file_passes_matching_sha(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["harborConfig"] = str(temp_path / "tbench.json")
            config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["imagePreflight"]["config"] = str(temp_path / "tbench.json")
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            summary_data = json.loads(summary.read_text())
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["prebuiltBinary"] = summary_data["prebuiltBinary"]
            plan["authFile"] = summary_data["authFile"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_route_manifest_image_preflight_does_not_require_summary_image_preflight(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path, image_preflight=False)
            summary_data = json.loads(summary.read_text())
            config = temp_path / "tbench.json"
            manifest = temp_path / "route-images.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": str(config),
                        "offline": True,
                        "pull": False,
                        "summary": {
                            "tasks": 1,
                            "unique_images": 1,
                            "present": 1,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {
                                "task_name": "route-task",
                                "image": "terminalbench/route-task:latest",
                                "status": "present",
                                "image_source": "task",
                            }
                        ],
                        "images": [
                            {
                                "image": "terminalbench/route-task:latest",
                                "tasks": ["route-task"],
                            }
                        ],
                    }
                )
                + "\n"
            )
            image_preflight = {
                "status": "passed",
                "source": "route_manifest",
                "config": str(config),
                "manifest": str(manifest),
                "tasks": 1,
                "uniqueImages": 1,
                "present": 1,
                "missing": 0,
                "unresolved": 0,
                "pullFailed": 0,
                "offline": True,
                "selectionErrors": [],
                "blockedTasks": [],
            }
            plan = ready_plan()
            config_sha = sha256(config.read_bytes()).hexdigest()
            plan["harborConfig"] = str(config)
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["prebuiltBinary"] = summary_data["prebuiltBinary"]
            plan["authFile"] = summary_data["authFile"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["requireImagePreflight"] = True
            plan["imagePreflightSource"] = "route_manifest"
            plan["imagePreflight"] = image_preflight

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                require_image_preflight=True,
                verify_pre_eval_summary=True,
                verify_image_manifest=True,
            )

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_verify_pre_eval_summary_file_rejects_output_dir_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            summary_data["outputDir"] = str(temp_path)
            summary.write_text(json.dumps(summary_data) + "\n")
            plan = ready_plan()
            plan["harborConfig"] = str(temp_path / "tbench.json")
            config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["prebuiltBinary"] = summary_data["prebuiltBinary"]
            plan["authFile"] = summary_data["authFile"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["preEvalOutputDir"] = str(temp_path / "other-pre-eval")

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("pre-eval summary outputDir mismatch", result.issues)

    def test_verify_pre_eval_summary_file_rejects_missing_launched_config_entry(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["prebuiltBinary"] = summary_data["prebuiltBinary"]
            plan["authFile"] = summary_data["authFile"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["harborConfig"] = "evals/reports/harbor/campaigns/missing-route.json"
            plan["harborConfigSha256"] = "a" * 64
            plan["preEvalHarborConfigSha256"] = "a" * 64

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "pre-eval summary missing Harbor config entry: "
            "evals/reports/harbor/campaigns/missing-route.json",
            result.issues,
        )

    def test_verify_pre_eval_summary_file_rejects_blocked_summary_even_with_matching_sha(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = Path(temp) / "pre-eval-summary.json"
            summary.write_text(
                json.dumps({"status": "blocked", "blockedChecks": ["harborReadiness"]})
                + "\n"
            )
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("pre-eval summary validation: summary status is blocked", result.issues)

    def test_verify_pre_eval_summary_file_rejects_stale_summary_when_max_age_is_set(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(
                temp_path,
                generated_at="2026-05-25T10:00:00+00:00",
            )
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
                max_pre_eval_age_seconds=3600,
                now=datetime(2026, 5, 25, 12, 0, 1, tzinfo=timezone.utc),
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "pre-eval summary validation: summary is stale: age 7201s exceeds max 3600s",
            result.issues,
        )

    def test_verify_pre_eval_summary_file_uses_plan_max_age_when_cli_age_is_absent(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(
                temp_path,
                generated_at="2026-05-25T10:00:00+00:00",
            )
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["maxPreEvalAgeSeconds"] = 3600

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
                now=datetime(2026, 5, 25, 12, 0, 1, tzinfo=timezone.utc),
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "pre-eval summary validation: summary is stale: age 7201s exceeds max 3600s",
            result.issues,
        )

    def test_verify_pre_eval_summary_file_rejects_mismatched_sha(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = Path(temp) / "pre-eval-summary.json"
            summary.write_text('{"status":"ok"}\n')
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = "0" * 64

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("pre-eval summary SHA-256 mismatch", result.issues)

    def test_verify_harbor_config_file_passes_matching_sha(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            config = Path(temp) / "tbench.json"
            config.write_text('{"job_name":"test"}\n')
            plan = ready_plan()
            plan["harborConfig"] = str(config)
            config_sha = sha256(config.read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_harbor_config=True,
            )

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_verify_harbor_config_file_rejects_mismatched_sha(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            config = Path(temp) / "tbench.json"
            config.write_text('{"job_name":"test"}\n')
            plan = ready_plan()
            plan["harborConfig"] = str(config)
            plan["harborConfigSha256"] = "0" * 64

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_harbor_config=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("Harbor config SHA-256 mismatch", result.issues)

    def test_verify_harbor_config_rejects_pre_eval_config_hash_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            config = Path(temp) / "tbench.json"
            config.write_text('{"job_name":"test"}\n')
            plan = ready_plan()
            plan["harborConfig"] = str(config)
            plan["harborConfigSha256"] = sha256(config.read_bytes()).hexdigest()
            plan["preEvalHarborConfigSha256"] = "0" * 64

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_harbor_config=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("pre-eval Harbor config SHA-256 mismatch", result.issues)

if __name__ == "__main__":
    unittest.main()
