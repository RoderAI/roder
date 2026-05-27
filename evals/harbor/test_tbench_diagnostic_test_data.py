#!/usr/bin/env python3

from __future__ import annotations

import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"


class TBenchDiagnosticTestDataTests(unittest.TestCase):
    def test_summary_helpers_delegate_diagnostic_fixture_data(self) -> None:
        helper_paths = [
            HARBOR_DIR / "pre_eval_summary_test_helpers.py",
            HARBOR_DIR / "validate_pre_eval_summary_test_helpers.py",
            HARBOR_DIR / "launch_plan_test_helpers.py",
            HARBOR_DIR / "run_roder_tbench_full_test_helpers.py",
            HARBOR_DIR / "test_write_pre_eval_options_summary.py",
            HARBOR_DIR / "test_write_pre_eval_config_summary.py",
            HARBOR_DIR / "test_write_pre_eval_tbench_summary.py",
            HARBOR_DIR / "test_validate_pre_eval_tbench_diagnostics.py",
        ]

        offenders = []
        for path in helper_paths:
            source = path.read_text()
            if "tbench_diagnostic_test_data" not in source:
                offenders.append(path.name)
            self.assertNotIn("EXPECTED_TBENCH_FIXTURES =", source, path.name)
            self.assertNotIn("EXPECTED_COMMAND_CHECK_FIXTURES =", source, path.name)

        self.assertEqual([], offenders)


if __name__ == "__main__":
    unittest.main()
