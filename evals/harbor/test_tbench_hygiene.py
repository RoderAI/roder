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
from tbench_hygiene import hygiene_labels, scan_trial_features  # noqa: E402
from tbench_trial import load_trials  # noqa: E402


class TrajectoryFeatureTests(unittest.TestCase):
    def test_counts_model_tool_and_verification_activity(self) -> None:
        events = [
            _thread(),
            _reasoning(),
            _assistant_segment(),
            _tool("call_1", "shell", "echo hi > out.txt", "completed"),
            _assistant_segment(),
            _tool("call_2", "shell", "missing-binary", "failed"),
            _tool("call_3", "verification_review", "", "completed", payload={"testsRun": ["pytest"]}),
            _turn_done(),
        ]
        trial = _load_one(events, reward=0.0)
        features = scan_trial_features(trial)

        self.assertEqual(3, features.tool_calls)
        self.assertEqual(1, features.failed_tools)
        self.assertEqual(1, features.verification_reviews)
        self.assertEqual(2, features.model_calls)
        self.assertIn("shell", features.tool_failures)

    def test_validation_after_write_clears_no_local_validation(self) -> None:
        events = [
            _thread(),
            _tool("call_1", "apply_patch", "", "completed", payload={"patch": "*** Update File: out.py"}),
            _tool("call_2", "shell", "pytest -q", "completed"),
            _turn_done(),
        ]
        trial = _load_one(events, reward=0.0)
        features = scan_trial_features(trial)
        self.assertTrue(features.has_validation_after_last_write)
        self.assertNotIn("no_local_validation", hygiene_labels(trial, features))

    def test_write_without_validation_flags_no_local_validation(self) -> None:
        events = [
            _thread(),
            _tool("call_1", "shell", "echo answer > /app/out.txt", "completed"),
            _tool("call_2", "shell", "cat /app/out.txt", "completed"),
            _turn_done(),
        ]
        trial = _load_one(events, reward=0.0)
        features = scan_trial_features(trial)
        self.assertFalse(features.has_validation_after_last_write)
        self.assertIn("no_local_validation", hygiene_labels(trial, features))


class HygieneLabelTests(unittest.TestCase):
    def test_provisional_final_message(self) -> None:
        events = [_thread(), _tool("c1", "shell", "true", "completed"), _turn_done()]
        trial = _load_one(
            events,
            reward=0.0,
            last_message="Wrote a provisional answer before the deadline.",
        )
        labels = hygiene_labels(trial, scan_trial_features(trial))
        self.assertIn("provisional_final_message", labels)

    def test_failed_last_tool(self) -> None:
        events = [_thread(), _tool("c1", "shell", "boom", "failed"), _turn_done()]
        trial = _load_one(
            events,
            reward=0.0,
            run_summary_extra={"last_tool": {"tool_name": "shell", "status": "failed"}},
        )
        self.assertIn("failed_last_tool", hygiene_labels(trial, scan_trial_features(trial)))

    def test_long_command_still_running(self) -> None:
        events = [_thread(), _tool("c1", "shell", "sleep 999", "completed"), _turn_done()]
        trial = _load_one(
            events,
            reward=0.0,
            run_summary_extra={"active_tool": {"tool_name": "shell", "status": "running"}},
        )
        self.assertIn("long_command_still_running", hygiene_labels(trial, scan_trial_features(trial)))

    def test_final_answer_only_when_no_successful_tool(self) -> None:
        events = [
            _thread(),
            _tool("c1", "shell", "bad", "failed"),
            _tool("c2", "shell", "bad", "failed"),
            _turn_done(),
        ]
        trial = _load_one(events, reward=0.0)
        self.assertIn("final_answer_only", hygiene_labels(trial, scan_trial_features(trial)))

    def test_no_final_answer_only_when_last_tool_succeeds(self) -> None:
        events = [
            _thread(),
            _tool("c1", "apply_patch", "", "completed", payload={"patch": "*** Update File: out.py"}),
            _tool("c2", "shell", "pytest -q", "completed"),
            _turn_done(),
        ]
        trial = _load_one(events, reward=0.0)
        self.assertNotIn("final_answer_only", hygiene_labels(trial, scan_trial_features(trial)))


