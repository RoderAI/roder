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


if __name__ == "__main__":
    unittest.main()
