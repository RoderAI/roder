#!/usr/bin/env python3

from __future__ import annotations

import json
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from pre_eval_summary_test_helpers import (
    MISSING_VALIDATION,
    MODULE_PATH,
    build_summary,
    load_module,
    write_image_manifest,
)



class PreEvalSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def build_summary(self, root: Path, **kwargs) -> dict:
        return build_summary(self.module, root, **kwargs)

    def test_summary_writer_delegates_tbench_contract_to_run_summary_helper(self) -> None:
        source = MODULE_PATH.read_text()

        self.assertNotIn("tbench_diagnostic_contract", source)
        self.assertIn("tbench_eval_run_summary", source)

    def test_summary_status_is_ok_when_all_checks_are_clean(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            image_manifest = root / "image-preflight.json"
            write_image_manifest(image_manifest, clean=True, tasks=2, present=2)

            summary = self.build_summary(
                root,
                tbench_outcomes=["pass", "pass"],
                analysis_status="ok",
                image_manifest=image_manifest,
            )

            self.assertEqual("ok", summary["status"])
            self.assertEqual([], summary["blockedChecks"])

    def test_summary_status_lists_blocking_checks(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            image_manifest = root / "image-preflight.json"
            write_image_manifest(
                image_manifest,
                clean=False,
                tasks=2,
                present=1,
                missing=1,
            )

            summary = self.build_summary(
                root,
                tbench_outcomes=["pass"],
                analysis_status="blocked",
                image_manifest=image_manifest,
            )

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(
                ["harborAnalysisBaseline", "imagePreflight"],
                summary["blockedChecks"],
            )

    def test_missing_tbench_eval_run_blocks_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(Path(temp))

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["tbenchDiagnostics"], summary["blockedChecks"])
            self.assertEqual("missing", summary["checks"]["tbenchDiagnostics"]["status"])

    def test_empty_tbench_eval_run_blocks_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(Path(temp), tbench_outcomes=[])

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["tbenchDiagnostics"], summary["blockedChecks"])
            self.assertEqual("failed", summary["checks"]["tbenchDiagnostics"]["status"])

    def test_failed_readiness_status_blocks_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                harbor_readiness_status="failed",
                failure_step="python3 evals/harbor/validate_harbor_readiness.py",
                failure_exit_code=1,
            )

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(
                {
                    "step": "python3 evals/harbor/validate_harbor_readiness.py",
                    "exitCode": 1,
                },
                summary["failure"],
            )
            self.assertEqual(
                ["harborReadiness", "tbenchDiagnostics"],
                summary["blockedChecks"],
            )
            self.assertEqual("failed", summary["checks"]["harborReadiness"]["status"])

    def test_failed_roder_evals_status_blocks_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(Path(temp), roder_evals_status="failed")

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(
                ["roderEvalsLib", "tbenchDiagnostics"],
                summary["blockedChecks"],
            )
            self.assertEqual("failed", summary["checks"]["roderEvalsLib"]["status"])

    def test_failed_harbor_harness_tests_status_blocks_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                harbor_harness_tests_status="failed",
            )

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(
                ["harborHarnessTests", "tbenchDiagnostics"],
                summary["blockedChecks"],
            )
            self.assertEqual("failed", summary["checks"]["harborHarnessTests"]["status"])

    def test_missing_analysis_validation_blocks_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                tbench_outcomes=["pass"],
                analysis_status=MISSING_VALIDATION,
            )

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["harborAnalysisBaseline"], summary["blockedChecks"])
            self.assertEqual(
                "missing",
                summary["checks"]["harborAnalysisBaseline"]["status"],
            )

    def test_missing_image_preflight_manifest_blocks_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)

            summary = self.build_summary(
                root,
                tbench_outcomes=["pass"],
                image_manifest=root / "missing-image-manifest.json",
            )

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["imagePreflight"], summary["blockedChecks"])
            self.assertEqual("missing", summary["checks"]["imagePreflight"]["status"])

    def test_missing_speed_policy_eval_run_blocks_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                tbench_outcomes=["pass"],
                include_speed=True,
            )

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["speedPolicy"], summary["blockedChecks"])
            self.assertEqual("missing", summary["checks"]["speedPolicy"]["status"])

    def test_passed_tbench_missing_verification_blocks_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                tbench_outcomes=["pass"],
                tbench_metrics=[
                    [{"name": "verification_completed", "value": 0.0}],
                ],
            )

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["tbenchDiagnostics"], summary["blockedChecks"])
            self.assertEqual("failed", summary["checks"]["tbenchDiagnostics"]["status"])
            self.assertEqual(
                1,
                summary["checks"]["tbenchDiagnostics"]["missingVerification"],
            )

    def test_passed_tbench_unknown_reliability_errors_block_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(
                Path(temp),
                tbench_outcomes=["pass"],
                tbench_metrics=[
                    [
                        {"name": "verification_completed", "value": 1.0},
                        {"name": "reliability_unknown_errors", "value": 2.0},
                    ],
                ],
            )

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["tbenchDiagnostics"], summary["blockedChecks"])
            self.assertEqual("failed", summary["checks"]["tbenchDiagnostics"]["status"])
            self.assertEqual(
                2,
                summary["checks"]["tbenchDiagnostics"]["unknownReliabilityErrors"],
            )

    def test_require_ok_exits_nonzero_after_writing_blocked_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            tbench_dir = root / "tbench-diagnostics"
            tbench_dir.mkdir()
            summary_path = root / "pre-eval-summary.json"

            result = subprocess.run(
                [
                    sys.executable,
                    str(MODULE_PATH),
                    "--summary",
                    str(summary_path),
                    "--output-dir",
                    str(root),
                    "--tbench-dir",
                    str(tbench_dir),
                    "--run-tests",
                    "--require-ok",
                ],
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertEqual(1, result.returncode)
            summary = json.loads(summary_path.read_text())
            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["tbenchDiagnostics"], summary["blockedChecks"])
            self.assertIn("blocked", result.stderr)

    def test_summary_records_eval_analysis_and_prebuilt_metadata(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            image_manifest = root / "image-preflight.json"
            write_image_manifest(image_manifest, clean=True)
            prebuilt = root / "roder-linux-amd64"
            prebuilt.write_bytes(b"roder")
            prebuilt.chmod(prebuilt.stat().st_mode | stat.S_IXUSR)

            summary = self.build_summary(
                root,
                tbench_outcomes=["pass", "fail"],
                analysis_status="ok",
                image_manifest=image_manifest,
                run_tests=False,
                require_prebuilt=True,
                require_auth=True,
                prebuilt_binary=prebuilt,
            )

            self.assertEqual("skipped", summary["checks"]["roderEvalsLib"]["status"])
            self.assertEqual(9, summary["checks"]["tbenchDiagnostics"]["fixtures"])
            self.assertEqual(8, summary["checks"]["tbenchDiagnostics"]["passed"])
            self.assertEqual("ok", summary["checks"]["harborAnalysisBaseline"]["status"])
            self.assertEqual(str(prebuilt), summary["prebuiltBinary"]["path"])
            self.assertTrue(summary["prebuiltBinary"]["exists"])
            self.assertTrue(summary["prebuiltBinary"]["executable"])
            self.assertEqual(5, summary["prebuiltBinary"]["sizeBytes"])
            self.assertIn("sha256", summary["prebuiltBinary"])
            self.assertEqual(str(root / "codex.json"), summary["authFile"]["path"])
            self.assertFalse(summary["authFile"]["exists"])
            self.assertTrue(summary["authFile"]["required"])
            self.assertEqual("passed", summary["checks"]["imagePreflight"]["status"])
            self.assertEqual(1, summary["checks"]["imagePreflight"]["present"])

    def test_missing_prebuilt_metadata_is_recorded(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)

            summary = self.build_summary(
                root,
                tbench_outcomes=[],
                require_prebuilt=True,
                auth_file=root / "missing-codex.json",
            )

            self.assertFalse(summary["prebuiltBinary"]["exists"])
            self.assertFalse(summary["prebuiltBinary"]["executable"])
            self.assertNotIn("sha256", summary["prebuiltBinary"])
            self.assertFalse(summary["authFile"]["exists"])
            self.assertFalse(summary["authFile"]["required"])

    def test_auth_metadata_does_not_expose_tokens(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            auth = root / "codex.json"
            auth.write_text(
                json.dumps(
                    {
                        "access": "secret-access",
                        "refresh": "secret-refresh",
                        "account_id": "account",
                        "expires": "2027-01-01T00:00:00Z",
                        "type": "bearer",
                    }
                )
            )

            summary = self.build_summary(
                root,
                tbench_outcomes=[],
                require_auth=True,
                auth_file=auth,
            )

            text = json.dumps(summary)
            self.assertTrue(summary["authFile"]["exists"])
            self.assertEqual(
                ["access", "account_id", "expires", "refresh", "type"],
                summary["authFile"]["jsonFields"],
            )
            self.assertNotIn("secret-access", text)
            self.assertNotIn("secret-refresh", text)

    def test_summary_options_record_live_run_readiness_intent(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)

            summary = self.build_summary(
                root,
                tbench_outcomes=["pass"],
                run_tests=False,
                include_speed=True,
                require_prebuilt=True,
                require_auth=True,
                preflight_images=True,
                pull_images=True,
                image_config="evals/harbor/tbench-full-gpt55-medium.json",
            )

            self.assertEqual(
                {
                    "runTests": False,
                    "includeSpeed": True,
                    "requirePrebuilt": True,
                    "requireAuth": True,
                    "preflightImages": True,
                    "offlineImages": False,
                    "pullImages": True,
                    "imageConfig": "evals/harbor/tbench-full-gpt55-medium.json",
                    "analysisTarget": None,
                    "analysisBaseline": None,
                    "campaignSummary": None,
                },
                summary["options"],
            )

    def test_summary_records_offline_image_preflight_intent(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            image_manifest = root / "image-preflight.json"
            write_image_manifest(image_manifest, clean=True, offline=True)

            summary = self.build_summary(
                root,
                tbench_outcomes=["pass"],
                preflight_images=True,
                offline_images=True,
                image_config="evals/harbor/tbench-full-gpt55-medium.json",
                image_manifest=image_manifest,
            )

            self.assertTrue(summary["options"]["offlineImages"])
            self.assertFalse(summary["options"]["pullImages"])
            self.assertTrue(summary["checks"]["imagePreflight"]["offline"])

    def test_summary_blocks_offline_image_preflight_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            image_manifest = root / "image-preflight.json"
            write_image_manifest(image_manifest, clean=True, offline=False)

            summary = self.build_summary(
                root,
                tbench_outcomes=["pass"],
                preflight_images=True,
                offline_images=True,
                image_config="evals/harbor/tbench-full-gpt55-medium.json",
                image_manifest=image_manifest,
            )

            self.assertEqual("blocked", summary["status"])
            self.assertEqual(["imagePreflight"], summary["blockedChecks"])
            self.assertEqual("failed", summary["checks"]["imagePreflight"]["status"])
            self.assertIn(
                "image preflight did not run in offline mode",
                summary["checks"]["imagePreflight"]["issues"],
            )

    def test_summary_blocks_contradictory_image_preflight_options(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            image_manifest = root / "image-preflight.json"
            write_image_manifest(image_manifest, clean=True, offline=True)

            summary = self.build_summary(
                root,
                tbench_outcomes=["pass"],
                preflight_images=True,
                offline_images=True,
                pull_images=True,
                image_config="evals/harbor/tbench-full-gpt55-medium.json",
                image_manifest=image_manifest,
            )

            self.assertEqual("blocked", summary["status"])
            self.assertIn("preEvalOptions", summary["blockedChecks"])
            self.assertEqual("failed", summary["checks"]["preEvalOptions"]["status"])
            self.assertIn(
                "offlineImages cannot be combined with pullImages",
                summary["checks"]["preEvalOptions"]["issues"],
            )


if __name__ == "__main__":
    unittest.main()
