#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from run_roder_tbench_full_test_helpers import clean_summary  # noqa: E402

SCRIPT = ROOT / "evals/harbor/run-roder-tbench-full.sh"


class RunRoderTbenchFullGateTests(unittest.TestCase):
    def test_explicit_missing_pre_eval_summary_blocks_before_harbor(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_harbor = fake_bin / "harbor"
            fake_harbor.write_text("#!/usr/bin/env bash\necho harbor should not run >&2\n")
            fake_harbor.chmod(fake_harbor.stat().st_mode | stat.S_IXUSR)

            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env.get('PATH', '')}",
                    "RODER_HARBOR_LIVE_TBENCH": "1",
                    "RODER_HARBOR_PREBUILT_BINARY": str(temp_path / "roder"),
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(
                        temp_path / "missing-pre-eval-summary.json"
                    ),
                    "RODER_HARBOR_SKIP_PREFLIGHT": "1",
                }
            )

            result = subprocess.run(
                [str(SCRIPT)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertNotEqual(0, result.returncode)
        self.assertIn("validate_pre_eval_summary", result.stderr)
        self.assertIn("missing-pre-eval-summary.json", result.stderr)
        self.assertNotIn("harbor should not run", result.stderr)

    def test_dry_run_validates_summary_without_live_harbor_run(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            summary.write_text(
                json.dumps(clean_summary(prebuilt_binary=prebuilt), indent=2) + "\n"
            )

            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_harbor = fake_bin / "harbor"
            fake_harbor.write_text("#!/usr/bin/env bash\necho harbor should not run >&2\nexit 99\n")
            fake_harbor.chmod(fake_harbor.stat().st_mode | stat.S_IXUSR)

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.update(
                {
                    "PATH": f"{fake_bin}:{env.get('PATH', '')}",
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_SKIP_PREFLIGHT": "1",
                }
            )

            result = subprocess.run(
                [str(SCRIPT)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("Full run dry-run passed", result.stdout)
        self.assertNotIn("harbor should not run", result.stderr)

    def test_dry_run_writes_launch_plan(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            summary.write_text(
                json.dumps(clean_summary(prebuilt_binary=prebuilt), indent=2) + "\n"
            )
            launch_plan = temp_path / "launch-plan.json"

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.update(
                {
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_LAUNCH_PLAN": str(launch_plan),
                    "RODER_HARBOR_SKIP_PREFLIGHT": "1",
                    "RODER_HARBOR_PRE_EVAL_MAX_AGE_SECONDS": "3600",
                }
            )

            result = subprocess.run(
                [str(SCRIPT)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            plan = json.loads(launch_plan.read_text())
            expected_summary_sha = sha256(summary.read_bytes()).hexdigest()
            config_path = ROOT / "evals/harbor/tbench-full-gpt55-medium.json"
            expected_config_sha = sha256(config_path.read_bytes()).hexdigest()
            expected_prebuilt_sha = sha256(prebuilt.read_bytes()).hexdigest()
            expected_auth = prebuilt.parent / "codex.json"

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(str(summary), plan["preEvalSummary"])
        self.assertEqual("evals/harbor/tbench-full-gpt55-medium.json", plan["harborConfig"])
        self.assertEqual("evals/harbor/jobs/roder-tbench-full-gpt55-medium", plan["jobDir"])
        self.assertTrue(plan["dryRun"])
        self.assertFalse(plan["wouldRunHarbor"])
        self.assertFalse(plan["requireImagePreflight"])
        self.assertEqual(3600, plan["maxPreEvalAgeSeconds"])
        self.assertEqual("dry_run", plan["launchStatus"])
        self.assertEqual([], plan["blockedReasons"])
        self.assertEqual("ok", plan["preEvalSummaryStatus"]["status"])
        self.assertEqual([], plan["preEvalSummaryStatus"]["blockedChecks"])
        self.assertIn("generatedAt", plan["preEvalSummaryStatus"])
        self.assertEqual(
            expected_summary_sha,
            plan["preEvalSummarySha256"],
        )
        self.assertEqual(
            expected_config_sha,
            plan["harborConfigSha256"],
        )
        self.assertEqual(expected_config_sha, plan["preEvalHarborConfigSha256"])
        self.assertEqual(str(prebuilt), plan["prebuiltBinary"]["path"])
        self.assertEqual(expected_prebuilt_sha, plan["prebuiltBinary"]["sha256"])
        self.assertEqual(str(expected_auth), plan["authFile"]["path"])
        self.assertTrue(plan["authFile"]["validJson"])
        self.assertEqual("passed", plan["harborHarness"]["status"])
        self.assertGreater(plan["harborHarness"]["files"], 0)
        self.assertIn("combinedSha256", plan["harborHarness"])
        self.assertEqual("passed", plan["harborHarnessTests"]["status"])

    def test_dry_run_writes_default_launch_plan(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            summary.write_text(
                json.dumps(clean_summary(prebuilt_binary=prebuilt), indent=2) + "\n"
            )
            default_plan = ROOT / "evals/reports/harbor/roder-tbench-full-gpt55-medium-launch-plan.json"
            default_plan.unlink(missing_ok=True)

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.pop("RODER_HARBOR_LAUNCH_PLAN", None)
            env.update(
                {
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_SKIP_PREFLIGHT": "1",
                }
            )

            result = subprocess.run(
                [str(SCRIPT)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            plan = json.loads(default_plan.read_text())
            default_plan.unlink()

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertEqual(str(summary), plan["preEvalSummary"])
        self.assertTrue(plan["dryRun"])

    def test_live_mode_writes_launch_plan_before_harbor_run(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            summary.write_text(
                json.dumps(clean_summary(prebuilt_binary=prebuilt), indent=2) + "\n"
            )
            launch_plan = temp_path / "launch-plan.json"
            job_dir = ROOT / "evals/harbor/jobs/roder-tbench-full-gpt55-medium"
            created_job_dir = not job_dir.exists()
            job_dir.mkdir(parents=True, exist_ok=True)

            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_harbor = fake_bin / "harbor"
            fake_harbor.write_text("#!/usr/bin/env bash\necho harbor should not run >&2\nexit 99\n")
            fake_harbor.chmod(fake_harbor.stat().st_mode | stat.S_IXUSR)

            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env.get('PATH', '')}",
                    "RODER_HARBOR_LIVE_TBENCH": "1",
                    "RODER_HARBOR_PREBUILT_BINARY": str(prebuilt),
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_LAUNCH_PLAN": str(launch_plan),
                    "RODER_HARBOR_SKIP_PREFLIGHT": "1",
                }
            )

            result = subprocess.run(
                [str(SCRIPT)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            plan = json.loads(launch_plan.read_text())
            if created_job_dir:
                job_dir.rmdir()

        self.assertEqual(2, result.returncode)
        self.assertFalse(plan["dryRun"])
        self.assertFalse(plan["wouldRunHarbor"])
        self.assertTrue(plan["jobDirExists"])
        self.assertTrue(plan["jobDirBlocksLaunch"])
        self.assertEqual("existing_job_dir", plan["blockedBeforeHarbor"])
        self.assertEqual("blocked", plan["launchStatus"])
        self.assertEqual(["existing_job_dir"], plan["blockedReasons"])
        self.assertEqual(str(summary), plan["preEvalSummary"])
        self.assertFalse(plan["replaceJob"])
        self.assertTrue(plan["skipPreflight"])
        self.assertIn("launch blocked: existing_job_dir", result.stderr)
        self.assertNotIn("harbor should not run", result.stderr)

    def test_blocked_live_launch_exits_before_image_preflight(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            summary.write_text(
                json.dumps(
                    clean_summary(preflight_images=True, prebuilt_binary=prebuilt),
                    indent=2,
                )
                + "\n"
            )
            launch_plan = temp_path / "launch-plan.json"
            job_dir = ROOT / "evals/harbor/jobs/roder-tbench-full-gpt55-medium"
            created_job_dir = not job_dir.exists()
            job_dir.mkdir(parents=True, exist_ok=True)

            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_harbor = fake_bin / "harbor"
            fake_harbor.write_text("#!/usr/bin/env bash\necho harbor should not run >&2\nexit 99\n")
            fake_harbor.chmod(fake_harbor.stat().st_mode | stat.S_IXUSR)

            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env.get('PATH', '')}",
                    "RODER_HARBOR_LIVE_TBENCH": "1",
                    "RODER_HARBOR_PREBUILT_BINARY": str(prebuilt),
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                    "RODER_HARBOR_LAUNCH_PLAN": str(launch_plan),
                }
            )

            result = subprocess.run(
                [str(SCRIPT)],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            plan = json.loads(launch_plan.read_text())
            if created_job_dir:
                job_dir.rmdir()

        self.assertEqual(2, result.returncode)
        self.assertEqual("blocked", plan["launchStatus"])
        self.assertEqual(["existing_job_dir"], plan["blockedReasons"])
        self.assertIn("launch blocked: existing_job_dir", result.stderr)
        self.assertNotIn("preflight_tbench_images.py", result.stdout + result.stderr)
        self.assertNotIn("harbor should not run", result.stderr)


if __name__ == "__main__":
    unittest.main()
