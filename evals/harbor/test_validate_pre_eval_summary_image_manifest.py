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


class ValidatePreEvalSummaryImageManifestTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_verify_image_manifest_blocks_missing_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(
                Path(temp) / "missing-manifest.json"
            )

            result = self.module.validate_summary(
                summary,
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
                            "tasks": 3,
                            "unique_images": 3,
                            "present": 3,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                    }
                )
                + "\n"
            )
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_summary(
                summary,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight manifest tasks mismatch", result.issues)
        self.assertIn("imagePreflight manifest uniqueImages mismatch", result.issues)
        self.assertIn("imagePreflight manifest present mismatch", result.issues)

    def test_verify_image_manifest_blocks_partial_full_config_scope(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 4,
                            "unique_images": 4,
                            "present": 4,
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
                            for index in range(4)
                        ],
                        "images": [
                            {
                                "image": f"example/{index}:latest",
                                "tasks": [f"task-{index}"],
                            }
                            for index in range(4)
                        ],
                    }
                )
                + "\n"
            )
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_summary(
                summary,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "imagePreflight manifest task count does not match config: expected 89, got 4",
            result.issues,
        )

    def test_verify_image_manifest_blocks_selection_error_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 4,
                            "unique_images": 4,
                            "present": 4,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "selection_errors": ["terminal-bench@2.0: unresolved"],
                    }
                )
                + "\n"
            )
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_summary(
                summary,
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
                            "tasks": 4,
                            "unique_images": 4,
                            "present": 4,
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
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_summary(
                summary,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "imagePreflight manifest blockedTasks mismatch",
            result.issues,
        )

    def test_verify_image_manifest_blocks_task_row_count_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 4,
                            "unique_images": 4,
                            "present": 4,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {"task_name": "task-a", "status": "present"},
                            {"task_name": "task-b", "status": "present"},
                            {"task_name": "task-c", "status": "present"},
                        ],
                        "images": [
                            {"image": "example/a:latest"},
                            {"image": "example/b:latest"},
                            {"image": "example/c:latest"},
                            {"image": "example/d:latest"},
                        ],
                    }
                )
                + "\n"
            )
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_summary(
                summary,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight manifest task rows mismatch", result.issues)

    def test_verify_image_manifest_blocks_image_row_count_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 4,
                            "unique_images": 4,
                            "present": 4,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {"task_name": "task-a", "status": "present"},
                            {"task_name": "task-b", "status": "present"},
                            {"task_name": "task-c", "status": "present"},
                            {"task_name": "task-d", "status": "present"},
                        ],
                        "images": [
                            {"image": "example/a:latest"},
                            {"image": "example/b:latest"},
                            {"image": "example/c:latest"},
                        ],
                    }
                )
                + "\n"
            )
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_summary(
                summary,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight manifest image rows mismatch", result.issues)

    def test_verify_image_manifest_blocks_task_status_count_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 4,
                            "unique_images": 4,
                            "present": 4,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {"task_name": "task-a", "status": "present"},
                            {"task_name": "task-b", "status": "present"},
                            {"task_name": "task-c", "status": "present"},
                            {"task_name": "task-d", "status": "unknown"},
                        ],
                        "images": [
                            {"image": "example/a:latest"},
                            {"image": "example/b:latest"},
                            {"image": "example/c:latest"},
                            {"image": "example/d:latest"},
                        ],
                    }
                )
                + "\n"
            )
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_summary(
                summary,
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
                            "tasks": 4,
                            "unique_images": 4,
                            "present": 4,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {"task_name": "task-a", "status": "present"},
                            {"task_name": "task-b", "status": "present"},
                            {"task_name": "task-c", "status": "present"},
                            {"task_name": "task-d", "status": "present"},
                        ],
                        "images": [
                            {"image": "example/a:latest"},
                            {"image": "example/b:latest"},
                            {"image": "example/c:latest"},
                            {"image": "example/c:latest"},
                        ],
                    }
                )
                + "\n"
            )
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_summary(
                summary,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight manifest image rows are not unique", result.issues)

    def test_verify_image_manifest_blocks_duplicate_task_rows(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 4,
                            "unique_images": 4,
                            "present": 4,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {"task_name": "task-a", "status": "present"},
                            {"task_name": "task-b", "status": "present"},
                            {"task_name": "task-c", "status": "present"},
                            {"task_name": "task-c", "status": "present"},
                        ],
                        "images": [
                            {"image": "example/a:latest"},
                            {"image": "example/b:latest"},
                            {"image": "example/c:latest"},
                            {"image": "example/d:latest"},
                        ],
                    }
                )
                + "\n"
            )
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_summary(
                summary,
                require_image_preflight=True,
                verify_image_manifest=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight manifest task rows are not unique", result.issues)

    def test_verify_image_manifest_blocks_task_image_set_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 4,
                            "unique_images": 4,
                            "present": 4,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {
                                "task_name": "task-a",
                                "status": "present",
                                "image": "example/a:latest",
                            },
                            {
                                "task_name": "task-b",
                                "status": "present",
                                "image": "example/b:latest",
                            },
                            {
                                "task_name": "task-c",
                                "status": "present",
                                "image": "example/c:latest",
                            },
                            {
                                "task_name": "task-d",
                                "status": "present",
                                "image": "example/d:latest",
                            },
                        ],
                        "images": [
                            {"image": "example/a:latest"},
                            {"image": "example/b:latest"},
                            {"image": "example/c:latest"},
                            {"image": "example/e:latest"},
                        ],
                    }
                )
                + "\n"
            )
            summary = clean_summary()
            summary["checks"]["imagePreflight"]["manifest"] = str(manifest)

            result = self.module.validate_summary(
                summary,
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
