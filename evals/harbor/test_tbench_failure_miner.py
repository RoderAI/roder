#!/usr/bin/env python3
from __future__ import annotations

import sys
import unittest
from pathlib import Path

HARBOR_DIR = Path(__file__).resolve().parent
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

import tbench_route_test_helpers as fixtures  # noqa: E402
from tbench_failure_miner import (  # noqa: E402
    build_failure_records,
    clean_run_fail_tasks,
    clean_run_pass_tasks,
    redact,
)
from tbench_route_constants import normalize_task_name  # noqa: E402


class FailureMinerTest(unittest.TestCase):
    def setUp(self) -> None:
        self.analysis = fixtures.analysis()
        self.comparison = fixtures.comparison()
        self.miner = fixtures.miner_evidence()

    def _records_by_task(self, **kwargs):
        records = build_failure_records(
            analysis=self.analysis,
            comparison=self.comparison,
            miner_evidence=self.miner,
            **kwargs,
        )
        return {record.task: record for record in records}

    def test_pass_and_fail_task_sets(self) -> None:
        self.assertEqual(clean_run_pass_tasks(self.analysis), {"extract-elf", "fix-git"})
        self.assertEqual(len(clean_run_fail_tasks(self.analysis)), len(fixtures.FAIL_TASKS))
        self.assertNotIn("extract-elf", clean_run_fail_tasks(self.analysis))

    def test_route_assignment_from_miner_evidence(self) -> None:
        by_task = self._records_by_task()
        self.assertEqual(by_task["crack-7z-hash"].route, "deadline-extension")
        self.assertEqual(by_task["pytorch-model-recovery"].route, "task-contract")
        self.assertEqual(by_task["model-extraction-relu-logits"].route, "policy-framed")
        self.assertEqual(by_task["headless-terminal"].route, "historical-regression")
        self.assertEqual(by_task["qemu-startup"].route, "environment-service")
        self.assertEqual(by_task["make-doom-for-mips"].route, "capability")

    def test_descriptive_task_name_normalizes_and_attaches_evidence(self) -> None:
        # The miner record's task is "crack-7z-hash: recover ..."; it must still attach.
        by_task = self._records_by_task()
        record = by_task["crack-7z-hash"]
        self.assertTrue(record.has_miner_evidence)
        self.assertTrue(record.near_miss)
        self.assertEqual(record.task_timeout_sec, 900)

    def test_regression_flag_from_comparison(self) -> None:
        by_task = self._records_by_task()
        self.assertTrue(by_task["crack-7z-hash"].regression)
        self.assertTrue(by_task["headless-terminal"].regression)
        self.assertFalse(by_task["regex-chess"].regression)
        self.assertFalse(by_task["pytorch-model-recovery"].regression)

    def test_near_miss_and_timeout_fields(self) -> None:
        by_task = self._records_by_task()
        self.assertFalse(by_task["regex-chess"].near_miss)
        self.assertEqual(by_task["regex-chess"].task_timeout_sec, 3600)
        self.assertTrue(by_task["pytorch-model-recovery"].near_miss)

    def test_fallback_route_from_classes_without_evidence(self) -> None:
        records = build_failure_records(
            analysis=self.analysis, comparison=self.comparison, miner_evidence=None
        )
        by_task = {record.task: record for record in records}
        # crack-7z-hash carries deadline classes -> deadline-extension
        self.assertEqual(by_task["crack-7z-hash"].route, "deadline-extension")
        # model-extraction carries provider_policy_block -> policy-framed
        self.assertEqual(by_task["model-extraction-relu-logits"].route, "policy-framed")
        # make-doom-for-mips only scored_fail -> capability (no harness fix bucket)
        self.assertEqual(by_task["make-doom-for-mips"].route, "capability")
        self.assertFalse(by_task["crack-7z-hash"].has_miner_evidence)

    def test_evidence_is_redacted(self) -> None:
        by_task = self._records_by_task()
        joined = " ".join(by_task["crack-7z-hash"].evidence)
        # The evidence strings themselves carry no secret, but redaction must run.
        self.assertNotIn("sk-ABCDEF1234567890", joined)

    def test_redact_scrubs_tokens(self) -> None:
        self.assertNotIn("sk-ABCDEF1234567890", redact("token: sk-ABCDEF1234567890"))
        self.assertIn("<redacted", redact("api_key=supersecretvalue123"))

    def test_normalize_task_name_variants(self) -> None:
        self.assertEqual(normalize_task_name("terminal-bench/regex-chess"), "regex-chess")
        self.assertEqual(normalize_task_name("sam-cell-seg: implement /app/convert.py"), "sam-cell-seg")
        self.assertEqual(
            normalize_task_name("dna-assembly (Terminal-Bench 2.1) - Golden Gate"), "dna-assembly"
        )


if __name__ == "__main__":
    unittest.main()
