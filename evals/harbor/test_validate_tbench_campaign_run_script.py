#!/usr/bin/env python3

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path

try:
    from .tbench_campaign_test_helpers import generate_campaign, validate_campaign
except ImportError:
    from tbench_campaign_test_helpers import generate_campaign, validate_campaign


class ValidateTbenchCampaignRunScriptTests(unittest.TestCase):
    def test_rejects_generated_run_script_missing_job_dir_preservation_guard(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            run_script.write_text(
                script.replace("RODER_HARBOR_REPLACE_JOB", "RODER_HARBOR_UNSAFE_REPLACE")
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript missing required guard: RODER_HARBOR_REPLACE_JOB",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_route_job_dir_guard(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"  {route['jobDir']}\n"
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, "  evals/harbor/jobs/stale-route\n", 1)
                + f"\necho {route['jobDir']}\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript route job dirs mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_route_config_path(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            self.assertIn(route["config"], script)
            wrong_config = str(Path(route["config"]).with_name("stale-route.json"))
            run_script.write_text(
                script.replace(route["config"], wrong_config)
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript missing route xhigh-validated config path",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_extra_harbor_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            run_script.write_text(
                run_script.read_text()
                + "\nharbor run --config evals/harbor/tbench-smoke.json\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript harbor configs mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_harbor_run_hidden_in_comment(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"harbor run --config {route['config']}"
            self.assertIn(expected, script)
            run_script.write_text(script.replace(expected, "# " + expected, 1))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript harbor configs mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_preflight_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"preflight_tbench_images.py --config {route['config']}"
            self.assertIn(expected, script)
            stale = "preflight_tbench_images.py --config evals/harbor/tbench-smoke.json"
            run_script.write_text(script.replace(expected, stale))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript image preflight configs mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_preflight_hidden_in_comment(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"preflight_tbench_images.py --config {route['config']}"
            self.assertIn(expected, script)
            run_script.write_text(script.replace(expected, "# " + expected, 1))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript image preflight configs mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_preflight_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"--manifest {route['imageManifest']}"
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, "--manifest stale-images.json")
                + f"\necho {route['imageManifest']}\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript image preflight commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_route_launch_plan(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"--output {route['launchPlan']}"
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, "--output stale-launch-plan.json", 1)
                + f"\necho {route['launchPlan']}\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript launch plan commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_missing_launch_plan_pre_eval_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = '--pre-eval-summary "$pre_eval_summary" '
            self.assertIn(expected, script)
            run_script.write_text(script.replace(expected, "", 1))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript launch plan commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_stale_launch_plan_pre_eval_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = '--pre-eval-summary "$pre_eval_summary"'
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, "--pre-eval-summary stale-summary.json", 1)
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript launch plan commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_missing_launch_plan_run_context(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = ' ${launch_plan_run_context_args[@]+"${launch_plan_run_context_args[@]}"}'
            self.assertIn(expected, script)
            run_script.write_text(script.replace(expected, "", 1))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript launch plan commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_missing_ready_launch_plan_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = (
                f"python3 evals/harbor/validate_tbench_launch_plan.py {route['launchPlan']} "
                "--require-ready "
            )
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, f"# {expected}", 1)
                + f"\necho {route['launchPlan']}\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript launch plan validation commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_missing_launch_plan_summary_verification(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = " --verify-pre-eval-summary "
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, " ", 1)
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript launch plan validation commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_missing_launch_plan_image_manifest(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"--image-preflight-manifest {route['imageManifest']}"
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, "--image-preflight-manifest stale-images.json", 1)
                + f"\necho {route['imageManifest']}\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript launch plan commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_that_bypasses_preflight_args(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = '"${preflight_args[@]}"'
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, "--pull")
                + f"\necho {expected}\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript image preflight commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_pre_eval_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"pre_eval_args+=(--config {route['config']})"
            self.assertIn(expected, script)
            stale = "pre_eval_args+=(--config evals/harbor/tbench-smoke.json)"
            run_script.write_text(script.replace(expected, stale))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript pre-eval configs mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_summary_required_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"summary_validation_args+=(--require-config {route['config']})"
            self.assertIn(expected, script)
            stale = (
                "summary_validation_args+=(--require-config "
                "evals/harbor/tbench-smoke.json)"
            )
            run_script.write_text(script.replace(expected, stale))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript summary required configs mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_analysis_job_dir(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"analyze_tbench_run.py {route['jobDir']} "
            self.assertIn(expected, script)
            stale = "analyze_tbench_run.py evals/harbor/jobs/stale-route "
            run_script.write_text(script.replace(expected, stale))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript analysis commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_that_drops_require_clean_analysis(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = " --require-clean "
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, " ", 1)
                + "\necho --require-clean\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript analysis commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_that_analyzes_before_harbor_run(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            harbor = f"harbor run --config {route['config']}"
            analysis = f"python3 evals/harbor/analyze_tbench_run.py {route['jobDir']} "
            self.assertIn(harbor, script)
            self.assertIn(analysis, script)
            run_script.write_text(script.replace(f"{harbor}\n{analysis}", f"{analysis}\n{harbor}\n", 1))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript route command order mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_baseline_analysis_json(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"validate_tbench_analysis.py {route['analysisJson']} "
            self.assertIn(expected, script)
            stale = "validate_tbench_analysis.py stale-analysis.json "
            run_script.write_text(script.replace(expected, stale))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript baseline validation commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_expected_trials(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = f"--expected-trials {route['taskCount']}"
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, "--expected-trials 999")
                + f"\necho {expected}\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript baseline validation commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_with_wrong_baseline_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = "--baseline evals/harbor/tbench-clean-baseline.json"
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(expected, "--baseline stale-clean-baseline.json")
                + f"\necho {expected}\n"
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript baseline validation commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_missing_final_campaign_analysis_validation(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = (
                'python3 evals/harbor/validate_tbench_campaign.py "$MANIFEST" '
                '--require-image-preflight --require-analysis --require-launch-plans '
                '--preflight-dir "$PREFLIGHT_DIR"'
            )
            self.assertIn(expected, script)
            run_script.write_text(
                script.replace(
                    expected,
                    'python3 evals/harbor/validate_tbench_campaign.py "$MANIFEST" '
                    '--require-image-preflight --preflight-dir "$PREFLIGHT_DIR"',
                )
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript campaign validation commands mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_that_final_campaign_analysis_runs_too_early(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            route = data["routes"][0]
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            final_validation = (
                'python3 evals/harbor/validate_tbench_campaign.py "$MANIFEST" '
                '--require-image-preflight --require-analysis --require-launch-plans '
                '--preflight-dir "$PREFLIGHT_DIR"'
            )
            harbor = f"harbor run --config {route['config']}"
            self.assertIn(final_validation, script)
            self.assertIn(harbor, script)
            without_final = script.replace("\n" + final_validation + "\n", "\n", 1)
            run_script.write_text(
                without_final.replace(harbor, final_validation + "\n" + harbor, 1)
            )

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript final campaign validation order mismatch",
            result.stderr,
        )

    def test_rejects_generated_run_script_that_does_not_run_pre_eval_args(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            output_dir = Path(temp) / "campaign"
            manifest = generate_campaign(output_dir)
            data = json.loads(manifest.read_text())
            run_script = Path(data["runScript"])
            script = run_script.read_text()
            expected = '  evals/harbor/run-roder-pre-eval-diagnostics.sh "${pre_eval_args[@]}"'
            self.assertIn(expected, script)
            run_script.write_text(script.replace(expected, f"  echo {expected.strip()}"))

            result = validate_campaign(manifest)

        self.assertNotEqual(0, result.returncode)
        self.assertIn(
            "runScript pre-eval diagnostics invocation mismatch",
            result.stderr,
        )


if __name__ == "__main__":
    unittest.main()
