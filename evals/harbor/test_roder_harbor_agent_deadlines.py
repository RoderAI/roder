#!/usr/bin/env python3

from __future__ import annotations

import asyncio
import importlib.util
import sys
import tempfile
import types
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
HARBOR_DIR = ROOT / "evals/harbor"
if str(HARBOR_DIR) not in sys.path:
    sys.path.insert(0, str(HARBOR_DIR))
MODULE_PATH = HARBOR_DIR / "roder_harbor_agent.py"


class FakeBaseInstalledAgent:
    def __init__(self, model_name: str | None = None, *args, **kwargs) -> None:
        self.model_name = model_name
        self.logs_dir = Path(".")

    def _setup_env(self) -> dict[str, str]:
        return {}


class FakeExecInput:
    def __init__(self, *, command: str, env: dict[str, str]) -> None:
        self.command = command
        self.env = env


class FakeAgentContext:
    metadata: dict | None = None


class FakeEnvironmentPaths:
    agent_dir = Path("/logs/agent")


class FakeResult:
    def __init__(self, return_code: int) -> None:
        self.return_code = return_code


def is_policy_block_check(command: str) -> bool:
    # The check command is the only one with the negated tool_name grep; the
    # run script also contains the marker literal (it greps for it too).
    return "! grep -q" in command


def is_run_script(command: str) -> bool:
    return "set -uo pipefail" in command


class RecordingEnvironment:
    """Records exec calls and replays scripted return codes by command shape."""

    def __init__(self, *, policy_block_hits: int) -> None:
        self.calls: list[str] = []
        self._policy_block_remaining = policy_block_hits

    async def exec(self, command: str, env: dict | None = None, **kwargs):
        self.calls.append(command)
        # The policy-block check exits 0 (block present, retry) while hits
        # remain, then non-zero (cleared) so the loop stops.
        if is_policy_block_check(command):
            if self._policy_block_remaining > 0:
                self._policy_block_remaining -= 1
                return FakeResult(0)
            return FakeResult(1)
        return FakeResult(0)


def load_module():
    fake_modules = {
        "harbor": types.ModuleType("harbor"),
        "harbor.agents": types.ModuleType("harbor.agents"),
        "harbor.agents.installed": types.ModuleType("harbor.agents.installed"),
        "harbor.agents.installed.base": types.ModuleType("harbor.agents.installed.base"),
        "harbor.environments": types.ModuleType("harbor.environments"),
        "harbor.environments.base": types.ModuleType("harbor.environments.base"),
        "harbor.models": types.ModuleType("harbor.models"),
        "harbor.models.agent": types.ModuleType("harbor.models.agent"),
        "harbor.models.agent.context": types.ModuleType("harbor.models.agent.context"),
        "harbor.models.trial": types.ModuleType("harbor.models.trial"),
        "harbor.models.trial.paths": types.ModuleType("harbor.models.trial.paths"),
    }
    fake_modules["harbor.agents.installed.base"].BaseInstalledAgent = FakeBaseInstalledAgent
    fake_modules["harbor.agents.installed.base"].ExecInput = FakeExecInput
    fake_modules["harbor.environments.base"].BaseEnvironment = object
    fake_modules["harbor.models.agent.context"].AgentContext = FakeAgentContext
    fake_modules["harbor.models.trial.paths"].EnvironmentPaths = FakeEnvironmentPaths
    previous = {name: sys.modules.get(name) for name in fake_modules}
    sys.modules.update(fake_modules)
    try:
        spec = importlib.util.spec_from_file_location("roder_harbor_agent", MODULE_PATH)
        module = importlib.util.module_from_spec(spec)
        assert spec.loader is not None
        spec.loader.exec_module(module)
        return module
    finally:
        for name, value in previous.items():
            if value is None:
                sys.modules.pop(name, None)
            else:
                sys.modules[name] = value


def write_task_cache(cache: Path, task: str, timeout: float) -> None:
    task_dir = cache / "cacheid" / task
    task_dir.mkdir(parents=True, exist_ok=True)
    (task_dir / "task.toml").write_text(f"[agent]\ntimeout_sec = {timeout}\n")


