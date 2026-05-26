#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from run_roder_tbench_full_test_helpers import SCRIPT, ROOT, clean_summary


class RunRoderTbenchFullDryRunImageGateTests(unittest.TestCase):
    def test_dry_run_rejects_missing_image_preflight_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            data = clean_summary(preflight_images=True, prebuilt_binary=prebuilt)
            data["checks"]["imagePreflight"]["manifest"] = str(
                temp_path / "missing-manifest.json"
            )
            summary.write_text(json.dumps(data, indent=2) + "\n")
            launch_plan = temp_path / "launch-plan.json"

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.update(
                {
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_LAUNCH_PLAN": str(launch_plan),
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
        self.assertIn("imagePreflight manifest cannot be read:", result.stderr)
        self.assertNotIn("Full run dry-run passed", result.stdout)

    def test_dry_run_launch_plan_rejects_nonclean_image_preflight_details(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            data = clean_summary(preflight_images=True, prebuilt_binary=prebuilt)
            data["checks"]["imagePreflight"].update(
                {
                    "missing": 1,
                    "unresolved": 1,
                    "pullFailed": 1,
                    "selectionErrors": ["unresolved task image"],
                    "blockedTasks": [
                        {"taskName": "missing-image-task", "status": "missing"}
                    ],
                }
            )
            summary.write_text(json.dumps(data, indent=2) + "\n")
            launch_plan = temp_path / "launch-plan.json"

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.update(
                {
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_LAUNCH_PLAN": str(launch_plan),
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
        self.assertIn("imagePreflight missing is 1", result.stderr)
        self.assertIn("imagePreflight unresolved is 1", result.stderr)
        self.assertIn("imagePreflight pullFailed is 1", result.stderr)
        self.assertIn("imagePreflight selectionErrors are non-empty", result.stderr)
        self.assertIn("imagePreflight blockedTasks are non-empty", result.stderr)
        self.assertNotIn("Full run dry-run passed", result.stdout)

    def test_dry_run_launch_plan_uses_reused_summary_output_dir(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            data = clean_summary(preflight_images=True, prebuilt_binary=prebuilt)
            data["outputDir"] = str(temp_path)
            summary.write_text(json.dumps(data, indent=2) + "\n")
            launch_plan = temp_path / "launch-plan.json"

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.update(
                {
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_LAUNCH_PLAN": str(launch_plan),
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
            plan_data = (
                json.loads(launch_plan.read_text())
                if launch_plan.exists()
                else None
            )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIsNotNone(plan_data)
        self.assertEqual(str(temp_path), plan_data["preEvalOutputDir"])

    def test_dry_run_launch_plan_uses_reused_summary_preflight_options(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            data = clean_summary(preflight_images=True, prebuilt_binary=prebuilt)
            data["options"]["pullImages"] = True
            summary.write_text(json.dumps(data, indent=2) + "\n")
            launch_plan = temp_path / "launch-plan.json"

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.pop("RODER_HARBOR_PREFLIGHT_PULL", None)
            env.update(
                {
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_LAUNCH_PLAN": str(launch_plan),
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
            plan_data = (
                json.loads(launch_plan.read_text())
                if launch_plan.exists()
                else None
            )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIsNotNone(plan_data)
        self.assertTrue(plan_data["requireImagePreflight"])
        self.assertTrue(plan_data["pullPreflight"])

    def test_dry_run_verifies_reused_summary_manifest_when_skip_preflight_is_set(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            data = clean_summary(preflight_images=True, prebuilt_binary=prebuilt)
            data["checks"]["imagePreflight"]["manifest"] = str(
                temp_path / "missing-manifest.json"
            )
            summary.write_text(json.dumps(data, indent=2) + "\n")
            launch_plan = temp_path / "launch-plan.json"

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.update(
                {
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_SKIP_PREFLIGHT": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_LAUNCH_PLAN": str(launch_plan),
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
        self.assertIn("imagePreflight manifest cannot be read:", result.stderr)
        self.assertNotIn("Full run dry-run passed", result.stdout)


if __name__ == "__main__":
    unittest.main()
