#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from run_roder_tbench_full_test_helpers import clean_summary  # noqa: E402

SCRIPT = ROOT / "evals/harbor/run-roder-tbench-smoke.sh"


class RunRoderTbenchSmokeGateTests(unittest.TestCase):
    def test_dry_run_validates_smoke_summary_without_live_harbor(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            summary = temp_path / "pre-eval-summary.json"
            prebuilt = temp_path / "roder-linux-amd64"
            data = clean_summary(preflight_images=True, prebuilt_binary=prebuilt)
            rewrite_image_preflight_for_smoke(data, temp_path / "smoke-images.json")
            summary.write_text(json.dumps(data, indent=2) + "\n")

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
        self.assertIn("Smoke dry-run passed", result.stdout)
        self.assertNotIn("harbor should not run", result.stderr)

    def test_dry_run_rejects_reused_full_image_preflight_summary(self) -> None:
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

            env = os.environ.copy()
            env.pop("RODER_HARBOR_LIVE_TBENCH", None)
            env.update(
                {
                    "RODER_HARBOR_DRY_RUN": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(summary),
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
        self.assertIn(
            "image preflight config does not match required config",
            result.stderr,
        )
        self.assertNotIn("Smoke dry-run passed", result.stdout)

    def test_live_run_blocks_existing_job_dir_without_replace(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            temp_path = Path(temp)
            repo = temp_path / "repo"
            script = copy_smoke_script(repo)
            job_dir = repo / "evals/harbor/jobs/roder-tbench-smoke"
            job_dir.mkdir(parents=True)
            sentinel = job_dir / "result.json"
            sentinel.write_text('{"existing":true}\n')

            fake_bin = temp_path / "bin"
            fake_bin.mkdir()
            fake_python = fake_bin / "python3"
            fake_python.write_text("#!/usr/bin/env bash\nexit 0\n")
            fake_python.chmod(fake_python.stat().st_mode | stat.S_IXUSR)
            fake_harbor = fake_bin / "harbor"
            fake_harbor.write_text("#!/usr/bin/env bash\necho harbor should not run >&2\nexit 99\n")
            fake_harbor.chmod(fake_harbor.stat().st_mode | stat.S_IXUSR)

            env = os.environ.copy()
            env.update(
                {
                    "PATH": f"{fake_bin}:{env.get('PATH', '')}",
                    "RODER_HARBOR_LIVE_TBENCH": "1",
                    "RODER_HARBOR_SKIP_PREFLIGHT": "1",
                    "RODER_HARBOR_PRE_EVAL_SUMMARY": str(temp_path / "summary.json"),
                    "RODER_HARBOR_PREBUILT_BINARY": str(temp_path / "roder"),
                }
            )

            result = subprocess.run(
                [str(script)],
                cwd=repo,
                env=env,
                text=True,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=False,
            )

            self.assertEqual(2, result.returncode)
            self.assertTrue(sentinel.exists())
            self.assertIn("already exists", result.stderr)
            self.assertIn("RODER_HARBOR_REPLACE_JOB=1", result.stderr)
            self.assertNotIn("harbor should not run", result.stderr)


def rewrite_image_preflight_for_smoke(summary: dict, manifest: Path) -> None:
    manifest.write_text(
        json.dumps(
            {
                "clean": True,
                "config": "evals/harbor/tbench-smoke.json",
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
                        "task_name": "break-filter-js-from-html",
                        "status": "present",
                        "image": "example/smoke:latest",
                    }
                ],
                "images": [
                    {
                        "image": "example/smoke:latest",
                        "tasks": ["break-filter-js-from-html"],
                    }
                ],
            },
            indent=2,
        )
        + "\n"
    )
    summary["options"]["imageConfig"] = "evals/harbor/tbench-smoke.json"
    summary["checks"]["imagePreflight"].update(
        {
            "config": "evals/harbor/tbench-smoke.json",
            "manifest": str(manifest),
            "tasks": 1,
            "uniqueImages": 1,
            "present": 1,
        }
    )


def copy_smoke_script(repo: Path) -> Path:
    script = repo / "evals/harbor/run-roder-tbench-smoke.sh"
    script.parent.mkdir(parents=True)
    script.write_text(SCRIPT.read_text())
    script.chmod(script.stat().st_mode | stat.S_IXUSR)
    return script


if __name__ == "__main__":
    unittest.main()
