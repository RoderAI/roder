#!/usr/bin/env python3

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))

from analyze_tbench_run import analyze_job  # noqa: E402


class AnalyzeTBenchRunTests(unittest.TestCase):
    def test_provider_runtime_failures_are_clean_run_errors(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            job_dir = Path(temp) / "job"
            job_dir.mkdir()
            (job_dir / "result.json").write_text(
                json.dumps({"stats": {"n_trials": 2}}) + "\n"
            )
            write_trial(
                job_dir,
                "stream-task__abc123",
                provider_error_kind="stream_incomplete",
                exit_status=0,
            )
            write_trial(
                job_dir,
                "bad-status-task__def456",
                provider_error_kind=None,
                exit_status=2,
            )

            analysis = analyze_job(job_dir)

        self.assertFalse(analysis["clean"])
        self.assertEqual(
            {
                "provider_stream_incomplete": 1,
                "roder_exec_error_status": 1,
            },
            analysis["stats"]["harness_error_classes"],
        )


def write_trial(
    job_dir: Path,
    trial_name: str,
    *,
    provider_error_kind: str | None,
    exit_status: int,
) -> None:
    trial_dir = job_dir / trial_name
    agent_dir = trial_dir / "agent"
    agent_dir.mkdir(parents=True)
    task_name = trial_name.split("__", 1)[0]
    (trial_dir / "result.json").write_text(
        json.dumps(
            {
                "trial_name": trial_name,
                "task_name": task_name,
                "verifier_result": {"rewards": {"reward": 0.0}},
            }
        )
        + "\n"
    )
    (trial_dir / "config.json").write_text("{}\n")
    run_summary = {"exit_status": exit_status}
    if provider_error_kind is not None:
        run_summary["provider_error_kind"] = provider_error_kind
    (agent_dir / "roder-run-summary.json").write_text(
        json.dumps(run_summary) + "\n"
    )


if __name__ == "__main__":
    unittest.main()
