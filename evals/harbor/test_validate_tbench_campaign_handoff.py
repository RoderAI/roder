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


class ValidateTbenchCampaignHandoffTests(unittest.TestCase):
    def test_rejects_pre_eval_summary_path_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp) / "campaign")
            data = json.loads(manifest.read_text())
            data["preEval"] = {
                "outputDir": str(Path(temp) / "campaign/pre-eval"),
                "summary": str(Path(temp) / "wrong/pre-eval-summary.json"),
            }
            manifest.write_text(json.dumps(data, indent=2) + "\n")

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("preEval summary mismatch", result.stderr)

    def test_rejects_run_script_pre_eval_output_dir_drift(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp) / "campaign")
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = (
                'pre_eval_output_dir="${RODER_HARBOR_PRE_EVAL_OUTPUT_DIR:-'
                '$PREFLIGHT_DIR/pre-eval}"'
            )
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(
                    expected,
                    'pre_eval_output_dir="${RODER_HARBOR_PRE_EVAL_OUTPUT_DIR:-'
                    '$PREFLIGHT_DIR/wrong-pre-eval}"',
                    1,
                )
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn("runScript pre-eval output dir mismatch", result.stderr)


if __name__ == "__main__":
    unittest.main()
