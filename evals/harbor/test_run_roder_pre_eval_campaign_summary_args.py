#!/usr/bin/env python3

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "evals/harbor/run-roder-pre-eval-diagnostics.sh"


class PreEvalDiagnosticsCampaignSummaryArgsTests(unittest.TestCase):
    def test_campaign_summary_is_recorded_in_pre_eval_summary_args(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text(
                "#!/usr/bin/env bash\n"
                "printf '%s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_python.chmod(0o755)
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'cargo %s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_cargo.chmod(0o755)
            campaign_summary = temp_path / "combined-summary.json"
            campaign_summary.write_text("{}\n")
            call_log = temp_path / "calls.log"
            env = {
                **dict(PATH=f"{fake_bin}:{os.environ['PATH']}"),
                "CALL_LOG": str(call_log),
                "HOME": str(temp_path),
            }

            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--campaign-summary",
                    str(campaign_summary),
                    "--skip-tests",
                    "--output-dir",
                    str(temp_path / "diagnostics"),
                ],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            calls = call_log.read_text() if call_log.exists() else ""

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn(
            f"write_pre_eval_summary.py --summary {temp_path / 'diagnostics/pre-eval-summary.json'}",
            calls,
        )
        self.assertIn(f"--campaign-summary {campaign_summary}", calls)


if __name__ == "__main__":
    unittest.main()
