#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
MODULE_PATH = ROOT / "evals/harbor/validate_tbench_launch_plan.py"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from launch_plan_test_helpers import ready_plan  # noqa: E402


def load_module():
    spec = importlib.util.spec_from_file_location(
        "validate_tbench_launch_plan",
        MODULE_PATH,
    )
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class ValidateTbenchLaunchPlanImageManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_verify_image_manifest_blocks_missing_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            plan = ready_plan()
            plan["imagePreflight"]["manifest"] = str(
                Path(temp) / "missing-manifest.json"
            )

            result = self.module.validate_plan(
                plan,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertTrue(
            any(
                issue.startswith("imagePreflight manifest cannot be read:")
                for issue in result.issues
            )
        )

    def test_verify_image_manifest_blocks_mismatched_counts(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 88,
                            "unique_images": 88,
                            "present": 88,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                    }
                )
                + "\n"
            )
            plan = ready_plan()
            plan["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_plan(
                plan,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight manifest tasks mismatch", result.issues)
        self.assertIn("imagePreflight manifest uniqueImages mismatch", result.issues)
        self.assertIn("imagePreflight manifest present mismatch", result.issues)

    def test_verify_image_manifest_blocks_selection_error_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 89,
                            "unique_images": 89,
                            "present": 89,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "selection_errors": ["terminal-bench@2.0: unresolved"],
                    }
                )
                + "\n"
            )
            plan = ready_plan()
            plan["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_plan(
                plan,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "imagePreflight manifest selectionErrors mismatch",
            result.issues,
        )

    def test_verify_image_manifest_blocks_blocked_task_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 89,
                            "unique_images": 89,
                            "present": 89,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {
                                "task_name": "missing-image-task",
                                "status": "missing",
                                "image": "example/missing:latest",
                            }
                        ],
                    }
                )
                + "\n"
            )
            plan = ready_plan()
            plan["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_plan(
                plan,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "imagePreflight manifest blockedTasks mismatch",
            result.issues,
        )

    def test_verify_image_manifest_blocks_task_status_count_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 89,
                            "unique_images": 89,
                            "present": 89,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            *[
                                {"task_name": f"task-{index}", "status": "present"}
                                for index in range(88)
                            ],
                            {"task_name": "task-88", "status": "unknown"},
                        ],
                        "images": [
                            {"image": f"example/{index}:latest"}
                            for index in range(89)
                        ],
                    }
                )
                + "\n"
            )
            plan = ready_plan()
            plan["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_plan(
                plan,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "imagePreflight manifest present status count mismatch",
            result.issues,
        )

    def test_verify_image_manifest_blocks_duplicate_image_rows(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 89,
                            "unique_images": 89,
                            "present": 89,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {"task_name": f"task-{index}", "status": "present"}
                            for index in range(89)
                        ],
                        "images": [
                            *[
                                {"image": f"example/{index}:latest"}
                                for index in range(88)
                            ],
                            {"image": "example/87:latest"},
                        ],
                    }
                )
                + "\n"
            )
            plan = ready_plan()
            plan["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_plan(
                plan,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight manifest image rows are not unique", result.issues)

    def test_verify_image_manifest_blocks_task_image_set_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 89,
                            "unique_images": 89,
                            "present": 89,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {
                                "task_name": f"task-{index}",
                                "status": "present",
                                "image": f"example/{index}:latest",
                            }
                            for index in range(89)
                        ],
                        "images": [
                            *[
                                {"image": f"example/{index}:latest"}
                                for index in range(88)
                            ],
                            {"image": "example/other:latest"},
                        ],
                    }
                )
                + "\n"
            )
            plan = ready_plan()
            plan["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_plan(
                plan,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "imagePreflight manifest task images do not match image rows",
            result.issues,
        )


if __name__ == "__main__":
    unittest.main()
