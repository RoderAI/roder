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


def bind_plan_to_summary(summary: Path, temp_path: Path) -> dict:
    summary_data = json.loads(summary.read_text())
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
    return plan


class LaunchPlanOptionCopyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_rejects_preflight_option_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            summary_data["options"]["preflightImages"] = True
            summary.write_text(json.dumps(summary_data) + "\n")
            plan = bind_plan_to_summary(summary, temp_path)
            plan["requireImagePreflight"] = False

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("pre-eval summary preflightImages mismatch", result.issues)

    def test_rejects_pull_option_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            summary_data["options"]["pullImages"] = True
            summary.write_text(json.dumps(summary_data) + "\n")
            plan = bind_plan_to_summary(summary, temp_path)
            plan["pullPreflight"] = False

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("pre-eval summary pullImages mismatch", result.issues)

    def test_rejects_offline_option_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            summary_data["options"]["offlineImages"] = True
            summary_data["checks"]["imagePreflight"]["offline"] = True
            summary.write_text(json.dumps(summary_data) + "\n")
            plan = bind_plan_to_summary(summary, temp_path)
            plan["offlinePreflight"] = False

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("pre-eval summary offlineImages mismatch", result.issues)


if __name__ == "__main__":
    unittest.main()
