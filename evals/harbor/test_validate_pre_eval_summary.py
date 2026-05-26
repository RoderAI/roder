#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import hashlib
import sys
import tempfile
import unittest
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from validate_pre_eval_summary_test_helpers import clean_summary  # noqa: E402
from pre_eval_live_checks import combined_file_digest  # noqa: E402

MODULE_PATH = ROOT / "evals/harbor/validate_pre_eval_summary.py"


def load_module():
    spec = importlib.util.spec_from_file_location("validate_pre_eval_summary", MODULE_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class ValidatePreEvalSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def test_clean_live_ready_summary_passes_required_gates(self) -> None:
        result = self.module.validate_summary(
            clean_summary(),
            require_prebuilt=True,
            require_auth=True,
            require_tests=True,
            require_image_preflight=True,
            require_analysis=True,
        )

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_missing_required_image_preflight_blocks(self) -> None:
        summary = clean_summary()
        summary["options"]["preflightImages"] = False
        summary["checks"].pop("imagePreflight")

        result = self.module.validate_summary(
            summary,
            require_prebuilt=True,
            require_auth=True,
            require_tests=True,
            require_image_preflight=True,
            require_analysis=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("required image preflight did not run", result.issues)

    def test_require_tests_blocks_missing_harbor_harness_tests(self) -> None:
        summary = clean_summary()
        summary["checks"].pop("harborHarnessTests")

        result = self.module.validate_summary(summary, require_tests=True)

        self.assertFalse(result.ok)
        self.assertIn("harborHarnessTests check missing", result.issues)

    def test_require_tests_blocks_failed_harbor_harness_tests(self) -> None:
        summary = clean_summary()
        summary["checks"]["harborHarnessTests"]["status"] = "failed"

        result = self.module.validate_summary(summary, require_tests=True)

        self.assertFalse(result.ok)
        self.assertIn(
            "required Harbor harness tests did not pass: failed",
            result.issues,
        )

    def test_required_image_preflight_blocks_config_mismatch(self) -> None:
        summary = clean_summary()
        summary["checks"]["imagePreflight"]["config"] = "evals/harbor/tbench-smoke.json"

        result = self.module.validate_summary(
            summary,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("image preflight config does not match requested config", result.issues)

    def test_required_image_preflight_blocks_offline_mode_mismatch(self) -> None:
        summary = clean_summary()
        summary["options"]["offlineImages"] = True

        result = self.module.validate_summary(
            summary,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("image preflight did not run in offline mode", result.issues)

    def test_rejects_contradictory_image_preflight_options(self) -> None:
        summary = clean_summary()
        summary["options"]["offlineImages"] = True
        summary["options"]["pullImages"] = True

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn(
            "offlineImages cannot be combined with pullImages",
            result.issues,
        )

    def test_rejects_missing_pre_eval_options_check(self) -> None:
        summary = clean_summary()
        summary["checks"].pop("preEvalOptions", None)

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("preEvalOptions check missing", result.issues)

    def test_required_image_preflight_rejects_nonclean_details(self) -> None:
        summary = clean_summary()
        summary["checks"]["imagePreflight"]["missing"] = 1
        summary["checks"]["imagePreflight"]["unresolved"] = 1
        summary["checks"]["imagePreflight"]["pullFailed"] = 1
        summary["checks"]["imagePreflight"]["selectionErrors"] = [
            "unresolved task image"
        ]
        summary["checks"]["imagePreflight"]["blockedTasks"] = [
            {"taskName": "missing-image-task", "status": "missing"}
        ]

        result = self.module.validate_summary(
            summary,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight missing is 1", result.issues)
        self.assertIn("imagePreflight unresolved is 1", result.issues)
        self.assertIn("imagePreflight pullFailed is 1", result.issues)
        self.assertIn("imagePreflight selectionErrors are non-empty", result.issues)
        self.assertIn("imagePreflight blockedTasks are non-empty", result.issues)

    def test_required_image_preflight_requires_clean_detail_fields(self) -> None:
        summary = clean_summary()
        for field in (
            "missing",
            "unresolved",
            "pullFailed",
            "selectionErrors",
            "blockedTasks",
        ):
            summary["checks"]["imagePreflight"].pop(field, None)

        result = self.module.validate_summary(
            summary,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight missing is missing", result.issues)
        self.assertIn("imagePreflight unresolved is missing", result.issues)
        self.assertIn("imagePreflight pullFailed is missing", result.issues)
        self.assertIn("imagePreflight selectionErrors is missing", result.issues)
        self.assertIn("imagePreflight blockedTasks is missing", result.issues)

    def test_required_image_preflight_requires_manifest_and_count_fields(self) -> None:
        summary = clean_summary()
        for field in ("manifest", "tasks", "uniqueImages", "present"):
            summary["checks"]["imagePreflight"].pop(field, None)

        result = self.module.validate_summary(
            summary,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight manifest is missing", result.issues)
        self.assertIn("imagePreflight tasks is missing", result.issues)
        self.assertIn("imagePreflight uniqueImages is missing", result.issues)
        self.assertIn("imagePreflight present is missing", result.issues)

    def test_required_image_preflight_allows_shared_images(self) -> None:
        summary = clean_summary()
        summary["checks"]["imagePreflight"]["tasks"] = 2
        summary["checks"]["imagePreflight"]["uniqueImages"] = 1
        summary["checks"]["imagePreflight"]["present"] = 2

        result = self.module.validate_summary(
            summary,
            require_image_preflight=True,
        )

        self.assertTrue(result.ok)
        self.assertEqual([], result.issues)

    def test_required_image_preflight_rejects_inconsistent_counts(self) -> None:
        summary = clean_summary()
        summary["checks"]["imagePreflight"]["tasks"] = 0
        summary["checks"]["imagePreflight"]["uniqueImages"] = 2
        summary["checks"]["imagePreflight"]["present"] = 1

        result = self.module.validate_summary(
            summary,
            require_image_preflight=True,
        )

        self.assertFalse(result.ok)
        self.assertIn("imagePreflight tasks is not positive", result.issues)
        self.assertIn(
            "imagePreflight present exceeds tasks",
            result.issues,
        )
        self.assertIn("imagePreflight uniqueImages exceeds tasks", result.issues)

    def test_verify_harbor_configs_blocks_hash_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            config = Path(temp) / "tbench-full.json"
            config.write_text('{"job_name": "current"}\n')
            summary = clean_summary()
            summary["checks"]["harborConfigs"] = {
                "status": "passed",
                "issues": [],
                "entries": [
                    {
                        "path": str(config),
                        "sha256": "c" * 64,
                    }
                ],
            }

            result = self.module.validate_summary(
                summary,
                verify_harbor_configs=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("Harbor config SHA-256 mismatch", result.issues)

    def test_harbor_configs_requires_deadline_policy(self) -> None:
        summary = clean_summary()
        summary["checks"]["harborConfigs"].pop("deadlinePolicy")

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("harborConfigs deadlinePolicy is missing", result.issues)

    def test_harbor_configs_blocks_deadline_policy_mismatch(self) -> None:
        summary = clean_summary()
        summary["checks"]["harborConfigs"]["deadlinePolicy"] = {
            "overrideTimeoutSec": 900,
            "softTimeoutSec": 890,
            "evalDeadlineSeconds": 870,
        }

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        text = "\n".join(result.issues)
        self.assertIn("harborConfigs deadlinePolicy overrideTimeoutSec", text)
        self.assertIn("harborConfigs deadlinePolicy softTimeoutSec", text)
        self.assertIn("harborConfigs deadlinePolicy evalDeadlineSeconds", text)

    def test_verify_harbor_configs_blocks_missing_required_file_set(self) -> None:
        config = ROOT / "evals/harbor/tbench-full-gpt55-medium.json"
        digest = hashlib.sha256(config.read_bytes()).hexdigest()
        summary = clean_summary()
        summary["checks"]["harborConfigs"] = {
            "status": "passed",
            "issues": [],
            "entries": [
                {
                    "path": str(config),
                    "sha256": digest,
                }
            ],
        }

        result = self.module.validate_summary(
            summary,
            verify_harbor_configs=True,
        )

        self.assertFalse(result.ok)
        self.assertIn(
            "harborConfigs required file missing: evals/harbor/tbench-smoke.json",
            result.issues,
        )

    def test_verify_harbor_configs_blocks_missing_required_extra_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            route_config = Path(temp) / "route.json"
            route_config.write_text('{"job_name":"route"}\n')
            summary = clean_summary()
            summary["checks"]["harborConfigs"] = {
                "status": "passed",
                "issues": [],
                "entries": [
                    {
                        "path": str(path),
                        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                    }
                    for path in self.module.DEFAULT_CONFIGS
                ],
            }

            result = self.module.validate_summary(
                summary,
                verify_harbor_configs=True,
                required_configs=[route_config],
            )

        self.assertFalse(result.ok)
        self.assertIn(
            f"harborConfigs required file missing: {route_config}",
            result.issues,
        )

    def test_verify_harbor_configs_requires_image_preflight_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            image_config = Path(temp) / "image-route.json"
            image_config.write_text('{"job_name":"image-route"}\n')
            summary = clean_summary()
            summary["options"]["imageConfig"] = str(image_config)
            summary["checks"]["imagePreflight"]["config"] = str(image_config)
            summary["checks"]["harborConfigs"] = {
                "status": "passed",
                "issues": [],
                "entries": [
                    {
                        "path": str(path),
                        "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                    }
                    for path in self.module.DEFAULT_CONFIGS
                ],
            }

            result = self.module.validate_summary(
                summary,
                require_image_preflight=True,
                verify_harbor_configs=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            f"harborConfigs required file missing: {image_config}",
            result.issues,
        )

    def test_verify_prebuilt_binary_blocks_hash_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "roder-linux-amd64"
            binary.write_bytes(b"current")
            summary = clean_summary()
            summary["prebuiltBinary"] = {
                "required": True,
                "exists": True,
                "executable": True,
                "linuxX8664Elf": True,
                "path": str(binary),
                "sha256": "d" * 64,
            }

            result = self.module.validate_summary(
                summary,
                verify_prebuilt_binary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("prebuilt binary SHA-256 mismatch", result.issues)

    def test_verify_prebuilt_binary_blocks_non_executable_file(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "roder-linux-amd64"
            binary.write_bytes(b"same-content")
            binary.chmod(0o644)
            summary = clean_summary()
            summary["prebuiltBinary"] = {
                "required": True,
                "exists": True,
                "executable": True,
                "linuxX8664Elf": True,
                "path": str(binary),
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            }

            result = self.module.validate_summary(
                summary,
                verify_prebuilt_binary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("prebuilt binary is not executable", result.issues)

    def test_verify_prebuilt_binary_blocks_non_linux_x86_64_elf(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "roder-linux-amd64"
            binary.write_bytes(b"not an elf")
            binary.chmod(0o755)
            summary = clean_summary()
            summary["prebuiltBinary"] = {
                "required": True,
                "exists": True,
                "executable": True,
                "linuxX8664Elf": True,
                "path": str(binary),
                "sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
            }

            result = self.module.validate_summary(
                summary,
                verify_prebuilt_binary=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("prebuilt binary is not Linux x86-64 ELF", result.issues)

    def test_verify_auth_file_blocks_invalid_json(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            auth = Path(temp) / "codex.json"
            auth.write_text("{")
            summary = clean_summary()
            summary["authFile"] = {
                "required": True,
                "exists": True,
                "validJson": True,
                "path": str(auth),
            }

            result = self.module.validate_summary(
                summary,
                verify_auth_file=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("auth file JSON is invalid", result.issues)

    def test_verify_auth_file_blocks_missing_required_fields(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            auth = Path(temp) / "codex.json"
            auth.write_text('{"access":"token"}\n')
            summary = clean_summary()
            summary["authFile"] = {
                "required": True,
                "exists": True,
                "validJson": True,
                "path": str(auth),
            }

            result = self.module.validate_summary(
                summary,
                verify_auth_file=True,
            )

        self.assertFalse(result.ok)
        self.assertIn(
            "auth file missing required auth field(s): refresh, account_id, type",
            result.issues,
        )
        self.assertIn("auth file missing required auth field(s): expires", result.issues)

    def test_missing_harbor_harness_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"].pop("harborHarness", None)

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("harborHarness check missing", result.issues)

    def test_malformed_harbor_harness_blocks(self) -> None:
        summary = clean_summary()
        summary["checks"]["harborHarness"] = {
            "status": "failed",
            "files": 0,
            "issues": ["missing adapter"],
            "entries": [],
        }

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("harborHarness status is failed", result.issues)
        self.assertIn("harbor harness issues: missing adapter", result.issues)
        self.assertIn("harborHarness files is not positive", result.issues)
        self.assertIn("harborHarness combinedSha256 is missing", result.issues)
        self.assertIn("harborHarness entries are missing", result.issues)

    def test_verify_harbor_harness_files_blocks_hash_mismatch(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "roder_harbor_agent.py"
            path.write_text("current adapter\n")
            summary = clean_summary()
            summary["checks"]["harborHarness"] = {
                "status": "passed",
                "files": 1,
                "issues": [],
                "combinedSha256": "a" * 64,
                "entries": [
                    {
                        "path": str(path),
                        "sha256": "b" * 64,
                        "sizeBytes": len("previous adapter\n"),
                    }
                ],
            }

            result = self.module.validate_summary(
                summary,
                verify_harness_files=True,
            )

        self.assertFalse(result.ok)
        self.assertIn("Harbor harness file SHA-256 mismatch", result.issues)

    def test_verify_harbor_harness_files_blocks_missing_required_file_set(self) -> None:
        harness = ROOT / "evals/harbor/roder_harbor_agent.py"
        digest = hashlib.sha256(harness.read_bytes()).hexdigest()
        summary = clean_summary()
        summary["checks"]["harborHarness"] = {
            "status": "passed",
            "files": 1,
            "issues": [],
            "combinedSha256": combined_file_digest(
                [{"path": str(harness), "sha256": digest}]
            ),
            "entries": [
                {
                    "path": str(harness),
                    "sha256": digest,
                    "sizeBytes": harness.stat().st_size,
                }
            ],
        }

        result = self.module.validate_summary(
            summary,
            verify_harness_files=True,
        )

        self.assertFalse(result.ok)
        self.assertIn(
            "harborHarness required file missing: "
            "evals/harbor/pre_eval_image_preflight_validation.py",
            result.issues,
        )

    def test_blocked_summary_reports_blocked_checks(self) -> None:
        summary = clean_summary()
        summary["status"] = "blocked"
        summary["blockedChecks"] = ["harborAnalysisBaseline"]

        result = self.module.validate_summary(summary)

        self.assertFalse(result.ok)
        self.assertIn("summary status is blocked", result.issues)
        self.assertIn("blocked checks: harborAnalysisBaseline", result.issues)

    def test_stale_summary_blocks_when_max_age_is_set(self) -> None:
        summary = clean_summary()
        summary["generatedAt"] = "2026-05-25T10:00:00+00:00"

        result = self.module.validate_summary(
            summary,
            max_age_seconds=3600,
            now=datetime(2026, 5, 25, 12, 0, 1, tzinfo=timezone.utc),
        )

        self.assertFalse(result.ok)
        self.assertIn(
            "summary is stale: age 7201s exceeds max 3600s",
            result.issues,
        )


if __name__ == "__main__":
    unittest.main()