class AnalyzeJobHygieneTests(unittest.TestCase):
    def test_analysis_includes_hygiene_summary_and_rankings(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            job_dir = Path(temp) / "job"
            job_dir.mkdir()
            (job_dir / "result.json").write_text(json.dumps({"stats": {"n_trials": 2}}) + "\n")
            _write_trial(
                job_dir,
                "provisional__a1",
                [_thread(), _tool("c1", "shell", "echo x > o.txt", "completed"), _turn_done()],
                reward=0.0,
                last_message="Best provisional output before the deadline.",
                run_summary_extra={"deadline_finalized": True, "elapsed_seconds": 700},
            )
            _write_trial(
                job_dir,
                "clean-pass__b1",
                [
                    _thread(),
                    _tool("c1", "apply_patch", "", "completed", payload={"patch": "*** Update File: x"}),
                    _tool("c2", "shell", "pytest -q", "completed"),
                    _turn_done(),
                ],
                reward=1.0,
                last_message="All tests pass.",
            )
            analysis = analyze_job(job_dir)

        hygiene = analysis["hygiene"]
        self.assertEqual(1, hygiene["provisional_final_message"]["count"])
        self.assertEqual(1, hygiene["no_local_validation"]["count"])
        rankings = analysis["failed_trial_rankings"]
        deadline_tasks = [entry["task_name"] for entry in rankings["deadline_burn"]]
        self.assertIn("provisional", deadline_tasks)
        # passing trial is never ranked as a failure
        self.assertNotIn("clean-pass", deadline_tasks)
        self.assertIn("provisional_final_message", analysis["trial_hygiene"]["provisional__a1"])


# --- synthetic event + trial builders ---


def _thread() -> dict:
    return {"type": "thread.started", "thread_id": "thread"}


def _turn_done() -> dict:
    return {"type": "turn.completed", "usage": {"input_tokens": 10, "output_tokens": 2}}


def _reasoning() -> dict:
    return {
        "type": "item.completed",
        "item": {"id": "r", "type": "reasoning", "text": "thinking", "status": "completed"},
    }


def _assistant_segment() -> dict:
    return {
        "type": "item.completed",
        "item": {
            "id": "raw",
            "type": "raw",
            "status": "completed",
            "payload": {"ProviderMetadata": {"segment": "assistant"}},
        },
    }


def _tool(call_id: str, tool_name: str, command: str, status: str, payload: dict | None = None) -> list[dict]:
    payload = dict(payload or {})
    if command and "command" not in payload and "cmd" not in payload:
        payload["command"] = command
    started = {
        "type": "item.started",
        "item": {
            "id": call_id,
            "type": "toolExecution",
            "status": "inProgress",
            "tool_name": tool_name,
            "tool_call_id": call_id,
            "payload": payload,
        },
    }
    done = {
        "type": "item.completed",
        "item": {
            "id": call_id,
            "type": "toolExecution",
            "status": status,
            "tool_name": tool_name,
            "tool_call_id": call_id,
            "text": f"ran {command}",
        },
    }
    return [started, done]


def _flatten(events: list) -> list[dict]:
    flat: list[dict] = []
    for event in events:
        if isinstance(event, list):
            flat.extend(event)
        else:
            flat.append(event)
    return flat


def _write_trial(
    job_dir: Path,
    trial_name: str,
    events: list,
    *,
    reward: float,
    last_message: str = "Done.",
    run_summary_extra: dict | None = None,
) -> Path:
    trial_dir = job_dir / trial_name
    agent_dir = trial_dir / "agent"
    agent_dir.mkdir(parents=True)
    task_name = trial_name.split("__", 1)[0]
    (trial_dir / "result.json").write_text(
        json.dumps(
            {
                "trial_name": trial_name,
                "task_name": task_name,
                "verifier_result": {"rewards": {"reward": reward}},
            }
        )
        + "\n"
    )
    (trial_dir / "config.json").write_text("{}\n")
    (agent_dir / "roder-events.jsonl").write_text(
        "".join(json.dumps(event) + "\n" for event in _flatten(events))
    )
    run_summary = {"model": "gpt-5.5", "reasoning": "xhigh", "exit_status": 0}
    run_summary.update(run_summary_extra or {})
    (agent_dir / "roder-run-summary.json").write_text(json.dumps(run_summary) + "\n")
    (agent_dir / "roder-last-message.txt").write_text(last_message)
    return trial_dir


def _load_one(
    events: list,
    *,
    reward: float,
    last_message: str = "Done.",
    run_summary_extra: dict | None = None,
):
    temp = tempfile.mkdtemp()
    job_dir = Path(temp) / "job"
    job_dir.mkdir()
    _write_trial(
        job_dir,
        "task__x",
        events,
        reward=reward,
        last_message=last_message,
        run_summary_extra=run_summary_extra,
    )
    return load_trials(job_dir)[0]


if __name__ == "__main__":
    unittest.main()
