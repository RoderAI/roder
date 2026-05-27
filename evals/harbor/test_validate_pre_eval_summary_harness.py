#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from pre_eval_harness_summary import DEFAULT_HARNESS_FILES  # noqa: E402
from pre_eval_live_checks import combined_file_digest, file_sha256  # noqa: E402
from validate_pre_eval_summary_test_helpers import clean_summary  # noqa: E402

MODULE_PATH = ROOT / "evals/harbor/validate_pre_eval_summary.py"


def load_module():
    spec = importlib.util.spec_from_file_location("validate_pre_eval_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def harness_entry(path: Path) -> dict:
    return {
        "path": str(path),
        "sha256": file_sha256(path),
        "sizeBytes": path.stat().st_size,
    }


class ValidatePreEvalSummaryHarnessTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_verify_harbor_harness_files_blocks_duplicate_entries(self) -> None:
        entries = [harness_entry(path) for path in DEFAULT_HARNESS_FILES]
        entries.append(dict(entries[0]))
        summary = clean_summary()
        summary["checks"]["harborHarness"] = {
            "status": "passed",
            "files": len(entries),
            "issues": [],
            "combinedSha256": combined_file_digest(entries),
            "entries": entries,
        }

        result = self.module.validate_summary(summary, verify_harness_files=True)

        self.assertFalse(result.ok)
        self.assertIn(
            f"harborHarness duplicate file entry: {entries[0]['path']}",
            result.issues,
        )

    def test_harbor_harness_blocks_files_count_mismatch(self) -> None:
        entries = [harness_entry(path) for path in DEFAULT_HARNESS_FILES]
        summary = clean_summary()
        summary["checks"]["harborHarness"] = {
            "status": "passed",
            "files": len(entries) + 1,
            "issues": [],
            "combinedSha256": combined_file_digest(entries),
            "entries": entries,
        }

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            f"harborHarness files count mismatch: expected {len(entries)}, got {len(entries) + 1}",
            result.issues,
        )


if __name__ == "__main__":
    unittest.main()
