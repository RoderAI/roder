#!/usr/bin/env python3

from __future__ import annotations

import importlib
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR = ROOT / "evals/harbor"


def load_module(name: str):
    if str(HARBOR) not in sys.path:
        sys.path.insert(0, str(HARBOR))
    return importlib.import_module(name)


class TaskNameFromLogsDirTests(unittest.TestCase):
    def setUp(self) -> None:
        self.windows = load_module("tbench_task_windows")

    def test_parses_task_name_from_trial_agent_dir(self) -> None:
        name = self.windows.task_name_from_logs_dir(
            "/jobs/roder-tbench/compile-compcert__aB3xQ/agent"
        )
        self.assertEqual("compile-compcert", name)

    def test_hyphenated_task_names_are_preserved(self) -> None:
        name = self.windows.task_name_from_logs_dir(
            "/jobs/j/break-filter-js-from-html__ZZ9/agent"
        )
        self.assertEqual("break-filter-js-from-html", name)

    def test_missing_suffix_returns_none(self) -> None:
        self.assertIsNone(self.windows.task_name_from_logs_dir("/jobs/j/plain/agent"))
        self.assertIsNone(self.windows.task_name_from_logs_dir(None))


class LookupTaskAgentTimeoutTests(unittest.TestCase):
    def setUp(self) -> None:
        self.windows = load_module("tbench_task_windows")

    def _write_task(self, cache: Path, cache_id: str, task: str, timeout: object) -> None:
        task_dir = cache / cache_id / task
        task_dir.mkdir(parents=True, exist_ok=True)
        body = "[agent]\n"
        if timeout is not None:
            body += f"timeout_sec = {timeout}\n"
        (task_dir / "task.toml").write_text(body)

    def _write_canonical(
        self, cache: Path, task: str, timeout: object, dataset: str = "terminal-bench", content_hash: str = "abc123"
    ) -> None:
        task_dir = cache / "packages" / dataset / task / content_hash
        task_dir.mkdir(parents=True, exist_ok=True)
        (task_dir / "task.toml").write_text(f"[agent]\ntimeout_sec = {timeout}\n")

    def test_reads_declared_agent_timeout(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            self._write_task(cache, "aaa", "compile-compcert", 2400.0)
            self.assertEqual(
                2400.0,
                self.windows.lookup_task_agent_timeout_sec(
                    "compile-compcert", cache_dir=cache
                ),
            )

    def test_unknown_task_returns_none(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            self._write_task(cache, "aaa", "known", 900)
            self.assertIsNone(
                self.windows.lookup_task_agent_timeout_sec("missing", cache_dir=cache)
            )

    def test_missing_cache_dir_returns_none(self) -> None:
        self.assertIsNone(
            self.windows.lookup_task_agent_timeout_sec(
                "x", cache_dir=Path("/no/such/cache")
            )
        )

    def test_canonical_packages_copy_wins_over_stale_per_job_snapshot(self) -> None:
        # Regression: the per-job snapshot (<jobhash>/<task>/task.toml) can carry a
        # stale/older-version shorter window; the canonical packages/ copy holds the
        # real Terminal-Bench window Harbor enforces. Even when the stale copy has a
        # newer mtime, the canonical copy must win.
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            self._write_canonical(cache, "crack-7z-hash", 1800.0)
            self._write_task(cache, "MAyhSNHCZegJvZuPyTitjJ", "crack-7z-hash", 900.0)
            # Make the stale per-job snapshot the newest by mtime.
            import os
            stale = cache / "MAyhSNHCZegJvZuPyTitjJ" / "crack-7z-hash" / "task.toml"
            os.utime(stale, (10_000_000_000, 10_000_000_000))
            self.assertEqual(
                1800.0,
                self.windows.lookup_task_agent_timeout_sec(
                    "crack-7z-hash", cache_dir=cache
                ),
            )

    def test_falls_back_to_per_job_snapshot_when_no_canonical_copy(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            self._write_task(cache, "jobhash1", "some-task", 1200.0)
            self.assertEqual(
                1200.0,
                self.windows.lookup_task_agent_timeout_sec("some-task", cache_dir=cache),
            )

    def test_non_positive_or_absent_timeout_is_ignored(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            self._write_task(cache, "aaa", "zerotask", 0)
            self._write_task(cache, "bbb", "notimeout", None)
            self.assertIsNone(
                self.windows.lookup_task_agent_timeout_sec("zerotask", cache_dir=cache)
            )
            self.assertIsNone(
                self.windows.lookup_task_agent_timeout_sec("notimeout", cache_dir=cache)
            )


if __name__ == "__main__":
    unittest.main()
