#!/usr/bin/env python3

from __future__ import annotations

import importlib.util
import sys
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


def load_module():
    fake_modules = {
        "harbor": types.ModuleType("harbor"),
        "harbor.agents": types.ModuleType("harbor.agents"),
        "harbor.agents.installed": types.ModuleType("harbor.agents.installed"),
        "harbor.agents.installed.base": types.ModuleType(
            "harbor.agents.installed.base"
        ),
        "harbor.environments": types.ModuleType("harbor.environments"),
        "harbor.environments.base": types.ModuleType("harbor.environments.base"),
        "harbor.models": types.ModuleType("harbor.models"),
        "harbor.models.agent": types.ModuleType("harbor.models.agent"),
        "harbor.models.agent.context": types.ModuleType("harbor.models.agent.context"),
        "harbor.models.trial": types.ModuleType("harbor.models.trial"),
        "harbor.models.trial.paths": types.ModuleType("harbor.models.trial.paths"),
    }
    fake_modules["harbor.agents.installed.base"].BaseInstalledAgent = (
        FakeBaseInstalledAgent
    )
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


class RoderHarborAgentConfigTests(unittest.TestCase):
    def test_fractional_provider_retry_backoff_factor_is_preserved(self) -> None:
        module = load_module()

        agent = module.RoderCli(reliability_provider_retry_backoff_factor="1.5")

        self.assertIn(
            "provider_retry_backoff_factor = 1.5",
            module.reliability_config_toml(agent._reliability),
        )


if __name__ == "__main__":
    unittest.main()
