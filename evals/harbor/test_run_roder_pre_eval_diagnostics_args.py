#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "evals/harbor/run-roder-pre-eval-diagnostics.sh"


class PreEvalDiagnosticsArgsTests(unittest.TestCase):
    def test_help_describes_skip_tests_as_all_local_test_gates(self) -> None:
        result = subprocess.run(
            [str(SCRIPT), "--help"],
            cwd=ROOT,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn(
            "--skip-tests     skip Harbor Python and roder-evals unit test gates",
            result.stdout,
        )

    def test_pull_images_requires_image_preflight(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--pull-images",
                    "--skip-tests",
                    "--output-dir",
                    str(Path(temp) / "diagnostics"),
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(2, result.returncode)
        self.assertIn("--pull-images requires --preflight-images", result.stderr)
        self.assertNotIn("cargo run", result.stdout)

    def test_analysis_baseline_requires_analysis_target(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--analysis-baseline",
                    str(Path(temp) / "baseline.json"),
                    "--skip-tests",
                    "--output-dir",
                    str(Path(temp) / "diagnostics"),
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(2, result.returncode)
        self.assertIn("--analysis-baseline requires --analysis", result.stderr)
        self.assertNotIn("cargo run", result.stdout)

    def test_image_config_requires_image_preflight(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--image-config",
                    str(Path(temp) / "tbench.json"),
                    "--skip-tests",
                    "--output-dir",
                    str(Path(temp) / "diagnostics"),
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(2, result.returncode)
        self.assertIn("--image-config requires --preflight-images", result.stderr)
        self.assertNotIn("cargo run", result.stdout)

    def test_auth_file_requires_auth_gate(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--auth-file",
                    str(Path(temp) / "codex.json"),
                    "--skip-tests",
                    "--output-dir",
                    str(Path(temp) / "diagnostics"),
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(2, result.returncode)
        self.assertIn("--auth-file requires --require-auth", result.stderr)
        self.assertNotIn("cargo run", result.stdout)

    def test_config_requires_value(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--config",
                    "--skip-tests",
                    "--output-dir",
                    str(Path(temp) / "diagnostics"),
                ],
                cwd=ROOT,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

        self.assertEqual(2, result.returncode)
        self.assertIn("--config requires a value", result.stderr)
        self.assertNotIn("cargo run", result.stdout)

    def test_image_config_is_recorded_as_harbor_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text(
                "#!/usr/bin/env bash\n"
                "printf '%s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_python.chmod(0o755)
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'cargo %s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_cargo.chmod(0o755)
            image_config = temp_path / "route.json"
            image_config.write_text("{}\n")
            call_log = temp_path / "calls.log"
            env = {
                **dict(PATH=f"{fake_bin}:{os.environ['PATH']}"),
                "CALL_LOG": str(call_log),
                "HOME": str(temp_path),
            }

            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--preflight-images",
                    "--image-config",
                    str(image_config),
                    "--skip-tests",
                    "--output-dir",
                    str(temp_path / "diagnostics"),
                ],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            calls = call_log.read_text()

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn(
            f"validate_harbor_readiness.py --config evals/harbor/tbench-full-gpt55-medium.json --config evals/harbor/tbench-smoke.json --config {image_config}",
            calls,
        )
        self.assertIn(
            f"write_pre_eval_summary.py --summary {temp_path / 'diagnostics/pre-eval-summary.json'}",
            calls,
        )
        self.assertIn(f"--config {image_config}", calls)
        preflight_call = next(
            line for line in calls.splitlines() if "preflight_tbench_images.py" in line
        )
        self.assertNotIn("--offline", preflight_call)

    def test_offline_images_keeps_local_only_image_preflight(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text(
                "#!/usr/bin/env bash\n"
                "printf '%s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_python.chmod(0o755)
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'cargo %s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_cargo.chmod(0o755)
            call_log = temp_path / "calls.log"
            env = {
                **dict(PATH=f"{fake_bin}:{os.environ['PATH']}"),
                "CALL_LOG": str(call_log),
                "HOME": str(temp_path),
            }

            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--preflight-images",
                    "--offline-images",
                    "--skip-tests",
                    "--output-dir",
                    str(temp_path / "diagnostics"),
                ],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            calls = call_log.read_text()

        self.assertEqual(0, result.returncode, result.stderr)
        preflight_call = next(
            line for line in calls.splitlines() if "preflight_tbench_images.py" in line
        )
        self.assertIn("--offline", preflight_call)
        summary_call = next(
            line for line in calls.splitlines() if "write_pre_eval_summary.py" in line
        )
        self.assertIn("--offline-images", summary_call)

    def test_default_tests_run_harbor_python_unittest_gate(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'python3 %s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_python.chmod(0o755)
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'cargo %s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_cargo.chmod(0o755)
            call_log = temp_path / "calls.log"
            env = {
                **dict(PATH=f"{fake_bin}:{os.environ['PATH']}"),
                "CALL_LOG": str(call_log),
                "HOME": str(temp_path),
            }

            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--output-dir",
                    str(temp_path / "diagnostics"),
                ],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            calls = call_log.read_text()

        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn(
            "python3 -m unittest discover -s evals/harbor -p test_*.py",
            calls,
        )

    def test_harbor_python_tests_do_not_inherit_wrapper_dry_run_env(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'python3 %s DRY=%s\\n' \"$*\" \"${RODER_HARBOR_DRY_RUN-unset}\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_python.chmod(0o755)
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'cargo %s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_cargo.chmod(0o755)
            call_log = temp_path / "calls.log"
            env = {
                **dict(PATH=f"{fake_bin}:{os.environ['PATH']}"),
                "CALL_LOG": str(call_log),
                "HOME": str(temp_path),
                "RODER_HARBOR_DRY_RUN": "1",
            }

            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--output-dir",
                    str(temp_path / "diagnostics"),
                ],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            calls = call_log.read_text()

        self.assertEqual(0, result.returncode, result.stderr)
        unittest_call = next(
            line for line in calls.splitlines() if "-m unittest discover" in line
        )
        self.assertIn("DRY=unset", unittest_call)

    def test_harbor_python_tests_do_not_inherit_wrapper_run_control_env(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'python3 %s LIVE=%s REPLACE=%s SKIP=%s\\n' \"$*\" "
                "\"${RODER_HARBOR_LIVE_TBENCH-unset}\" "
                "\"${RODER_HARBOR_REPLACE_JOB-unset}\" "
                "\"${RODER_HARBOR_SKIP_PREFLIGHT-unset}\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_python.chmod(0o755)
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'cargo %s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_cargo.chmod(0o755)
            call_log = temp_path / "calls.log"
            env = {
                **dict(PATH=f"{fake_bin}:{os.environ['PATH']}"),
                "CALL_LOG": str(call_log),
                "HOME": str(temp_path),
                "RODER_HARBOR_LIVE_TBENCH": "1",
                "RODER_HARBOR_REPLACE_JOB": "1",
                "RODER_HARBOR_SKIP_PREFLIGHT": "1",
            }

            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--output-dir",
                    str(temp_path / "diagnostics"),
                ],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            calls = call_log.read_text()

        self.assertEqual(0, result.returncode, result.stderr)
        unittest_call = next(
            line for line in calls.splitlines() if "-m unittest discover" in line
        )
        self.assertIn("LIVE=unset", unittest_call)
        self.assertIn("REPLACE=unset", unittest_call)
        self.assertIn("SKIP=unset", unittest_call)

    def test_harbor_python_tests_do_not_inherit_pre_eval_handoff_env(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'python3 %s SUMMARY=%s CAMPAIGN=%s ANALYSIS=%s\\n' \"$*\" "
                "\"${RODER_HARBOR_PRE_EVAL_SUMMARY-unset}\" "
                "\"${RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY-unset}\" "
                "\"${RODER_HARBOR_PRE_EVAL_ANALYSIS-unset}\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_python.chmod(0o755)
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'cargo %s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_cargo.chmod(0o755)
            call_log = temp_path / "calls.log"
            env = {
                **dict(PATH=f"{fake_bin}:{os.environ['PATH']}"),
                "CALL_LOG": str(call_log),
                "HOME": str(temp_path),
                "RODER_HARBOR_PRE_EVAL_SUMMARY": str(temp_path / "summary.json"),
                "RODER_HARBOR_PRE_EVAL_CAMPAIGN_SUMMARY": str(
                    temp_path / "campaign-summary.json"
                ),
                "RODER_HARBOR_PRE_EVAL_ANALYSIS": str(temp_path / "analysis.json"),
            }

            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--output-dir",
                    str(temp_path / "diagnostics"),
                ],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            calls = call_log.read_text()

        self.assertEqual(0, result.returncode, result.stderr)
        unittest_call = next(
            line for line in calls.splitlines() if "-m unittest discover" in line
        )
        self.assertIn("SUMMARY=unset", unittest_call)
        self.assertIn("CAMPAIGN=unset", unittest_call)
        self.assertIn("ANALYSIS=unset", unittest_call)

    def test_failed_harbor_python_tests_write_blocked_summary(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'python3 %s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "if [[ \"$*\" == '-m unittest discover -s evals/harbor -p test_*.py' ]]; then\n"
                "  exit 7\n"
                "fi\n"
                "exec /usr/bin/python3 \"$@\"\n"
            )
            fake_python.chmod(0o755)
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/usr/bin/env bash\n"
                "printf 'cargo %s\\n' \"$*\" >> \"$CALL_LOG\"\n"
                "exit 0\n"
            )
            fake_cargo.chmod(0o755)
            call_log = temp_path / "calls.log"
            diagnostics_dir = temp_path / "diagnostics"
            env = {
                **dict(PATH=f"{fake_bin}:{os.environ['PATH']}"),
                "CALL_LOG": str(call_log),
                "HOME": str(temp_path),
            }

            result = subprocess.run(
                [
                    str(SCRIPT),
                    "--output-dir",
                    str(diagnostics_dir),
                ],
                cwd=ROOT,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )
            summary = json.loads((diagnostics_dir / "pre-eval-summary.json").read_text())
            calls = call_log.read_text()

        self.assertEqual(7, result.returncode)
        self.assertIn(
            "python3 -m unittest discover -s evals/harbor -p test_*.py",
            calls,
        )
        self.assertNotIn("cargo test -p roder-evals --lib", calls)
        self.assertEqual("blocked", summary["status"])
        self.assertEqual("failed", summary["checks"]["harborHarnessTests"]["status"])
        self.assertEqual("not_run", summary["checks"]["roderEvalsLib"]["status"])
        self.assertEqual(
            "python3 -m unittest discover -s evals/harbor -p test_*.py",
            summary["failure"]["step"],
        )
        self.assertEqual(7, summary["failure"]["exitCode"])


if __name__ == "__main__":
    unittest.main()
