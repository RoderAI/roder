#!/usr/bin/env python3

from __future__ import annotations

import argparse
import importlib.util
import json
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "evals/harbor/write_tbench_launch_plan.py"


def load_module():
    spec = importlib.util.spec_from_file_location("write_tbench_launch_plan", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class WriteTbenchLaunchPlanTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_reused_summary_analysis_target_enables_required_analysis(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            config = temp_path / "tbench.json"
            config.write_text('{"config":true}\n')
            summary = temp_path / "pre-eval-summary.json"
            summary.write_text(
                json.dumps(
                    {
                        "status": "ok",
                        "blockedChecks": [],
                        "options": {
                            "analysisTarget": "evals/reports/harbor/full-analysis.json",
                        },
                        "checks": {
                            "harborConfigs": {
                                "entries": [
                                    {
                                        "path": str(config),
                                        "sha256": "a" * 64,
                                    }
                                ]
                            },
                        },
                    }
                )
                + "\n"
            )

            plan = self.module.build_launch_plan(
                argparse.Namespace(
                    output=temp_path / "launch-plan.json",
                    pre_eval_summary=summary,
                    pre_eval_output_dir=str(temp_path / "pre-eval"),
                    pre_eval_ran_here=False,
                    require_image_preflight=False,
                    require_analysis=False,
                    analysis_target="",
                    skip_preflight=True,
                    pull_preflight=False,
                    replace_job=False,
                    dry_run=True,
                    image_preflight_manifest=None,
                    job_dir=temp_path / "jobs/roder-tbench-full",
                    harbor_config=config,
                    analysis_json=temp_path / "analysis.json",
                    analysis_markdown=temp_path / "analysis.md",
                    max_pre_eval_age_seconds=7200,
                    campaign_summary="",
                )
            )

        self.assertTrue(plan["requireAnalysis"])

    def test_reused_summary_offline_images_enables_offline_preflight(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            config = temp_path / "tbench.json"
            config.write_text('{"config":true}\n')
            summary = temp_path / "pre-eval-summary.json"
            summary.write_text(
                json.dumps(
                    {
                        "status": "ok",
                        "blockedChecks": [],
                        "options": {
                            "preflightImages": True,
                            "offlineImages": True,
                            "pullImages": False,
                        },
                        "checks": {
                            "harborConfigs": {
                                "entries": [
                                    {
                                        "path": str(config),
                                        "sha256": "a" * 64,
                                    }
                                ]
                            },
                            "imagePreflight": {
                                "status": "passed",
                                "offline": True,
                            },
                        },
                    }
                )
                + "\n"
            )

            plan = self.module.build_launch_plan(
                argparse.Namespace(
                    output=temp_path / "launch-plan.json",
                    pre_eval_summary=summary,
                    pre_eval_output_dir=str(temp_path / "pre-eval"),
                    pre_eval_ran_here=False,
                    require_image_preflight=False,
                    require_analysis=False,
                    analysis_target="",
                    skip_preflight=True,
                    pull_preflight=False,
                    replace_job=False,
                    dry_run=True,
                    image_preflight_manifest=None,
                    job_dir=temp_path / "jobs/roder-tbench-full",
                    harbor_config=config,
                    analysis_json=temp_path / "analysis.json",
                    analysis_markdown=temp_path / "analysis.md",
                    max_pre_eval_age_seconds=7200,
                    campaign_summary="",
                )
            )

        self.assertTrue(plan.get("offlinePreflight"))

    def test_route_image_manifest_populates_required_image_preflight(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            config = temp_path / "route.json"
            config.write_text('{"datasets":[{"task_names":["route-task"]}]}\n')
            manifest = temp_path / "route-images.json"
            manifest.write_text(
                json.dumps(
                    {
                        "clean": True,
                        "config": str(config),
                        "summary": {
                            "tasks": 1,
                            "unique_images": 1,
                            "present": 1,
                            "missing": 0,
                            "unresolved": 0,
                            "pull_failed": 0,
                        },
                        "tasks": [
                            {
                                "task_name": "route-task",
                                "image": "terminalbench/route-task:latest",
                                "status": "present",
                                "image_source": "task",
                            }
                        ],
                        "images": [
                            {
                                "image": "terminalbench/route-task:latest",
                                "tasks": ["route-task"],
                            }
                        ],
                    }
                )
                + "\n"
            )
            summary = temp_path / "pre-eval-summary.json"
            summary.write_text(
                json.dumps(
                    {
                        "status": "ok",
                        "blockedChecks": [],
                        "options": {"preflightImages": False},
                        "checks": {
                            "harborConfigs": {
                                "entries": [
                                    {
                                        "path": str(config),
                                        "sha256": sha256(config.read_bytes()).hexdigest(),
                                    }
                                ]
                            },
                        },
                    }
                )
                + "\n"
            )

            plan = self.module.build_launch_plan(
                argparse.Namespace(
                    output=temp_path / "launch-plan.json",
                    pre_eval_summary=summary,
                    pre_eval_output_dir=str(temp_path / "pre-eval"),
                    pre_eval_ran_here=False,
                    require_image_preflight=False,
                    require_analysis=False,
                    analysis_target="",
                    skip_preflight=False,
                    pull_preflight=False,
                    replace_job=False,
                    dry_run=True,
                    image_preflight_manifest=manifest,
                    job_dir=temp_path / "jobs/route",
                    harbor_config=config,
                    analysis_json=temp_path / "analysis.json",
                    analysis_markdown=temp_path / "analysis.md",
                    max_pre_eval_age_seconds=7200,
                    campaign_summary="",
                )
            )

        self.assertTrue(plan["requireImagePreflight"])
        self.assertEqual("route_manifest", plan["imagePreflightSource"])
        self.assertEqual(str(config), plan["imagePreflight"]["config"])
        self.assertEqual(str(manifest), plan["imagePreflight"]["manifest"])
        self.assertEqual(1, plan["imagePreflight"]["tasks"])
        self.assertEqual(1, plan["imagePreflight"]["present"])


if __name__ == "__main__":
    unittest.main()
