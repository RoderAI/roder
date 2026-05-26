#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from hashlib import sha256
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "evals/harbor/generate_tbench_campaign.py"


class GenerateTbenchCampaignTests(unittest.TestCase):
    def test_default_campaign_writes_routed_configs_and_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--base-config",
                    "evals/harbor/tbench-full-gpt55-medium.json",
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
            manifest = json.loads(
                (output_dir / "validated-conversions-manifest.json").read_text()
            )
            medium = json.loads(
                Path(
                    next(
                        route
                        for route in manifest["routes"]
                        if route["name"] == "medium-validated"
                    )["config"]
                ).read_text()
            )
            xhigh = json.loads(
                Path(
                    next(
                        route
                        for route in manifest["routes"]
                        if route["name"] == "xhigh-validated"
                    )["config"]
                ).read_text()
            )
            plan_first = json.loads(
                Path(
                    next(
                        route
                        for route in manifest["routes"]
                        if route["name"] == "xhigh-plan-first"
                    )["config"]
                ).read_text()
            )
            run_script_exists = Path(manifest["runScript"]).exists()
            routes = {route["name"]: route for route in manifest["routes"]}
            medium_config_sha = sha256(
                Path(routes["medium-validated"]["config"]).read_bytes()
            ).hexdigest()
            medium_manifest_sha = routes["medium-validated"].get("configSha256")
            script = Path(manifest["runScript"]).read_text()

        self.assertEqual("validated-conversions", manifest["campaign"])
        self.assertEqual(3, manifest["summary"]["routes"])
        self.assertEqual(15, manifest["summary"]["uniqueTasks"])
        self.assertEqual(
            {
                "outputDir": str(output_dir / "pre-eval"),
                "summary": str(output_dir / "pre-eval/pre-eval-summary.json"),
            },
            manifest["preEval"],
        )
        self.assertIn("scoreProjection", manifest)
        self.assertEqual(
            {
                "suiteTasks": 89,
                "baselinePasses": 50,
                "campaignConversionCandidates": 15,
                "projectedPassesIfAllRoutesPass": 65,
                "projectedMeanIfAllRoutesPass": 65 / 89,
                "codexCliTargetPasses": 73,
                "codexCliGap": 8,
                "sotaTargetPasses": 76,
                "sotaGap": 11,
            },
            manifest["scoreProjection"],
        )
        self.assertTrue(run_script_exists)

        self.assertEqual(
            {"medium-validated", "xhigh-validated", "xhigh-plan-first"},
            set(routes),
        )
        self.assertEqual(medium_config_sha, medium_manifest_sha)
        self.assertTrue(routes["medium-validated"]["analysisJson"].endswith("-analysis.json"))
        self.assertTrue(routes["medium-validated"]["analysisMarkdown"].endswith(".md"))
        self.assertTrue(routes["medium-validated"]["analysisManifestDir"].endswith("/manifests/medium-validated"))
        self.assertEqual(
            str(output_dir / "medium-validated-images.json"),
            routes["medium-validated"]["imageManifest"],
        )
        self.assertEqual(
            str(output_dir / "medium-validated-launch-plan.json"),
            routes["medium-validated"]["launchPlan"],
        )
        self.assertEqual(
            str(output_dir / "xhigh-plan-first-images.json"),
            routes["xhigh-plan-first"]["imageManifest"],
        )
        self.assertEqual(
            str(output_dir / "xhigh-plan-first-launch-plan.json"),
            routes["xhigh-plan-first"]["launchPlan"],
        )
        self.assertIn(routes["medium-validated"]["imageManifest"], script)
        self.assertIn(routes["xhigh-plan-first"]["imageManifest"], script)
        self.assertIn(routes["medium-validated"]["launchPlan"], script)
        self.assertIn(routes["xhigh-plan-first"]["launchPlan"], script)

        self.assertEqual(4, medium["datasets"][0]["n_tasks"])
        self.assertEqual("medium", medium["agents"][0]["kwargs"]["reasoning"])
        self.assertEqual(7, xhigh["datasets"][0]["n_tasks"])
        self.assertEqual("xhigh", xhigh["agents"][0]["kwargs"]["reasoning"])
        self.assertEqual(4, plan_first["datasets"][0]["n_tasks"])
        self.assertTrue(plan_first["agents"][0]["kwargs"]["plan_first_enabled"])
        self.assertEqual(
            "medium",
            plan_first["agents"][0]["kwargs"]["plan_first_reasoning"],
        )
        self.assertIn("/logs/agent/roder-plan.md", plan_first["artifacts"])
        self.assertEqual(4, plan_first["orchestrator"]["n_concurrent_trials"])
        self.assertFalse(plan_first["environment"]["delete"])

    def test_default_campaign_writes_guarded_run_script(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
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
            manifest = json.loads(
                (output_dir / "validated-conversions-manifest.json").read_text()
            )
            run_script = Path(manifest["runScript"])
            script = run_script.read_text()
            run_script_executable = os.access(run_script, os.X_OK)

        self.assertTrue(run_script_executable)
        self.assertIn("RODER_HARBOR_LIVE_TBENCH", script)
        self.assertIn("validate_tbench_campaign.py", script)
        self.assertIn("preflight_tbench_images.py", script)
        self.assertIn("run-roder-pre-eval-diagnostics.sh", script)
        self.assertIn("validate_pre_eval_summary.py", script)
        self.assertIn("write_tbench_launch_plan.py", script)
        self.assertIn("validate_tbench_launch_plan.py", script)
        self.assertIn("RODER_HARBOR_PRE_EVAL_SUMMARY", script)
        self.assertIn("--require-prebuilt", script)
        self.assertIn("--require-auth", script)
        self.assertIn("--pre-eval-summary", script)
        self.assertIn("--verify-pre-eval-summary", script)
        self.assertIn("--image-preflight-manifest", script)
        self.assertIn("--require-image-preflight", script)
        self.assertIn("--verify-image-manifest", script)
        self.assertIn("--require-launch-plans", script)
        self.assertIn("--allow-dry-run-launch-plans", script)
        self.assertIn("--allow-dry-run", script)
        self.assertIn("--require-ready", script)
        self.assertIn("pre_eval_ran_here=0", script)
        self.assertIn("pre_eval_ran_here=1", script)
        self.assertIn("launch_plan_run_context_args=(--pre-eval-ran-here)", script)
        self.assertIn("pre_eval_args+=(--config", script)
        self.assertIn("--verify-prebuilt-binary", script)
        self.assertIn("--verify-auth-file", script)
        self.assertIn("summary_validation_args+=(--require-config", script)
        self.assertIn("analyze_tbench_run.py", script)
        self.assertIn("--require-clean", script)
        self.assertIn("validate_tbench_analysis.py", script)
        self.assertIn("--expected-trials 4", script)
        self.assertIn("--expected-trials 7", script)
        self.assertIn("--require-analysis", script)
        self.assertIn("roder-tbench-validated-conversions-medium", script)
        self.assertIn("medium-validated-analysis.json", script)
        self.assertIn("medium-validated.md", script)
        self.assertIn("harbor run --config", script)
        self.assertIn("validated-conversions-medium-validated.json", script)
        self.assertIn("medium-validated-images.json", script)
        self.assertIn("xhigh-plan-first-images.json", script)
        self.assertIn("route_job_dirs=(", script)
        self.assertIn("RODER_HARBOR_REPLACE_JOB", script)
        self.assertIn("already exists", script)

    def test_run_script_dry_run_validates_handoff_without_harbor(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            output_dir = temp_path / "campaign"
            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--route",
                    "xhigh-validated",
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
            manifest = json.loads(
                (output_dir / "validated-conversions-manifest.json").read_text()
            )
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
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env.get('PATH', '')}",
                    "PYTHON_STUB_LOG": str(stub_log),
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                }
            )

            run_result = subprocess.run(
                ["bash", manifest["runScript"]],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            stub_text = stub_log.read_text()

        self.assertEqual(0, run_result.returncode, run_result.stderr)
        self.assertIn("Campaign dry-run complete", run_result.stdout)
        self.assertNotIn("harbor", run_result.stderr.lower())
        self.assertIn("validate_pre_eval_summary.py", stub_text)
        self.assertIn("preflight_tbench_images.py", stub_text)
        self.assertIn("write_tbench_launch_plan.py", stub_text)
        self.assertIn("validate_tbench_launch_plan.py", stub_text)

    def test_run_script_blocks_existing_route_job_dir_without_replace(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            output_dir = temp_path / "campaign"
            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--route",
                    "xhigh-validated",
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
            manifest = json.loads(
                (output_dir / "validated-conversions-manifest.json").read_text()
            )
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text("#!/usr/bin/env bash\nexit 0\n")
            fake_python.chmod(0o755)
            fake_harbor = fake_bin / "harbor"
            fake_harbor.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'harbor ran\\n' >> \"$HARBOR_STUB_LOG\"\n"
                "exit 0\n"
            )
            fake_harbor.chmod(0o755)
            fake_repo = temp_path / "repo"
            existing_job = (
                fake_repo
                / "evals/harbor/jobs/roder-tbench-validated-conversions-xhigh"
            )
            existing_job.mkdir(parents=True)
            sentinel = existing_job / "result.json"
            sentinel.write_text("{}\n")
            harbor_log = temp_path / "harbor.log"
            summary = temp_path / "pre-eval-summary.json"
            summary.write_text("{}\n")
            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env.get('PATH', '')}",
                    "HARBOR_STUB_LOG": str(harbor_log),
                    "RODER_REPO_ROOT": str(fake_repo),
                    "RODER_HARBOR_LIVE_TBENCH": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
                }
            )

            run_result = subprocess.run(
                ["bash", manifest["runScript"]],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            sentinel_exists = sentinel.exists()
            harbor_log_exists = harbor_log.exists()

        self.assertEqual(2, run_result.returncode)
        self.assertTrue(sentinel_exists)
        self.assertFalse(harbor_log_exists)
        self.assertIn("already exists", run_result.stderr)
        self.assertIn("RODER_HARBOR_REPLACE_JOB=1", run_result.stderr)

    def test_route_filter_writes_only_requested_route(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--route",
                    "xhigh-validated",
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
            manifest = json.loads(
                (output_dir / "validated-conversions-manifest.json").read_text()
            )

        self.assertEqual(
            ["xhigh-validated"],
            [route["name"] for route in manifest["routes"]],
        )
        self.assertEqual(7, manifest["summary"]["uniqueTasks"])

    def test_verifier_contract_campaign_writes_near_miss_route(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--campaign",
                    "verifier-contract",
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
            manifest = json.loads(
                (output_dir / "verifier-contract-manifest.json").read_text()
            )
            route = manifest["routes"][0]
            config = json.loads(Path(route["config"]).read_text())

        self.assertEqual("verifier-contract", manifest["campaign"])
        self.assertEqual(["near-misses"], [route["name"] for route in manifest["routes"]])
        self.assertEqual(
            [
                "dna-assembly",
                "dna-insert",
                "gcode-to-text",
                "protein-assembly",
                "sam-cell-seg",
                "torch-tensor-parallelism",
                "video-processing",
            ],
            route["tasks"],
        )
        self.assertEqual("xhigh", route["reasoning"])
        self.assertEqual(7, manifest["summary"]["uniqueTasks"])
        self.assertEqual(7, config["datasets"][0]["n_tasks"])
        self.assertEqual(route["tasks"], config["datasets"][0]["task_names"])
        self.assertFalse(config["agents"][0]["kwargs"].get("plan_first_enabled", False))

    def test_environment_target_campaign_writes_service_target_route(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--campaign",
                    "environment-target",
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
            manifest = json.loads(
                (output_dir / "environment-target-manifest.json").read_text()
            )
            route = manifest["routes"][0]
            config = json.loads(Path(route["config"]).read_text())

        self.assertEqual("environment-target", manifest["campaign"])
        self.assertEqual(["service-targets"], [route["name"] for route in manifest["routes"]])
        self.assertEqual(
            [
                "install-windows-3.11",
                "qemu-alpine-ssh",
                "qemu-startup",
                "train-fasttext",
            ],
            route["tasks"],
        )
        self.assertEqual("xhigh", route["reasoning"])
        self.assertEqual(4, manifest["summary"]["uniqueTasks"])
        self.assertEqual(4, config["orchestrator"]["n_concurrent_trials"])
        self.assertFalse(config["environment"]["delete"])

    def test_historical_wins_campaign_writes_missing_win_routes(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"

            result = subprocess.run(
                [
                    "python3",
                    str(SCRIPT),
                    "--campaign",
                    "historical-wins",
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
            manifest = json.loads(
                (output_dir / "historical-wins-manifest.json").read_text()
            )
            routes = {route["name"]: route for route in manifest["routes"]}
            policy_config = json.loads(Path(routes["policy-framed"]["config"]).read_text())
            qemu_config = json.loads(Path(routes["environment-targeted"]["config"]).read_text())

        self.assertEqual("historical-wins", manifest["campaign"])
        self.assertEqual({"policy-framed", "environment-targeted"}, set(routes))
        self.assertEqual(
            ["password-recovery", "vulnerable-secret"],
            routes["policy-framed"]["tasks"],
        )
        self.assertEqual(["qemu-startup"], routes["environment-targeted"]["tasks"])
        self.assertEqual("medium", routes["policy-framed"]["reasoning"])
        self.assertEqual("medium", routes["environment-targeted"]["reasoning"])
        self.assertEqual(3, manifest["summary"]["uniqueTasks"])
        self.assertEqual(53, manifest["scoreProjection"]["projectedPassesIfAllRoutesPass"])
        self.assertEqual(20, manifest["scoreProjection"]["codexCliGap"])
        self.assertEqual(2, policy_config["datasets"][0]["n_tasks"])
        self.assertEqual(1, qemu_config["datasets"][0]["n_tasks"])

    def test_list_campaigns_does_not_require_output_dir(self) -> None:
        result = subprocess.run(
            ["python3", str(SCRIPT), "--list"],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn("validated-conversions", result.stdout)
        self.assertIn("verifier-contract", result.stdout)
        self.assertIn("environment-target", result.stdout)
        self.assertIn("historical-wins", result.stdout)
        self.assertIn("near-misses: 7 tasks", result.stdout)
        self.assertIn("service-targets: 4 tasks", result.stdout)
        self.assertIn("policy-framed: 2 tasks", result.stdout)


if __name__ == "__main__":
    unittest.main()
