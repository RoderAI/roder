#!/usr/bin/env python3
"""Trial dataclass and per-trial artifact loaders for Harbor Terminal-Bench analysis.

Split out of ``analyze_tbench_run.py`` (which grew past the 500-line limit). This
module owns raw-file parsing: loading a Harbor job dir into ``Trial`` records and
the derived accessors (reward, exit status, artifact sizes) that classification
and feature extraction build on.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from tbench_analysis_constants import CORE_ARTIFACTS


@dataclass
class Trial:
    name: str
    task_name: str
    path: Path
    result: dict[str, Any]
    config: dict[str, Any]
    trial_log: str
    exception_text: str
    setup_text: str
    agent_text: str
    run_summary: dict[str, Any]

    @property
    def combined_text(self) -> str:
        chunks = [self.trial_log, self.exception_text, self.setup_text, self.agent_text]
        return "\n".join(chunk for chunk in chunks if chunk)

    @property
    def exception_info(self) -> dict[str, Any] | None:
        info = self.result.get("exception_info")
        return info if isinstance(info, dict) else None

    @property
    def exception_type(self) -> str | None:
        info = self.exception_info
        value = info.get("exception_type") if info else None
        return str(value) if value else None

    @property
    def reward(self) -> float | None:
        verifier = self.result.get("verifier_result")
        if not isinstance(verifier, dict):
            return None
        rewards = verifier.get("rewards")
        if not isinstance(rewards, dict):
            return None
        reward = rewards.get("reward")
        try:
            return float(reward)
        except (TypeError, ValueError):
            return None

    @property
    def expected_artifacts(self) -> list[str]:
        artifacts = self.config.get("artifacts")
        if not isinstance(artifacts, list):
            return []
        names: list[str] = []
        for artifact in artifacts:
            if not isinstance(artifact, str):
                continue
            if artifact.startswith("/logs/agent/"):
                names.append(artifact.removeprefix("/logs/agent/"))
            else:
                names.append(Path(artifact).name)
        return names

    def has_agent_started(self) -> bool:
        return self.result.get("agent_execution") is not None or (
            self.path / "agent" / "command-0"
        ).exists()

    def missing_expected_artifacts(self) -> list[str]:
        agent_dir = self.path / "agent"
        missing = [name for name in self.expected_artifacts if not (agent_dir / name).exists()]
        if self.has_agent_started():
            missing.extend(
                name for name in CORE_ARTIFACTS if name not in missing and not (agent_dir / name).exists()
            )
        return sorted(set(missing))

    def agent_artifact_path(self, name: str) -> Path:
        return self.path / "agent" / name

    def agent_artifact_size(self, name: str) -> int | None:
        path = self.agent_artifact_path(name)
        if not path.exists():
            return None
        try:
            return path.stat().st_size
        except OSError:
            return None

    def has_nonempty_agent_artifact(self, name: str) -> bool:
        size = self.agent_artifact_size(name)
        return size is not None and size > 0

    def roder_exit_status(self) -> int | None:
        summary_status = self.run_summary.get("exit_status")
        if summary_status is not None:
            try:
                return int(summary_status)
            except (TypeError, ValueError):
                pass
        match = re.search(r"roder exec finished with status (\d+)", self.setup_text)
        if not match:
            return None
        try:
            return int(match.group(1))
        except ValueError:
            return None


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def load_json_if_present(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {}
    try:
        value = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError):
        return {}
    return value if isinstance(value, dict) else {}


def read_text(path: Path) -> str:
    if not path.exists():
        return ""
    try:
        return path.read_text(errors="replace")
    except OSError:
        return ""


def setup_text(trial_dir: Path) -> str:
    setup_dir = trial_dir / "agent" / "setup"
    chunks = []
    for name in ("return-code.txt", "stdout.txt", "stderr.txt"):
        text = read_text(setup_dir / name)
        if text:
            chunks.append(f"--- {name} ---\n{text}")
    summary = read_text(trial_dir / "agent" / "setup-summary.txt")
    if summary:
        chunks.append(f"--- setup-summary.txt ---\n{summary}")
    return "\n".join(chunks)


def agent_text(trial_dir: Path) -> str:
    chunks = []
    for base in (trial_dir / "agent", trial_dir / "artifacts"):
        for name in ("roder-stderr.txt", "roder-cli.txt"):
            text = read_text(base / name)
            if text:
                chunks.append(f"--- {base.name}/{name} ---\n{text}")
    return "\n".join(chunks)


def task_name_from_trial_name(name: str) -> str:
    return name.split("__", 1)[0]


def load_trials(job_dir: Path) -> list[Trial]:
    trials: list[Trial] = []
    for result_path in sorted(job_dir.glob("*/result.json")):
        trial_dir = result_path.parent
        result = load_json(result_path)
        config_path = trial_dir / "config.json"
        config = load_json(config_path) if config_path.exists() else {}
        name = str(result.get("trial_name") or trial_dir.name)
        task_name = str(result.get("task_name") or task_name_from_trial_name(name))
        exception = read_text(trial_dir / "exception.txt")
        trials.append(
            Trial(
                name=name,
                task_name=task_name,
                path=trial_dir,
                result=result,
                config=config,
                trial_log=read_text(trial_dir / "trial.log"),
                exception_text=exception,
                setup_text=setup_text(trial_dir),
                agent_text=agent_text(trial_dir),
                run_summary=load_json_if_present(
                    trial_dir / "agent" / "roder-run-summary.json"
                ),
            )
        )
    return trials
