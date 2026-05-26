#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

try:
    from .tbench_campaign_test_helpers import generate_campaign, validate_campaign
except ImportError:
    from tbench_campaign_test_helpers import generate_campaign, validate_campaign


class ValidateTbenchCampaignRunScriptCampaignSummaryTests(unittest.TestCase):
    def test_generated_run_script_threads_campaign_summary_handoff(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp) / "campaign")
            data = json.loads(manifest.read_text())
            script = Path(data["runScript"]).read_text()

        self.assertIn(
            'pre_eval_campaign_summary="${RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY:-}"',
            script,
        )
        self.assertIn(
            'pre_eval_args+=(--campaign-summary "$pre_eval_campaign_summary")',
            script,
        )
        self.assertIn(
            'summary_validation_args+=(--require-campaign-summary)',
            script,
        )
        self.assertIn(
            'summary_validation_args+=(--campaign-summary "$pre_eval_campaign_summary")',
            script,
        )
        self.assertIn(
            'launch_plan_campaign_args=(--campaign-summary "$pre_eval_campaign_summary")',
            script,
        )
        self.assertIn(
            'launch_plan_validation_campaign_args=(--require-campaign-summary)',
            script,
        )

    def test_rejects_generated_run_script_missing_campaign_summary_guard(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            manifest = generate_campaign(Path(temp) / "campaign")
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            run_script.write_text(
                run_script.read_text().replace(
                    'pre_eval_campaign_summary="${RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY:-}"',
                    'pre_eval_campaign_summary=""',
                )
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript pre-eval campaign summary guard mismatch",
            result.stderr,
        )

    def test_dry_run_threads_campaign_summary_to_python_gates(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = generate_campaign(temp_path / "campaign")
            data = json.loads(manifest.read_text())
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text(
                "#!/usr/bin/env bash\n"
                "printf '%s\\n' \"$*\" >> \"$PYTHON_STUB_LOG\"\n"
                "exit 0\n"
            )
            fake_python.chmod(0o755)
            stub_log = temp_path / "python-calls.log"
            summary = temp_path / "pre-eval-summary.json"
            summary.write_text("{}\n")
            campaign_summary = temp_path / "combined-summary.json"
            campaign_summary.write_text("{}\n")
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env.get('PATH', '')}",
                    "PYTHON_STUB_LOG": str(stub_log),
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY": str(campaign_summary),
                }
            )

            result = subprocess.run(
                ["bash", data["runScript"]],
                cwd=Path(__file__).resolve().parents[2],
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            calls = stub_log.read_text()

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn(
            f"validate_pre_eval_summary.py {summary} --require-prebuilt",
            calls,
        )
        self.assertIn("--require-campaign-summary", calls)
        self.assertIn(f"--campaign-summary {campaign_summary}", calls)
        self.assertIn(
            f"write_tbench_launch_plan.py --output {data['routes'][0]['launchPlan']}",
            calls,
        )
        self.assertIn(f"--campaign-summary {campaign_summary}", calls)
        self.assertIn("validate_tbench_launch_plan.py", calls)
        self.assertIn("--require-campaign-summary", calls)


if __name__ == "__main__":
    unittest.main()
