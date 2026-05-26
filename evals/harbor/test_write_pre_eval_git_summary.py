#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "evals/harbor/pre_eval_git_summary.py"


def load_module():
    spec = importlib.util.spec_from_file_location("pre_eval_git_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class PreEvalGitSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_git_summary_records_bounded_dirty_paths(self) -> None:
        status = "\n".join(
            [
                "M crates/roder-core/src/fake_provider.rs",
                " M evals/harbor/run-roder-pre-eval-diagnostics.sh",
                "?? evals/harbor/test_write_pre_eval_git_summary.py",
                "R  old-name.txt -> new-name.txt",
            ]
        )

        summary = self.module.git_summary_from_status(
            head="abc123",
            status=status,
            dirty_path_limit=2,
        )

        self.assertEqual("abc123", summary["head"])
        self.assertEqual(4, summary["dirtyPathCount"])
        self.assertEqual(2, summary["dirtyPathLimit"])
        self.assertTrue(summary["dirtyPathTruncated"])
        self.assertEqual(
            [
                "crates/roder-core/src/fake_provider.rs",
                "evals/harbor/run-roder-pre-eval-diagnostics.sh",
            ],
            summary["dirtyPaths"],
        )


if __name__ == "__main__":
    unittest.main()
