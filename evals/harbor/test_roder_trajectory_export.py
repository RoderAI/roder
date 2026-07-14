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

from roder_trajectory_export import (  # noqa: E402
    build_trajectory,
    export_job,
    export_trial,
)
from roder_trajectory_features import Redactor  # noqa: E402

BEARER_SECRET = "sk-live-ABCDEFGHIJKLMNOP1234567890abcd"
API_KEY_SECRET = "sk-proj-ZZZ9988776655443322110099887766"
LONG_TOKEN = "A" * 240


class RoderTrajectoryExportTests(unittest.TestCase):
    def test_normal_conversion_produces_atif_trajectory(self) -> None:
        events = [
            {"type": "thread.started", "thread_id": "thread-1"},
            {"type": "turn.started", "turn_id": "turn-1"},
            _user("List the files and read the target."),
            _reasoning("I will inspect the workspace first."),
            _tool_start("call_1", "list_files", {"path": "."}),
            _tool_done("call_1", "list_files", "main.py\ntests/", "completed"),
            _agent_message("Found the target files."),
            {
                "type": "turn.completed",
                "usage": {
                    "input_tokens": 100,
                    "cached_input_tokens": 40,
                    "output_tokens": 20,
                    "reasoning_output_tokens": 5,
                },
            },
        ]
        with tempfile.TemporaryDirectory() as temp:
            trial_dir = _write_trial(Path(temp), "list-task__aaa", events, reward=0.0)
            trajectory = build_trajectory(trial_dir)

        self.assertEqual("ATIF-v1.7", trajectory["schema_version"])
        self.assertEqual("thread-1", trajectory["session_id"])
        self.assertEqual("roder", trajectory["agent"]["name"])
        self.assertEqual("gpt-5.5", trajectory["agent"]["model_name"])
        self.assertEqual(100, trajectory["final_metrics"]["total_prompt_tokens"])
        self.assertEqual(0.0, trajectory["extra"]["reward"])
        self._assert_atif_invariants(trajectory)

        sources = [step["source"] for step in trajectory["steps"]]
        self.assertEqual("user", sources[0])
        tool_steps = [s for s in trajectory["steps"] if s.get("tool_calls")]
        self.assertEqual(1, len(tool_steps))
        self.assertEqual("list_files", tool_steps[0]["tool_calls"][0]["function_name"])
        # reasoning attaches to the next agent step, not the user step.
        self.assertTrue(any(s.get("reasoning_content") for s in trajectory["steps"]))

    def test_redacts_bearer_token_and_api_key_in_command_and_output(self) -> None:
        events = [
            {"type": "thread.started", "thread_id": "thread-2"},
            {"type": "turn.started", "turn_id": "turn-2"},
            _user("Call the API."),
            _tool_start(
                "call_1",
                "shell",
                {"command": f'curl -H "Authorization: Bearer {BEARER_SECRET}" https://api.test'},
            ),
            _tool_done(
                "call_1",
                "shell",
                f"Exit code: 0\nOutput:\nOPENAI_API_KEY={API_KEY_SECRET}\n",
                "completed",
            ),
            {"type": "turn.completed", "usage": {"input_tokens": 5, "output_tokens": 1}},
        ]
        with tempfile.TemporaryDirectory() as temp:
            trial_dir = _write_trial(Path(temp), "api-task__bbb", events, reward=0.0)
            output_path = trial_dir / "agent" / "trajectory.json"
            result = export_trial(trial_dir, output_path)
            serialized = output_path.read_text()

        self.assertEqual("exported", result["status"])
        self.assertGreaterEqual(result["redactions"], 2)
        self.assertNotIn(BEARER_SECRET, serialized)
        self.assertNotIn(API_KEY_SECRET, serialized)
        self.assertIn("[REDACTED", serialized)

    def test_truncates_giant_tool_output(self) -> None:
        redactor = Redactor()
        big = "line\n" * 20000
        events = [
            {"type": "thread.started", "thread_id": "thread-3"},
            {"type": "turn.started", "turn_id": "turn-3"},
            _user("Dump a big file."),
            _tool_start("call_1", "shell", {"command": "cat big.txt"}),
            _tool_done("call_1", "shell", big, "completed"),
            {"type": "turn.completed", "usage": {"input_tokens": 5, "output_tokens": 1}},
        ]
        with tempfile.TemporaryDirectory() as temp:
            trial_dir = _write_trial(Path(temp), "big-task__ccc", events, reward=0.0)
            trajectory = build_trajectory(trial_dir)

        self.assertGreaterEqual(trajectory["extra"]["truncations"], 1)
        serialized = json.dumps(trajectory)
        self.assertIn("[TRUNCATED", serialized)
        tool_step = [s for s in trajectory["steps"] if s.get("observation")][0]
        content = tool_step["observation"]["results"][0]["content"]
        self.assertLess(len(content), len(big))

    def test_empty_events_file_writes_unsupported_reason(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            trial_dir = _write_trial(Path(temp), "empty-task__ddd", [], reward=0.0)
            (trial_dir / "agent" / "roder-events.jsonl").write_text("")
            output_path = trial_dir / "agent" / "trajectory.json"
            result = export_trial(trial_dir, output_path)
            artifact = json.loads(output_path.read_text())

        self.assertEqual("unsupported", result["status"])
        self.assertIn("unsupported_reason", artifact)
        self.assertIn("empty", artifact["unsupported_reason"])

    def test_corrupt_events_file_writes_unsupported_reason(self) -> None:
        with tempfile.TemporaryDirectory() as temp:
            trial_dir = _write_trial(Path(temp), "corrupt-task__eee", [], reward=0.0)
            (trial_dir / "agent" / "roder-events.jsonl").write_text(
                "not json\n{broken\n<<<garbage>>>\n"
            )
            output_path = trial_dir / "agent" / "trajectory.json"
            result = export_trial(trial_dir, output_path)
            artifact = json.loads(output_path.read_text())

        self.assertEqual("unsupported", result["status"])
        self.assertIn("no convertible events", artifact["unsupported_reason"])

    def test_failed_only_selection_skips_passing_trials(self) -> None:
        base_events = [
            {"type": "thread.started", "thread_id": "t"},
            {"type": "turn.started", "turn_id": "u"},
            _user("Do the task."),
            _tool_start("call_1", "shell", {"command": "echo hi"}),
            _tool_done("call_1", "shell", "Exit code: 0\nhi", "completed"),
            {"type": "turn.completed", "usage": {"input_tokens": 5, "output_tokens": 1}},
        ]
        with tempfile.TemporaryDirectory() as temp:
            job_dir = Path(temp) / "job"
            job_dir.mkdir()
            (job_dir / "result.json").write_text(json.dumps({"stats": {}}) + "\n")
            _write_trial(job_dir, "passing__p1", base_events, reward=1.0)
            _write_trial(job_dir, "failing__f1", base_events, reward=0.0)
            output_root = Path(temp) / "out"
            summary = export_job(job_dir, output_root, failed_only=True)

            # File-existence checks must run before the temp dir is cleaned up.
            self.assertTrue((output_root / "failing__f1" / "trajectory.json").exists())
            self.assertFalse((output_root / "passing__p1").exists())

        self.assertEqual(1, summary["trials"])
        self.assertEqual(1, summary["exported"])

    def _assert_atif_invariants(self, trajectory: dict) -> None:
        steps = trajectory["steps"]
        self.assertGreaterEqual(len(steps), 1)
        for index, step in enumerate(steps):
            self.assertEqual(index + 1, step["step_id"])
            self.assertIn(step["source"], {"system", "user", "agent"})
            self.assertIsInstance(step["message"], str)
            if step["source"] != "agent":
                for field in ("model_name", "reasoning_content", "tool_calls", "metrics"):
                    self.assertNotIn(field, step)
            observation = step.get("observation")
            if observation:
                call_ids = {tc["tool_call_id"] for tc in step.get("tool_calls", [])}
                for result in observation["results"]:
                    if result.get("source_call_id") is not None:
                        self.assertIn(result["source_call_id"], call_ids)


def _user(text: str) -> dict:
    return {"type": "item.completed", "item": {"id": "u", "type": "userMessage", "text": text}}


def _reasoning(text: str) -> dict:
    return {
        "type": "item.completed",
        "item": {"id": "r", "type": "reasoning", "text": text, "status": "completed"},
    }


def _agent_message(text: str) -> dict:
    return {
        "type": "item.completed",
        "item": {"id": "m", "type": "agentMessage", "text": text, "status": "completed"},
    }


def _tool_start(call_id: str, tool_name: str, payload: dict) -> dict:
    return {
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


def _tool_done(call_id: str, tool_name: str, text: str, status: str) -> dict:
    return {
        "type": "item.completed",
        "item": {
            "id": call_id,
            "type": "toolExecution",
            "status": status,
            "tool_name": tool_name,
            "tool_call_id": call_id,
            "text": text,
        },
    }


def _write_trial(base: Path, trial_name: str, events: list[dict], *, reward: float) -> Path:
    trial_dir = base / trial_name
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
    (agent_dir / "roder-events.jsonl").write_text(
        "".join(json.dumps(event) + "\n" for event in events)
    )
    (agent_dir / "roder-run-summary.json").write_text(
        json.dumps(
            {
                "provider": "codex",
                "model": "gpt-5.5",
                "reasoning": "xhigh",
                "policy_mode": "bypass",
                "elapsed_seconds": 12,
                "exit_status": 0,
            }
        )
        + "\n"
    )
    (agent_dir / "roder-last-message.txt").write_text("Done.")
    return trial_dir


if __name__ == "__main__":
    unittest.main()
