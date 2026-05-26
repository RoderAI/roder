#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import hashlib
import sys
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


class ValidatePreEvalSummaryConfigTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_harbor_configs_blocks_configs_count_mismatch(self) -> None:
        summary = clean_summary()
        entries = [
            {
                "path": "evals/harbor/tbench-full-gpt55-medium.json",
                "sha256": "a" * 64,
            }
        ]
        summary["checks"]["harborConfigs"] = {
            "status": "passed",
            "configs": len(entries) + 1,
            "issues": [],
            "entries": entries,
        }

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            f"harborConfigs configs count mismatch: expected {len(entries)}, got {len(entries) + 1}",
            result.issues,
        )

    def test_verify_harbor_configs_blocks_duplicate_entries(self) -> None:
        config_paths = [
            Path("evals/harbor/tbench-full-gpt55-medium.json"),
            Path("evals/harbor/tbench-smoke.json"),
        ]
        entries = [
            {
                "path": str(path),
                "sha256": hashlib.sha256((ROOT / path).read_bytes()).hexdigest(),
            }
            for path in config_paths
        ]
        entries.append(dict(entries[0]))
        summary = clean_summary()
        summary["checks"]["harborConfigs"] = {
            "status": "passed",
            "configs": len(entries),
            "issues": [],
            "entries": entries,
        }

        result = self.module.validate_summary(summary, verify_harbor_configs=True)

        self.assertFalse(result.ok)
        self.assertIn(
            f"harborConfigs duplicate file entry: {entries[0]['path']}",
            result.issues,
        )


if __name__ == "__main__":
    unittest.main()
