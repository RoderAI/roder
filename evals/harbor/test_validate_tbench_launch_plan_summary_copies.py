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


class LaunchPlanSummaryCopyTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_rejects_embedded_generated_at_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(
                temp_path,
                generated_at="2026-05-25T10:00:00+00:00",
            )
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["preEvalSummaryStatus"]["generatedAt"] = "2026-05-25T11:00:00+00:00"

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("pre-eval summary generatedAt mismatch", result.issues)

    def test_rejects_copied_config_hash_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["harborConfig"] = str(temp_path / "tbench.json")
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["preEvalHarborConfigSha256"] = "0" * 64

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("pre-eval Harbor config summary SHA-256 mismatch", result.issues)

    def test_rejects_copied_prebuilt_hash_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["prebuiltBinary"] = {
                "path": str(temp_path / "roder-linux-amd64"),
                "sha256": "0" * 64,
            }

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("prebuilt binary summary SHA-256 mismatch", result.issues)

    def test_rejects_copied_prebuilt_executable_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            summary_data = json.loads(summary.read_text())
            plan["harborConfig"] = str(temp_path / "tbench.json")
            config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["prebuiltBinary"] = dict(summary_data["prebuiltBinary"])
            plan["authFile"] = summary_data["authFile"]
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["prebuiltBinary"]["executable"] = False

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("prebuilt binary summary executable mismatch", result.issues)

    def test_rejects_copied_prebuilt_linux_elf_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            summary_data = json.loads(summary.read_text())
            plan["harborConfig"] = str(temp_path / "tbench.json")
            config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["prebuiltBinary"] = dict(summary_data["prebuiltBinary"])
            plan["authFile"] = summary_data["authFile"]
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["prebuiltBinary"]["linuxX8664Elf"] = False

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("prebuilt binary summary linuxX8664Elf mismatch", result.issues)

    def test_rejects_copied_prebuilt_size_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            summary_data["prebuiltBinary"]["sizeBytes"] = (
                temp_path / "roder-linux-amd64"
            ).stat().st_size
            summary.write_text(json.dumps(summary_data) + "\n")

            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["harborConfig"] = str(temp_path / "tbench.json")
            config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["prebuiltBinary"] = dict(summary_data["prebuiltBinary"])
            plan["authFile"] = summary_data["authFile"]
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["prebuiltBinary"]["sizeBytes"] += 1

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("prebuilt binary summary sizeBytes mismatch", result.issues)

    def test_rejects_copied_prebuilt_modified_at_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            summary_data["prebuiltBinary"]["modifiedAt"] = "2026-05-25T10:00:00+00:00"
            summary.write_text(json.dumps(summary_data) + "\n")

            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["harborConfig"] = str(temp_path / "tbench.json")
            config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["prebuiltBinary"] = dict(summary_data["prebuiltBinary"])
            plan["authFile"] = summary_data["authFile"]
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["prebuiltBinary"]["modifiedAt"] = "2026-05-25T11:00:00+00:00"

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("prebuilt binary summary modifiedAt mismatch", result.issues)

    def test_rejects_copied_prebuilt_file_type_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            summary_data["prebuiltBinary"]["fileType"] = "ELF 64-bit x86-64"
            summary.write_text(json.dumps(summary_data) + "\n")

            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["harborConfig"] = str(temp_path / "tbench.json")
            config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["prebuiltBinary"] = dict(summary_data["prebuiltBinary"])
            plan["authFile"] = summary_data["authFile"]
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["prebuiltBinary"]["fileType"] = "Mach-O arm64"

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("prebuilt binary summary fileType mismatch", result.issues)

    def test_rejects_copied_auth_fields_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["authFile"] = {
                "path": str(temp_path / "codex.json"),
                "validJson": True,
                "jsonFields": ["access"],
            }

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("auth file summary fields mismatch", result.issues)

    def test_rejects_copied_auth_size_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            summary_data["authFile"]["sizeBytes"] = (temp_path / "codex.json").stat().st_size
            summary.write_text(json.dumps(summary_data) + "\n")

            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["harborConfig"] = str(temp_path / "tbench.json")
            config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["prebuiltBinary"] = summary_data["prebuiltBinary"]
            plan["authFile"] = dict(summary_data["authFile"])
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["authFile"]["sizeBytes"] += 1

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("auth file summary sizeBytes mismatch", result.issues)

    def test_rejects_copied_auth_modified_at_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            summary_data = json.loads(summary.read_text())
            summary_data["authFile"]["modifiedAt"] = "2026-05-25T10:00:00+00:00"
            summary.write_text(json.dumps(summary_data) + "\n")

            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["harborConfig"] = str(temp_path / "tbench.json")
            config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["prebuiltBinary"] = summary_data["prebuiltBinary"]
            plan["authFile"] = dict(summary_data["authFile"])
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["authFile"]["modifiedAt"] = "2026-05-25T11:00:00+00:00"

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("auth file summary modifiedAt mismatch", result.issues)

    def test_rejects_copied_harness_digest_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["harborHarness"] = {
                "status": "passed",
                "combinedSha256": "0" * 64,
                "entries": [],
            }

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("harbor harness summary combined SHA-256 mismatch", result.issues)

    def test_rejects_copied_harness_file_count_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            summary_data = json.loads(summary.read_text())
            plan["harborHarness"] = dict(summary_data["checks"]["harborHarness"])
            plan["harborHarness"]["files"] = 0

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("harbor harness summary files mismatch", result.issues)

    def test_rejects_copied_harness_issues_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            summary_data = json.loads(summary.read_text())
            plan["harborConfig"] = str(temp_path / "tbench.json")
            config_sha = sha256((temp_path / "tbench.json").read_bytes()).hexdigest()
            plan["harborConfigSha256"] = config_sha
            plan["preEvalHarborConfigSha256"] = config_sha
            plan["prebuiltBinary"] = summary_data["prebuiltBinary"]
            plan["authFile"] = summary_data["authFile"]
            plan["imagePreflight"] = summary_data["checks"]["imagePreflight"]
            plan["harborHarness"] = dict(summary_data["checks"]["harborHarness"])
            plan["harborHarness"]["issues"] = ["injected drift"]

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("harbor harness summary issues mismatch", result.issues)

    def test_rejects_copied_harbor_harness_tests_status_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["harborHarnessTests"] = {"status": "failed"}

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("harbor harness tests summary status mismatch", result.issues)

    def test_rejects_copied_image_preflight_status_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
            plan = ready_plan()
            plan["harborConfig"] = str(temp_path / "tbench.json")
            plan["preEvalSummary"] = str(summary)
            plan["preEvalSummarySha256"] = sha256(summary.read_bytes()).hexdigest()
            plan["imagePreflight"] = {
                "status": "failed",
                "config": str(temp_path / "tbench.json"),
            }

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("image preflight summary status mismatch", result.issues)

    def test_rejects_copied_image_preflight_selection_errors_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
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
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["imagePreflight"] = dict(summary_data["checks"]["imagePreflight"])
            plan["imagePreflight"]["selectionErrors"] = ["injected drift"]

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "image preflight summary selectionErrors mismatch",
            result.issues,
        )

    def test_rejects_copied_image_preflight_blocked_tasks_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = write_clean_summary_fixture(temp_path)
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
            plan["harborHarness"] = summary_data["checks"]["harborHarness"]
            plan["imagePreflight"] = dict(summary_data["checks"]["imagePreflight"])
            plan["imagePreflight"]["blockedTasks"] = ["injected-task"]

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_pre_eval_summary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "image preflight summary blockedTasks mismatch",
            result.issues,
        )


if __name__ == "__main__":
    unittest.main()
