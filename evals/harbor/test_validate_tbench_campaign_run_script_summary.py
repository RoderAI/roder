#!/usr/bin/env python3

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

try:
    from .tbench_campaign_test_helpers import generate_campaign, validate_campaign
except ImportError:
    from tbench_campaign_test_helpers import generate_campaign, validate_campaign


class ValidateTbenchCampaignRunScriptSummaryTests(unittest.TestCase):
    def test_rejects_generated_run_script_that_does_not_validate_summary_args(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = (
                'python3 evals/harbor/validate_pre_eval_summary.py '
                '"${summary_validation_args[@]}"'
            )
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(
                    expected,
                    "python3 evals/harbor/validate_pre_eval_summary.py $pre_eval_summary",
                )
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript summary validation invocation mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_that_drops_summary_harness_verification(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = "  --verify-harness-files"
            self.assertIn(expected, script)
            run_script.write_text(script.replace(expected + "\n", "", 1))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript summary validation args missing: --verify-harness-files",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_summary_flag_hidden_in_echo(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = "  --verify-harness-files"
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected + "\n", "", 1)
                + "\necho --verify-harness-files\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript summary validation args missing: --verify-harness-files",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_summary_flag_hidden_in_comment(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = "  --verify-harness-files"
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected + "\n", "  # --verify-harness-files\n", 1)
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript summary validation args missing: --verify-harness-files",
            result.stderr,
        )


if __name__ == "__main__":
    unittest.main()
