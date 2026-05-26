#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
from hashlib import sha256
import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from tbench_diagnostic_test_data import passing_diagnostic_results  # noqa: E402

MODULE_PATH = ROOT / "evals/harbor/write_pre_eval_summary.py"


def load_module():
    spec = importlib.util.spec_from_file_location("write_pre_eval_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def write_eval_run(directory: Path) -> None:
    directory.mkdir(parents=True, exist_ok=True)
    (directory / "eval-run.json").write_text(
        json.dumps({"results": passing_diagnostic_results()})
    )


def write_config(
    path: Path,
    *,
    include_local_source: str,
    benchmark_guidance_enabled: bool = True,
) -> None:
    path.write_text(
        json.dumps(
            {
                "job_name": "roder-tbench-full-gpt55-medium",
                "timeout_multiplier": 1.0,
                "environment": {"delete": False},
                "orchestrator": {"n_concurrent_trials": 4},
                "agents": [
                    {
                        "model_name": "codex/gpt-5.5",
                        "override_timeout_sec": 1800,
                        "kwargs": {
                            "reasoning": "medium",
                            "speed_policy_enabled": False,
                            "speed_policy_eval_deadline_seconds": 1740,
                            "soft_timeout_sec": 1780,
                            "task_ledger_required": True,
                            "benchmark_guidance_enabled": benchmark_guidance_enabled,
                            "policy_mode": "bypass",
                            "include_prebuilt_binary": "true",
                            "include_local_source": include_local_source,
                        },
                    }
                ],
                "artifacts": [
                    "/logs/agent/roder-cli.txt",
                    "/logs/agent/roder-events.jsonl",
                    "/logs/agent/roder-stderr.txt",
                    "/logs/agent/roder-last-message.txt",
                    "/logs/agent/setup-summary.txt",
                    "/logs/agent/roder-run-summary.json",
                ],
            }
        )
    )


class PreEvalConfigSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def build_summary(self, root: Path, config_path: Path) -> dict:
        tbench_dir = root / "tbench-diagnostics"
        write_eval_run(tbench_dir)
        return self.module.build_summary(
            output_root=root,
            tbench_dir=tbench_dir,
            speed_dir=None,
            analysis_dir=None,
            run_tests=False,
            include_speed=False,
            require_prebuilt=False,
            preflight_images=False,
            pull_images=False,
            image_config="",
            analysis_target="",
            analysis_baseline="",
            prebuilt_binary=root / "missing-roder",
            auth_file=root / "codex.json",
            require_auth=False,
            image_manifest=None,
            config_paths=(config_path,),
        )

    def test_summary_records_prebuilt_only_harbor_config_invariants(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            config_path = root / "tbench.json"
            write_config(config_path, include_local_source="false")

            summary = self.build_summary(root, config_path)

            config_check = summary["checks"]["harborConfigs"]
            expected_sha = sha256(config_path.read_bytes()).hexdigest()
            self.assertEqual("passed", config_check["status"])
            self.assertEqual([], config_check["issues"])
            self.assertEqual(1, config_check["configs"])
            self.assertEqual(
                {
                    "path": str(config_path),
                    "sha256": expected_sha,
                    "jobName": "roder-tbench-full-gpt55-medium",
                    "modelName": "codex/gpt-5.5",
                    "reasoning": "medium",
                    "readinessIssues": [],
                    "nConcurrentTrials": 4,
                    "environmentDelete": False,
                    "overrideTimeoutSec": 1800,
                    "softTimeoutSec": 1780,
                    "evalDeadlineSeconds": 1740,
                    "speedPolicyEnabled": False,
                    "taskLedgerRequired": True,
                    "includePrebuiltBinary": True,
                    "includeLocalSource": False,
                },
                config_check["entries"][0],
            )

    def test_summary_blocks_source_fallback_harbor_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            config_path = root / "tbench.json"
            write_config(config_path, include_local_source="true")

            summary = self.build_summary(root, config_path)

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["harborConfigs"], summary["blockedChecks"])
            self.assertEqual("failed", summary["checks"]["harborConfigs"]["status"])
            self.assertIn("include_local_source", "\n".join(summary["checks"]["harborConfigs"]["issues"]))

    def test_summary_blocks_missing_benchmark_guidance_invariant(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            config_path = root / "tbench.json"
            write_config(
                config_path,
                include_local_source="false",
                benchmark_guidance_enabled=False,
            )

            summary = self.build_summary(root, config_path)

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["harborConfigs"], summary["blockedChecks"])
            self.assertEqual("failed", summary["checks"]["harborConfigs"]["status"])
            self.assertIn(
                "benchmark_guidance_enabled",
                "\n".join(summary["checks"]["harborConfigs"]["issues"]),
            )


if __name__ == "__main__":
    unittest.main()
