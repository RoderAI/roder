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


class PolicyBlockRetryTests(unittest.TestCase):
    def setUp(self) -> None:
        self.retry = load_module("roder_policy_block_retry")

    def test_check_command_requires_block_marker_and_no_tool_calls(self) -> None:
        command = self.retry.policy_block_check_command(
            events_path="/logs/agent/roder-events.jsonl",
            stderr_path="/logs/agent/roder-stderr.txt",
        )
        self.assertIn(self.retry.POLICY_BLOCK_MARKER, command)
        self.assertIn("roder-stderr.txt", command)
        # A tool-call in events must veto the retry.
        self.assertIn('"tool_name"', command)
        self.assertIn("! grep", command)

    def test_budget_available_early_in_the_window(self) -> None:
        budget = self.retry.policy_block_retry_budget_sec(
            soft_timeout_sec=780, elapsed_sec=20
        )
        self.assertIsNotNone(budget)
        self.assertLess(budget, 780)

    def test_no_budget_when_window_nearly_spent(self) -> None:
        self.assertIsNone(
            self.retry.policy_block_retry_budget_sec(
                soft_timeout_sec=780, elapsed_sec=700
            )
        )

    def test_no_budget_without_soft_timeout(self) -> None:
        self.assertIsNone(
            self.retry.policy_block_retry_budget_sec(
                soft_timeout_sec=None, elapsed_sec=1
            )
        )


if __name__ == "__main__":
    unittest.main()
