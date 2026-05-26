#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
MODULE_PATH = ROOT / "evals/harbor/pre_eval_harness_summary.py"


def load_module():
    spec = importlib.util.spec_from_file_location("pre_eval_harness_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class PreEvalHarnessSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_harness_summary_changes_when_adapter_file_changes(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            first = root / "roder_harbor_agent.py"
            second = root / "roder_exec_shell.py"
            first.write_text("print('first')\n")
            second.write_text("print('second')\n")

            before = self.module.harness_summary((first, second))
            first.write_text("print('first changed')\n")
            after = self.module.harness_summary((first, second))

        self.assertEqual("passed", before["status"])
        self.assertEqual(2, before["files"])
        self.assertNotEqual(before["combinedSha256"], after["combinedSha256"])

    def test_default_harness_files_include_launch_plan_dependency_validator(self) -> None:
        paths = {path.as_posix() for path in self.module.DEFAULT_HARNESS_FILES}

        self.assertIn(
            "evals/harbor/launch_plan_dependency_validation.py",
            paths,
        )

    def test_default_harness_files_include_full_run_scripts(self) -> None:
        paths = {path.as_posix() for path in self.module.DEFAULT_HARNESS_FILES}

        self.assertIn("evals/harbor/build-prebuilt-roder.sh", paths)
        self.assertIn("evals/harbor/preflight_tbench_images.py", paths)
        self.assertIn("evals/harbor/analyze_tbench_run.py", paths)

    def test_default_harness_files_include_smoke_wrapper(self) -> None:
        paths = {path.as_posix() for path in self.module.DEFAULT_HARNESS_FILES}

        self.assertIn("evals/harbor/run-roder-tbench-smoke.sh", paths)

    def test_default_harness_files_include_post_run_workflow_tools(self) -> None:
        paths = {path.as_posix() for path in self.module.DEFAULT_HARNESS_FILES}

        self.assertIn("evals/harbor/compare_tbench_runs.py", paths)
        self.assertIn("evals/harbor/rerun_tbench_subset.py", paths)
        self.assertIn("evals/harbor/suggest_tbench_campaign.py", paths)
        self.assertIn("evals/harbor/summarize_tbench_campaigns.py", paths)
        self.assertIn("evals/harbor/generate_tbench_campaign.py", paths)
        self.assertIn("evals/harbor/validate_tbench_campaign.py", paths)
        self.assertIn("evals/harbor/tbench_campaign_handoff.py", paths)
        self.assertIn("evals/harbor/tbench_campaign_score_projection.py", paths)

    def test_default_harness_files_include_split_campaign_helpers(self) -> None:
        paths = {path.as_posix() for path in self.module.DEFAULT_HARNESS_FILES}

        self.assertIn("evals/harbor/tbench_campaign_run_script.py", paths)
        self.assertIn("evals/harbor/tbench_campaign_script_commands.py", paths)

    def test_default_harness_files_include_split_agent_config_helpers(self) -> None:
        paths = {path.as_posix() for path in self.module.DEFAULT_HARNESS_FILES}

        self.assertIn("evals/harbor/roder_harbor_agent_config.py", paths)

    def test_default_harness_files_include_shared_validation_helpers(self) -> None:
        paths = {path.as_posix() for path in self.module.DEFAULT_HARNESS_FILES}

        self.assertIn("evals/harbor/pre_eval_image_preflight_validation.py", paths)
        self.assertIn("evals/harbor/pre_eval_run_summary.py", paths)
        self.assertIn("evals/harbor/pre_eval_summary_tbench_validation.py", paths)
        self.assertIn("evals/harbor/tbench_analysis_constants.py", paths)
        self.assertIn("evals/harbor/tbench_campaign_image_preflight.py", paths)

    def test_default_harness_files_include_harbor_python_guard_tests(self) -> None:
        paths = {path.as_posix() for path in self.module.DEFAULT_HARNESS_FILES}

        self.assertIn("evals/harbor/test_run_roder_pre_eval_diagnostics_args.py", paths)
        self.assertIn("evals/harbor/test_suggest_tbench_campaign.py", paths)
        self.assertIn("evals/harbor/test_summarize_tbench_campaigns.py", paths)
        self.assertIn("evals/harbor/test_validate_tbench_campaign_run_script.py", paths)
        self.assertIn(
            "evals/harbor/test_validate_tbench_campaign_run_script_guards.py",
            paths,
        )
        self.assertIn(
            "evals/harbor/test_validate_tbench_campaign_run_script_summary.py",
            paths,
        )
        self.assertIn(
            "evals/harbor/test_validate_tbench_campaign_score_projection.py",
            paths,
        )
        self.assertIn(
            "evals/harbor/test_validate_tbench_campaign_handoff.py",
            paths,
        )
        self.assertIn("evals/harbor/test_run_roder_tbench_full_gate.py", paths)
        self.assertIn("evals/harbor/test_validate_pre_eval_summary_configs.py", paths)
        self.assertIn("evals/harbor/test_validate_pre_eval_summary_harness.py", paths)
        self.assertIn("evals/harbor/test_validate_tbench_launch_plan_harness.py", paths)
        self.assertIn(
            "evals/harbor/test_validate_tbench_launch_plan_summary_copies.py",
            paths,
        )


if __name__ == "__main__":
    unittest.main()
