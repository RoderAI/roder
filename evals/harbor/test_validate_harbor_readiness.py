#!/usr/bin/env python3

from __future__ import annotations

import copy
import importlib.util
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))
MODULE_PATH = ROOT / "evals/harbor/validate_harbor_readiness.py"


def load_module():
    spec = importlib.util.spec_from_file_location("validate_harbor_readiness", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class HarborReadinessValidationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def load_config(self, name: str) -> dict:
        return json.loads((ROOT / f"evals/harbor/{name}").read_text())

    def test_checked_in_full_config_is_ready_for_local_deadline_run(self) -> None:
        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-medium.json",
            self.load_config("tbench-full-gpt55-medium.json"),
        )

        self.assertEqual([], issues)

    def test_checked_in_smoke_config_is_ready_for_local_deadline_run(self) -> None:
        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-smoke.json",
            self.load_config("tbench-smoke.json"),
        )

        self.assertEqual([], issues)

    def test_default_configs_include_gemini_35_flash_validation_set(self) -> None:
        config_paths = [str(path) for path in self.module.DEFAULT_CONFIGS]
        self.assertIn("evals/harbor/tbench-gemini35-flash-validation.json", config_paths)

        config = self.load_config("tbench-gemini35-flash-validation.json")
        agent = config["agents"][0]
        dataset = config["datasets"][0]

        self.assertEqual("gemini/gemini-3.5-flash", agent["model_name"])
        self.assertEqual("medium", agent["kwargs"]["reasoning"])
        self.assertEqual(6, dataset["n_tasks"])
        self.assertEqual(6, len(dataset["task_names"]))
        self.assertLessEqual(len(dataset["task_names"]), 8)
        self.assertNotIn("db-wal-recovery", dataset["task_names"])
        self.assertNotIn("query-optimize", dataset["task_names"])

    def test_deadline_regression_is_reported(self) -> None:
        config = self.load_config("tbench-full-gpt55-medium.json")
        config = copy.deepcopy(config)
        config["agents"][0]["override_timeout_sec"] = 900
        config["agents"][0]["kwargs"]["soft_timeout_sec"] = 890
        config["agents"][0]["kwargs"]["speed_policy_eval_deadline_seconds"] = 870

        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-medium.json",
            config,
        )

        text = "\n".join(issues)
        self.assertIn("override_timeout_sec", text)
        self.assertIn("soft_timeout_sec", text)
        self.assertIn("speed_policy_eval_deadline_seconds", text)

    def test_checked_in_per_task_config_is_leaderboard_valid_candidate(self) -> None:
        config = self.load_config("tbench-full-gpt55-xhigh-pertask.json")
        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-xhigh-pertask.json", config
        )
        self.assertEqual([], issues)
        self.assertEqual(
            "leaderboard-valid-candidate", self.module.config_deadline_track(config)
        )

    def test_checked_in_minimal_config_is_codex_parity_and_valid(self) -> None:
        config = self.load_config("tbench-full-gpt55-xhigh-minimal.json")
        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-xhigh-minimal.json", config
        )
        self.assertEqual([], issues)
        self.assertEqual("codex-parity", self.module.config_deadline_track(config))

    def test_leaderboard_track_rejects_benchmark_guidance(self) -> None:
        config = copy.deepcopy(self.load_config("tbench-full-gpt55-xhigh-minimal.json"))
        config["agents"][0]["kwargs"]["benchmark_guidance_enabled"] = True
        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-xhigh-minimal.json", config
        )
        self.assertIn("benchmark_guidance_enabled", "\n".join(issues))

    def test_leaderboard_track_rejects_task_ledger(self) -> None:
        config = copy.deepcopy(self.load_config("tbench-full-gpt55-xhigh-minimal.json"))
        config["agents"][0]["kwargs"]["task_ledger_required"] = True
        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-xhigh-minimal.json", config
        )
        self.assertIn("task_ledger_required", "\n".join(issues))

    def test_codex_parity_track_rejects_internal_deadline(self) -> None:
        config = copy.deepcopy(self.load_config("tbench-full-gpt55-xhigh-minimal.json"))
        config["agents"][0]["kwargs"]["speed_policy_eval_deadline_seconds"] = 720
        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-xhigh-minimal.json", config
        )
        self.assertIn("speed_policy_eval_deadline_seconds", "\n".join(issues))

    def test_per_task_track_rejects_window_override(self) -> None:
        config = copy.deepcopy(
            self.load_config("tbench-full-gpt55-xhigh-pertask.json")
        )
        config["agents"][0]["override_timeout_sec"] = 1800
        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-xhigh-pertask.json", config
        )
        self.assertIn("override_timeout_sec", "\n".join(issues))

    def test_per_task_track_rejects_agent_timeout_multiplier(self) -> None:
        config = copy.deepcopy(
            self.load_config("tbench-full-gpt55-xhigh-pertask.json")
        )
        config["agent_timeout_multiplier"] = 2.0
        config["agents"][0]["kwargs"]["agent_timeout_multiplier_hint"] = 2.0
        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-xhigh-pertask.json", config
        )
        self.assertIn("agent_timeout_multiplier", "\n".join(issues))
        self.assertEqual("local-only", self.module.config_deadline_track(config))

    def test_per_task_track_rejects_pinned_static_deadlines(self) -> None:
        config = copy.deepcopy(
            self.load_config("tbench-full-gpt55-xhigh-pertask.json")
        )
        config["agents"][0]["kwargs"]["soft_timeout_sec"] = 780
        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-xhigh-pertask.json", config
        )
        self.assertIn("soft_timeout_sec", "\n".join(issues))

    def test_source_fallback_is_reported_for_prebuilt_eval_configs(self) -> None:
        config = self.load_config("tbench-full-gpt55-medium.json")
        config = copy.deepcopy(config)
        config["agents"][0]["kwargs"]["include_local_source"] = "true"

        issues = self.module.validate_config(
            ROOT / "evals/harbor/tbench-full-gpt55-medium.json",
            config,
        )

        self.assertIn("include_local_source", "\n".join(issues))

    def test_gitignore_must_cover_generated_eval_outputs(self) -> None:
        issues = self.module.validate_gitignore(
            "target/\nevals/harbor/jobs/\nevals/reports/\n"
        )

        self.assertIn("evals/harbor/artifacts/", "\n".join(issues))

    def test_required_prebuilt_must_be_linux_x86_64_elf(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "roder"
            binary.write_text("#!/bin/sh\nexit 0\n")
            binary.chmod(binary.stat().st_mode | stat.S_IXUSR)

            issues = self.module.validate_prebuilt_binary(binary, required=True)

        text = "\n".join(issues)
        self.assertIn("Linux x86-64 ELF", text)

    def test_required_prebuilt_can_come_from_environment_override(self) -> None:
        configured = os.environ.get("RODER_HARBOR_PREBUILT_BINARY")
        try:
            os.environ["RODER_HARBOR_PREBUILT_BINARY"] = str(
                ROOT / "evals/harbor/artifacts/roder-linux-amd64"
            )

            issues = self.module.validate_prebuilt_binary(
                Path("/does/not/matter"),
                required=True,
            )
        finally:
            if configured is None:
                os.environ.pop("RODER_HARBOR_PREBUILT_BINARY", None)
            else:
                os.environ["RODER_HARBOR_PREBUILT_BINARY"] = configured

        self.assertEqual([], issues)

    def test_required_auth_rejects_missing_or_malformed_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            missing = Path(temp) / "missing-codex.json"
            malformed = Path(temp) / "codex.json"
            malformed.write_text(json.dumps({"access": "token"}))

            missing_issues = self.module.validate_auth_file(missing, required=True)
            malformed_issues = self.module.validate_auth_file(malformed, required=True)

        self.assertIn("auth file missing", "\n".join(missing_issues))
        self.assertIn("missing required auth field", "\n".join(malformed_issues))

    def test_required_auth_accepts_roder_codex_auth_shape(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            auth = Path(temp) / "codex.json"
            auth.write_text(
                json.dumps(
                    {
                        "access": "access-token",
                        "refresh": "refresh-token",
                        "account_id": "account",
                        "expires": 1800000000,
                        "type": "bearer",
                    }
                )
            )

            issues = self.module.validate_auth_file(auth, required=True)

        self.assertEqual([], issues)


if __name__ == "__main__":
    unittest.main()
