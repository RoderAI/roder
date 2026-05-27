#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "evals/harbor/preflight_tbench_images.py"


def load_module():
    spec = importlib.util.spec_from_file_location("preflight_tbench_images", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


class PreflightTbenchImagesTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_offline_selection_does_not_infer_scope_from_existing_jobs(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            trial_dir = temp_path / "jobs/partial-job/example-task__attempt-1"
            trial_dir.mkdir(parents=True)
            (trial_dir / "result.json").write_text(
                json.dumps({"task_name": "example-task"}) + "\n"
            )
            config = {
                "job_name": "partial-job",
                "jobs_dir": str(temp_path / "jobs"),
                "datasets": [
                    {
                        "name": "terminal-bench",
                        "version": "2.0",
                    }
                ],
            }

            tasks, unresolved = self.module.selected_tasks(config, allow_network=False)

        self.assertEqual([], tasks)
        self.assertEqual(
            [
                "terminal-bench@2.0: offline mode needs explicit task_names "
                "for image preflight"
            ],
            unresolved,
        )


if __name__ == "__main__":
    unittest.main()
