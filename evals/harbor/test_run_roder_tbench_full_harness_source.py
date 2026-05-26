#!/usr/bin/env python3

from __future__ import annotations

import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

import pre_eval_harness_summary  # noqa: E402
import run_roder_tbench_full_test_helpers as full_gate_helpers  # noqa: E402


class RunRoderTbenchFullHarnessSourceTests(unittest.TestCase):
    def test_full_run_gate_uses_pre_eval_harness_attestation_files(self) -> None:
        expected = [
            path.as_posix()
            for path in pre_eval_harness_summary.DEFAULT_HARNESS_FILES
        ]
        actual = [path.as_posix() for path in full_gate_helpers.HARNESS_FILES]

        self.assertEqual(expected, actual)

    def test_full_run_diagnostics_always_attest_launched_config(self) -> None:
        script = full_gate_helpers.SCRIPT.read_text()

        self.assertIn('pre_eval_args+=(--config "$harbor_config")', script)

    def test_full_run_summary_validation_always_requires_launched_config(self) -> None:
        script = full_gate_helpers.SCRIPT.read_text()

        self.assertIn(
            'summary_validation_args+=(--require-config "$harbor_config")',
            script,
        )


if __name__ == "__main__":
    unittest.main()
