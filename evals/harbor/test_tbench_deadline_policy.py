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


class TBenchDeadlinePolicyTests(unittest.TestCase):
    def test_deadline_policy_records_doubled_window(self) -> None:
        policy = load_module("tbench_deadline_policy")

        self.assertEqual(1800, policy.TBENCH_DEADLINE_POLICY.override_timeout_sec)
        self.assertEqual(1780, policy.TBENCH_DEADLINE_POLICY.soft_timeout_sec)
        self.assertEqual(1740, policy.TBENCH_DEADLINE_POLICY.eval_deadline_seconds)

    def test_readiness_and_campaign_validation_use_shared_deadline_policy(self) -> None:
        policy = load_module("tbench_deadline_policy")
        readiness = load_module("validate_harbor_readiness")
        campaign_readiness = load_module("tbench_campaign_route_readiness")

        self.assertIs(
            policy.TBENCH_DEADLINE_POLICY,
            readiness.TBENCH_DEADLINE_POLICY,
        )
        self.assertIs(
            policy.TBENCH_DEADLINE_POLICY,
            campaign_readiness.TBENCH_DEADLINE_POLICY,
        )


class DeriveTaskDeadlineLadderTests(unittest.TestCase):
    def setUp(self) -> None:
        self.policy = load_module("tbench_deadline_policy")

    def test_soft_and_deadline_sit_below_the_task_window(self) -> None:
        ladder = self.policy.derive_task_deadline_ladder(900, agent_timeout_multiplier=1.0)

        self.assertEqual(900, ladder.hard_timeout_sec)
        self.assertEqual(840, ladder.soft_timeout_sec)
        self.assertEqual(780, ladder.eval_deadline_seconds)
        self.assertLess(ladder.soft_timeout_sec, ladder.hard_timeout_sec)
        self.assertLess(ladder.eval_deadline_seconds, ladder.soft_timeout_sec)

    def test_long_task_window_is_restored_not_capped(self) -> None:
        ladder = self.policy.derive_task_deadline_ladder(3600, agent_timeout_multiplier=1.0)

        self.assertEqual(3540, ladder.soft_timeout_sec)
        self.assertEqual(3480, ladder.eval_deadline_seconds)

    def test_short_task_soft_still_precedes_hard_kill(self) -> None:
        # The 600s task is the case the clean run's x2 multiplier papered over:
        # a per-task soft timeout keeps roder inside the real window.
        ladder = self.policy.derive_task_deadline_ladder(600, agent_timeout_multiplier=1.0)

        self.assertLess(ladder.soft_timeout_sec, 600)
        self.assertLess(ladder.eval_deadline_seconds, ladder.soft_timeout_sec)

    def test_multiplier_scales_the_hard_window(self) -> None:
        ladder = self.policy.derive_task_deadline_ladder(900, agent_timeout_multiplier=2.0)

        self.assertEqual(1800, ladder.hard_timeout_sec)
        self.assertEqual(1740, ladder.soft_timeout_sec)

    def test_missing_or_invalid_timeout_returns_none(self) -> None:
        self.assertIsNone(self.policy.derive_task_deadline_ladder(None))
        self.assertIsNone(self.policy.derive_task_deadline_ladder(0))
        self.assertIsNone(self.policy.derive_task_deadline_ladder(-5))


if __name__ == "__main__":
    unittest.main()
