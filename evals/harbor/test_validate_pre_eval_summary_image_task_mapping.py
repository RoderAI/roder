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


class ValidatePreEvalSummaryImageTaskMappingTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_verify_image_manifest_blocks_image_task_mapping_mismatch(self) -> None:
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
                            {"image": "example/a:latest", "tasks": ["task-b"]},
                            {"image": "example/b:latest", "tasks": ["task-a"]},
                            {"image": "example/c:latest", "tasks": ["task-c"]},
                            {"image": "example/d:latest", "tasks": ["task-d"]},
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
            "imagePreflight manifest image task mapping mismatch",
            result.issues,
        )


if __name__ == "__main__":
    unittest.main()
