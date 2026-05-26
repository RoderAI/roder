#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
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


class PreEvalOptionsSummaryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.module = load_module()

    def build_summary(self, root: Path, *, preflight_images: bool) -> dict:
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
            preflight_images=preflight_images,
            pull_images=False,
            image_config="evals/harbor/tbench-full-gpt55-medium.json",
            analysis_target="",
            analysis_baseline="",
            prebuilt_binary=root / "missing-roder",
            auth_file=root / "codex.json",
            require_auth=False,
            image_manifest=None,
        )

    def test_skipped_image_preflight_does_not_record_image_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(Path(temp), preflight_images=False)

            self.assertFalse(summary["options"]["preflightImages"])
            self.assertIsNone(summary["options"]["imageConfig"])

    def test_enabled_image_preflight_records_image_config(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            summary = self.build_summary(Path(temp), preflight_images=True)

            self.assertTrue(summary["options"]["preflightImages"])
            self.assertEqual(
                "evals/harbor/tbench-full-gpt55-medium.json",
                summary["options"]["imageConfig"],
            )


if __name__ == "__main__":
    unittest.main()
