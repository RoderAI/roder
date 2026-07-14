#!/usr/bin/env python3

from __future__ import annotations

import importlib
import sys
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR = ROOT / "evals/harbor"


def load_module(name: str):
    if str(HARBOR) not in sys.path:
        sys.path.insert(0, str(HARBOR))
    return importlib.import_module(name)


class TerminalBenchGuidanceTests(unittest.TestCase):
    def setUp(self) -> None:
        self.guidance = load_module("roder_benchmark_guidance").TERMINAL_BENCH_GUIDANCE

    def test_completion_hygiene_bullets_present(self) -> None:
        # Generic, evaluation-neutral finalization guidance addressing the
        # false-completion and invariance/no-op failure families.
        self.assertIn("already-valid or benign inputs unchanged", self.guidance)
        self.assertIn("strongest local validation", self.guidance)
        self.assertIn("provisional", self.guidance)

    def test_guidance_stays_task_agnostic(self) -> None:
        # The guidance must never name a specific benchmark task or embed a
        # scoreable answer; it should read as generic harness advice.
        lowered = self.guidance.lower()
        for banned in ("__", "reward.txt", "ctrf.json", "verifier passes if"):
            self.assertNotIn(banned, lowered)


if __name__ == "__main__":
    unittest.main()