class ResolvedDeadlinesTests(unittest.TestCase):
    def test_static_track_uses_configured_values(self) -> None:
        module = load_module()
        agent = module.RoderCli(soft_timeout_sec=1780, speed_policy_eval_deadline_seconds=1740)
        self.assertEqual((1780, 1740), agent._resolved_deadlines())

    def test_per_task_track_derives_from_task_window(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            write_task_cache(cache, "compile-compcert", 2400)
            agent = module.RoderCli(
                per_task_deadlines=True,
                agent_timeout_multiplier_hint=1.0,
                task_cache_dir=str(cache),
            )
            agent.logs_dir = Path(tmp) / "jobs" / "j" / "compile-compcert__abc" / "agent"
            soft, deadline = agent._resolved_deadlines()
            self.assertEqual(2340, soft)
            self.assertEqual(2280, deadline)

    def test_per_task_track_falls_back_when_task_unknown(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as tmp:
            agent = module.RoderCli(
                per_task_deadlines=True,
                agent_timeout_multiplier_hint=1.0,
                task_cache_dir=str(tmp),
                soft_timeout_sec=780,
                speed_policy_eval_deadline_seconds=720,
            )
            agent.logs_dir = Path(tmp) / "jobs" / "j" / "unknown-task__abc" / "agent"
            self.assertEqual((780, 720), agent._resolved_deadlines())

    def test_codex_parity_has_no_internal_deadline(self) -> None:
        # Minimal / codex-parity config: no per_task, no soft, no eval -> (None, None)
        # -> the run script must have NO `timeout -s INT` wrapper and the config.toml
        # must have NO eval_deadline_seconds line, so the agent runs to Harbor's hard
        # window exactly like the Codex harness.
        module = load_module()
        agent = module.RoderCli(
            benchmark_guidance_enabled="false", task_ledger_required="false"
        )
        self.assertEqual((None, None), agent._resolved_deadlines())
        commands = agent.create_run_agent_commands("solve it")
        setup_command = commands[0].command
        run_script = commands[1].command
        self.assertNotIn("eval_deadline_seconds", setup_command)
        self.assertNotIn("timeout -k 5s -s INT", run_script)
        self.assertNotIn("--task-ledger-required", run_script)
        self.assertNotIn(
            "Terminal-Bench harness guidance", str(commands[1].env)
        )

    def test_per_task_derived_deadline_flows_into_run_script(self) -> None:
        module = load_module()
        with tempfile.TemporaryDirectory() as tmp:
            cache = Path(tmp)
            write_task_cache(cache, "regex-chess", 3600)
            agent = module.RoderCli(
                per_task_deadlines=True,
                agent_timeout_multiplier_hint=1.0,
                task_cache_dir=str(cache),
            )
            agent.logs_dir = Path(tmp) / "jobs" / "j" / "regex-chess__abc" / "agent"
            commands = agent.create_run_agent_commands("solve it")
            setup_command = commands[0].command
            run_script = commands[1].command
            # eval deadline is baked into config.toml (setup); soft timeout
            # wraps the exec in the run script.
            self.assertIn("eval_deadline_seconds = 3480", setup_command)
            self.assertIn("3540s", run_script)


class PolicyBlockRetryLoopTests(unittest.TestCase):
    def test_zero_progress_block_triggers_bounded_retry(self) -> None:
        module = load_module()
        agent = module.RoderCli(
            soft_timeout_sec=1780,
            speed_policy_eval_deadline_seconds=1740,
            policy_block_max_retries=2,
        )
        env = RecordingEnvironment(policy_block_hits=1)
        asyncio.run(agent.run("solve it", env, FakeAgentContext()))
        run_commands = [c for c in env.calls if is_run_script(c)]
        # First run + one retry run == 2 run-script executions.
        self.assertEqual(2, len(run_commands))

    def test_no_retry_when_block_absent(self) -> None:
        module = load_module()
        agent = module.RoderCli(
            soft_timeout_sec=1780,
            speed_policy_eval_deadline_seconds=1740,
            policy_block_max_retries=2,
        )
        env = RecordingEnvironment(policy_block_hits=0)
        asyncio.run(agent.run("solve it", env, FakeAgentContext()))
        run_commands = [c for c in env.calls if is_run_script(c)]
        self.assertEqual(1, len(run_commands))

    def test_retries_disabled_makes_no_check(self) -> None:
        module = load_module()
        agent = module.RoderCli(
            soft_timeout_sec=1780,
            speed_policy_eval_deadline_seconds=1740,
            policy_block_max_retries=0,
        )
        env = RecordingEnvironment(policy_block_hits=1)
        asyncio.run(agent.run("solve it", env, FakeAgentContext()))
        check_calls = [c for c in env.calls if is_policy_block_check(c)]
        self.assertEqual(0, len(check_calls))


if __name__ == "__main__":
    unittest.main()
