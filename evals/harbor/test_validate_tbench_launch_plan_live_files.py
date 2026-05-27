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
MODULE_PATH = ROOT / "evals/harbor/validate_tbench_launch_plan.py"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from launch_plan_test_helpers import (  # noqa: E402
    auth_json,
    executable_file,
    linux_x86_64_elf_bytes,
    ready_plan,
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


class LaunchPlanLiveFileTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_verify_prebuilt_binary_passes_matching_summary_sha(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            binary = executable_file(
                Path(temp) / "roder-linux-amd64",
                linux_x86_64_elf_bytes(),
            )
            plan = ready_plan()
            plan["prebuiltBinary"] = {
                "path": str(binary),
                "sha256": sha256(binary.read_bytes()).hexdigest(),
            }

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_prebuilt_binary=True,
            )

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_verify_prebuilt_binary_rejects_mismatched_summary_sha(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            binary = executable_file(Path(temp) / "roder-linux-amd64", b"roder binary")
            plan = ready_plan()
            plan["prebuiltBinary"] = {
                "path": str(binary),
                "sha256": "0" * 64,
            }

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_prebuilt_binary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("prebuilt binary SHA-256 mismatch", result.issues)

    def test_verify_auth_file_passes_valid_json_object(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            auth = Path(temp) / "codex.json"
            auth.write_text(auth_json())
            plan = ready_plan()
            plan["authFile"] = {"path": str(auth), "validJson": True}

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_auth_file=True,
            )

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_verify_auth_file_rejects_invalid_json(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            auth = Path(temp) / "codex.json"
            auth.write_text("{not-json")
            plan = ready_plan()
            plan["authFile"] = {"path": str(auth), "validJson": True}

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_auth_file=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("auth file JSON is invalid", result.issues)

    def test_verify_auth_file_rejects_missing_required_fields(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            auth = Path(temp) / "codex.json"
            auth.write_text('{"access":"token"}\n')
            plan = ready_plan()
            plan["authFile"] = {"path": str(auth), "validJson": True}

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_auth_file=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "auth file missing required auth field(s): refresh, account_id, type",
            result.issues,
        )
        self.assertIn("auth file missing required auth field(s): expires", result.issues)

    def test_verify_harness_files_passes_matching_digest(self) -> None:
        files = [
            {"path": str(path), "sha256": sha256(path.read_bytes()).hexdigest()}
            for path in self.module.DEFAULT_HARNESS_FILES
        ]
        plan = ready_plan()
        plan["harborHarness"] = {
            "status": "passed",
            "files": len(files),
            "entries": files,
            "combinedSha256": self.module.combined_file_digest(files),
        }

        result = self.module.validate_plan(
            plan,
            require_ready=True,
            verify_harness_files=True,
        )

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_verify_harness_files_rejects_changed_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            first = Path(temp) / "roder_harbor_agent.py"
            first.write_text("print('first')\n")
            files = [{"path": str(first), "sha256": sha256(first.read_bytes()).hexdigest()}]
            plan = ready_plan()
            plan["harborHarness"] = {
                "status": "passed",
                "files": 1,
                "entries": files,
                "combinedSha256": self.module.combined_file_digest(files),
            }
            first.write_text("print('changed')\n")

            result = self.module.validate_plan(
                plan,
                require_ready=True,
                verify_harness_files=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("Harbor harness file SHA-256 mismatch", result.issues)


if __name__ == "__main__":
    unittest.main()
