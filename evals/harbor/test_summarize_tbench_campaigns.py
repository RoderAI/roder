#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import json
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "evals/harbor/summarize_tbench_campaigns.py"
GENERATE = ROOT / "evals/harbor/generate_tbench_campaign.py"


def load_module():
    spec = importlib.util.spec_from_file_location("summarize_tbench_campaigns", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def write_manifest(path: Path, *, campaign: str, routes: list[dict[str, object]]) -> Path:
    path.write_text(
        json.dumps(
            {
                "campaign": campaign,
                "routes": routes,
            },
            indent=2,
        )
        + "\n"
    )
    return path


class SummarizeTbenchCampaignsTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_summarizes_unique_tasks_duplicates_and_score_projection(self) -> None:
        manifest_a = {
            "campaign": "alpha-campaign",
            "routes": [
                {"name": "medium", "tasks": ["task-a", "task-b"]},
            ],
        }
        manifest_b = {
            "campaign": "beta-campaign",
            "routes": [
                {"name": "xhigh", "tasks": ["task-b", "task-c"]},
            ],
        }

        report = self.module.summarize_campaign_manifests(
            [
                (Path("alpha-manifest.json"), manifest_a),
                (Path("beta-manifest.json"), manifest_b),
            ]
        )

        self.assertEqual(2, report["summary"]["campaigns"])
        self.assertEqual(2, report["summary"]["routes"])
        self.assertEqual(4, report["summary"]["tasks"])
        self.assertEqual(3, report["summary"]["uniqueTasks"])
        self.assertEqual(1, report["summary"]["duplicateTasks"])
        self.assertEqual(
            [
                {
                    "taskName": "task-b",
                    "owners": [
                        "alpha-campaign/medium",
                        "beta-campaign/xhigh",
                    ],
                }
            ],
            report["duplicates"],
        )
        self.assertEqual(
            53,
            report["scoreProjection"]["projectedPassesIfAllRoutesPass"],
        )

    def test_require_no_overlap_cli_blocks_duplicate_tasks(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            first = write_manifest(
                temp_path / "first.json",
                campaign="first",
                routes=[{"name": "route-a", "tasks": ["shared-task"]}],
            )
            second = write_manifest(
                temp_path / "second.json",
                campaign="second",
                routes=[{"name": "route-b", "tasks": ["shared-task"]}],
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    str(first),
                    str(second),
                    "--require-no-overlap",
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(1, result.returncode)
        self.assertIn("duplicate campaign tasks", result.stderr)
        self.assertIn("shared-task", result.stderr)

    def test_real_campaigns_project_combined_68_without_overlap(self) -> None:
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

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    str(validated_dir / "validated-conversions-manifest.json"),
                    str(historical_dir / "historical-wins-manifest.json"),
                    "--require-no-overlap",
                    "--expect-unique-tasks",
                    "18",
                    "--expect-projected-passes",
                    "68",
                    "--expect-task",
                    "password-recovery",
                    "--expect-task",
                    "qemu-startup",
                    "--expect-task",
                    "vulnerable-secret",
                    "--expect-owner",
                    "password-recovery=historical-wins/policy-framed",
                    "--expect-owner",
                    "qemu-startup=historical-wins/environment-targeted",
                    "--expect-owner",
                    "vulnerable-secret=historical-wins/policy-framed",
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("Projected passes if all routes pass: `68/89`", result.stdout)
        self.assertIn("Duplicate tasks: `0`", result.stdout)

    def test_validated_plus_historical_preset_checks_full_handoff_contract(self) -> None:
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

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    str(validated_dir / "validated-conversions-manifest.json"),
                    str(historical_dir / "historical-wins-manifest.json"),
                    "--preset",
                    "validated-plus-historical",
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("Projected passes if all routes pass: `68/89`", result.stdout)
        self.assertIn("Duplicate tasks: `0`", result.stdout)

    def test_validated_plus_historical_preset_blocks_wrong_owner(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = write_manifest(
                temp_path / "manifest.json",
                campaign="historical-wins",
                routes=[
                    {
                        "name": "policy-framed",
                        "tasks": [
                            "password-recovery",
                            "qemu-startup",
                            "vulnerable-secret",
                        ],
                    }
                ],
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    str(manifest),
                    "--preset",
                    "validated-plus-historical",
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(1, result.returncode)
        self.assertIn(
            "qemu-startup expected owner historical-wins/environment-targeted, got historical-wins/policy-framed",
            result.stderr,
        )

    def test_expected_projection_cli_blocks_stale_campaign_math(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = write_manifest(
                temp_path / "manifest.json",
                campaign="small",
                routes=[{"name": "route-a", "tasks": ["one-task"]}],
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    str(manifest),
                    "--expect-unique-tasks",
                    "18",
                    "--expect-projected-passes",
                    "68",
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(1, result.returncode)
        self.assertIn("uniqueTasks expected 18, got 1", result.stderr)
        self.assertIn("projectedPassesIfAllRoutesPass expected 68, got 51", result.stderr)

    def test_expected_task_cli_blocks_missing_required_task(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = write_manifest(
                temp_path / "manifest.json",
                campaign="small",
                routes=[{"name": "route-a", "tasks": ["one-task"]}],
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    str(manifest),
                    "--expect-task",
                    "password-recovery",
                    "--expect-task",
                    "qemu-startup",
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(1, result.returncode)
        self.assertIn("missing expected tasks: password-recovery, qemu-startup", result.stderr)

    def test_expected_owner_cli_blocks_wrong_route_assignment(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = write_manifest(
                temp_path / "manifest.json",
                campaign="historical-wins",
                routes=[{"name": "policy-framed", "tasks": ["qemu-startup"]}],
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    str(manifest),
                    "--expect-owner",
                    "qemu-startup=historical-wins/environment-targeted",
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(1, result.returncode)
        self.assertIn(
            "qemu-startup expected owner historical-wins/environment-targeted, got historical-wins/policy-framed",
            result.stderr,
        )

    def test_expected_owner_cli_rejects_malformed_expectation(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            manifest = write_manifest(
                temp_path / "manifest.json",
                campaign="historical-wins",
                routes=[{"name": "policy-framed", "tasks": ["password-recovery"]}],
            )

            result = subprocess.run(
                [
                    "python3",
                    str(MODULE_PATH),
                    str(manifest),
                    "--expect-owner",
                    "password-recovery",
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(2, result.returncode)
        self.assertIn(
            "--expect-owner must use TASK=CAMPAIGN/ROUTE: password-recovery",
            result.stderr,
        )
        self.assertNotIn("Traceback", result.stderr)


if __name__ == "__main__":
    unittest.main()
