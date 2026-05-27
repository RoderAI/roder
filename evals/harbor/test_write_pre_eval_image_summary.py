#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "evals/harbor/write_pre_eval_summary.py"


def load_module():
    spec = importlib.util.spec_from_file_location("write_pre_eval_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class PreEvalImageSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_image_summary_records_blocking_task_details(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = Path(temp) / "manifest.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": False,
                        "config": "evals/harbor/tbench-full-gpt55-medium.json",
                        "summary": {
                            "tasks": 3,
                            "unique_images": 2,
                            "present": 0,
                            "missing": 1,
                            "unresolved": 1,
                            "pull_failed": 1,
                        },
                        "selection_errors": ["terminal-bench@2.0: offline mode needs task_names"],
                        "tasks": [
                            {
                                "task_name": "missing-image-task",
                                "status": "missing",
                                "image": "example/missing:latest",
                                "image_source": "cache:/tmp/task.toml",
                            },
                            {
                                "task_name": "unresolved-image-task",
                                "status": "unresolved",
                                "image": None,
                                "image_source": "unresolved",
                            },
                            {
                                "task_name": "pull-failed-task",
                                "status": "pull_failed",
                                "image": "example/fails:latest",
                                "image_source": "registry-source",
                            },
                        ],
                    }
                )
            )

            summary = self.module.image_preflight_summary(manifest)

            self.assertEqual("failed", summary["status"])
            self.assertEqual(
                "evals/harbor/tbench-full-gpt55-medium.json",
                summary["config"],
            )
            self.assertEqual(
                ["terminal-bench@2.0: offline mode needs task_names"],
                summary["selectionErrors"],
            )
            self.assertEqual(
                [
                    {
                        "taskName": "missing-image-task",
                        "status": "missing",
                        "image": "example/missing:latest",
                        "imageSource": "cache:/tmp/task.toml",
                    },
                    {
                        "taskName": "unresolved-image-task",
                        "status": "unresolved",
                        "image": None,
                        "imageSource": "unresolved",
                    },
                    {
                        "taskName": "pull-failed-task",
                        "status": "pull_failed",
                        "image": "example/fails:latest",
                        "imageSource": "registry-source",
                    },
                ],
                summary["blockedTasks"],
            )


if __name__ == "__main__":
    unittest.main()
