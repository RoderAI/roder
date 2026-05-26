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


class ValidateTbenchCampaignRunScriptGuardTests(unittest.TestCase):
    def test_rejects_generated_run_script_with_live_gate_hidden_in_comment(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = '${RODER_HARBOR_LIVE_TBENCH:-}" != "1"'
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(
                    expected,
                    '${RODER_HARBOR_UNSAFE_LIVE_TBENCH:-}" != "1" '
                    '# RODER_HARBOR_LIVE_TBENCH',
                    1,
                )
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("runScript live-run guard mismatch", result.stderr)

    def test_rejects_generated_run_script_with_dry_run_guard_hidden_in_comment(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = 'dry_run="${RODER_HARBOR_DRY_RUN:-0}"'
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(
                    expected,
                    'dry_run="${RODER_HARBOR_UNSAFE_DRY_RUN:-0}" # RODER_HARBOR_DRY_RUN',
                    1,
                )
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("runScript dry-run guard mismatch", result.stderr)

    def test_rejects_generated_run_script_with_pre_eval_summary_hidden_in_comment(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = 'pre_eval_summary="${RODER_HARBOR_PRE_EVAL_SUMMARY:-}"'
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(
                    expected,
                    'pre_eval_summary="${RODER_HARBOR_UNSAFE_PRE_EVAL_SUMMARY:-}" '
                    '# RODER_HARBOR_PRE_EVAL_SUMMARY',
                    1,
                )
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("runScript pre-eval summary guard mismatch", result.stderr)

    def test_rejects_generated_run_script_with_replace_guard_hidden_in_comment(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = 'if [[ "${RODER_HARBOR_REPLACE_JOB:-}" != "1" ]]; then'
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(
                    expected,
                    'if [[ "${RODER_HARBOR_UNSAFE_REPLACE_JOB:-}" != "1" ]]; then '
                    '# RODER_HARBOR_REPLACE_JOB',
                    1,
                )
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("runScript replace-job guard mismatch", result.stderr)

    def test_rejects_generated_run_script_that_skips_pre_eval_tests(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = (
                'pre_eval_args=(--require-prebuilt --require-auth '
                '--output-dir "$pre_eval_output_dir")'
            )
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(
                    expected,
                    'pre_eval_args=(--require-prebuilt --require-auth --skip-tests '
                    '--output-dir "$pre_eval_output_dir")',
                    1,
                )
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("runScript pre-eval tests cannot be skipped", result.stderr)


if __name__ == "__main__":
    unittest.main()
