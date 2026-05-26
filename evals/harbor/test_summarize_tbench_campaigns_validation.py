#!/usr/bin/env python3

from __future__ import annotations

import json
import subprocess
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
GENERATE = ROOT / "evals/harbor/generate_tbench_campaign.py"
SCRIPT = ROOT / "evals/harbor/summarize_tbench_campaigns.py"


class SummarizeTbenchCampaignsValidationTests(unittest.TestCase):
    def test_json_output_records_passed_preset_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            validated_dir = temp_path / "validated-conversions"
            historical_dir = temp_path / "historical-wins"
            for campaign, output_dir in (
                ("validated-conversions", validated_dir),
                ("historical-wins", historical_dir),
            ):
                result = subprocess.run(
                    [
                        "python3",
                        str(GENERATE),
                        "--campaign",
                        campaign,
                        "--output-dir",
                        str(output_dir),
                    ],
                    cwd=ROOT,
                    text=True,
                    stdout=subprocess.PIPE,
                    stderr=subprocess.PIPE,
                    check=False,
                )
                self.assertEqual(0, result.returncode, result.stderr)
            report_path = temp_path / "combined-summary.json"

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    str(validated_dir / "validated-conversions-manifest.json"),
                    str(historical_dir / "historical-wins-manifest.json"),
                    "--preset",
                    "validated-plus-historical",
                    "--json",
                    str(report_path),
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            report = json.loads(report_path.read_text())
            validated_manifest = validated_dir / "validated-conversions-manifest.json"
            historical_manifest = historical_dir / "historical-wins-manifest.json"
            expected_hashes = {
                str(validated_manifest): sha256(validated_manifest.read_bytes()).hexdigest(),
                str(historical_manifest): sha256(historical_manifest.read_bytes()).hexdigest(),
            }
            manifest_hashes = {
                entry["manifest"]: entry["manifestSha256"]
                for entry in report["campaigns"]
            }

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual("ok", report["validation"]["status"])
        self.assertEqual("validated-plus-historical", report["validation"]["preset"])
        self.assertEqual([], report["validation"]["issues"])
        self.assertTrue(report["validation"]["requireNoOverlap"])
        self.assertEqual(18, report["validation"]["expectUniqueTasks"])
        self.assertEqual(68, report["validation"]["expectProjectedPasses"])
        self.assertIn("historical-wins/policy-framed", report["validation"]["expectRoutes"])
        self.assertEqual(
            expected_hashes[str(validated_manifest)],
            manifest_hashes[str(validated_manifest)],
        )
        self.assertEqual(
            expected_hashes[str(historical_manifest)],
            manifest_hashes[str(historical_manifest)],
        )

    def test_json_output_records_blocked_preset_validation_before_exit(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest_path = temp_path / "historical-only.json"
            manifest_path.write_text(
                json.dumps(
                    {
                        "campaign": "historical-wins",
                        "routes": [
                            {
                                "name": "policy-framed",
                                "tasks": [
                                    "password-recovery",
                                    "qemu-startup",
                                    "vulnerable-secret",
                                ],
                            }
                        ],
                    },
                    indent=2,
                )
                + "\n"
            )
            report_path = temp_path / "blocked-summary.json"

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    str(manifest_path),
                    "--preset",
                    "validated-plus-historical",
                    "--json",
                    str(report_path),
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            report = json.loads(report_path.read_text())

        self.assertEqual(1, result.returncode)
        self.assertEqual("blocked", report["validation"]["status"])
        self.assertEqual("validated-plus-historical", report["validation"]["preset"])
        self.assertIn(
            "missing expected campaigns: validated-conversions",
            report["validation"]["issues"],
        )
        self.assertIn("expectation mismatch", result.stderr)


if __name__ == "__main__":
    unittest.main()
