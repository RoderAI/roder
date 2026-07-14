"""Assemble the in-container setup and run shell commands for the Roder adapter.

Split out of ``roder_harbor_agent.py`` to keep the adapter under the repo's
500-line module rule. The builder reads the adapter's resolved settings and
delegates the individual shell fragments to their existing modules.
"""

from __future__ import annotations

import json
import shlex
from typing import Any

from harbor.models.trial.paths import EnvironmentPaths

from roder_harbor_agent_config import (
    reliability_config_toml,
    speed_policy_config_toml,
    tools_config_toml,
)
from roder_exec_shell import roder_exec_shell_fragment
from roder_plan_first import plan_first_shell_fragment
from roder_run_summary_fragment import run_summary_shell_fragment

CONFIG_DIR = "/tmp/roder-harbor"


def build_run_agent_commands(agent: Any, instruction: str) -> list[tuple[str, dict[str, str]]]:
    """Return ``(command, env)`` pairs for the setup and run exec steps."""
    provider, model = agent._resolved_provider_model()
    soft_timeout_sec, eval_deadline_seconds = agent._resolved_deadlines()
    paths = _trial_paths()

    setup = _build_setup_command(
        agent, provider=provider, model=model, eval_deadline_seconds=eval_deadline_seconds, paths=paths
    )
    run_script = _build_run_script(
        agent,
        provider=provider,
        model=model,
        soft_timeout_sec=soft_timeout_sec,
        eval_deadline_seconds=eval_deadline_seconds,
        paths=paths,
    )
    run = f"bash -lc {shlex.quote(run_script)}"
    env = {"RODER_CONFIG_DIR": CONFIG_DIR, "RODER_DATA_DIR": CONFIG_DIR}
    env.update(agent._provider_env(provider))
    env.update(agent._plan_first_env(instruction))
    return [(setup, env), (run, env)]


def _trial_paths() -> dict[str, str]:
    agent_dir = EnvironmentPaths.agent_dir
    names = {
        "events": "roder-events.jsonl",
        "stderr": "roder-stderr.txt",
        "output": "roder-cli.txt",
        "last_message": "roder-last-message.txt",
        "setup_summary": "setup-summary.txt",
        "run_summary": "roder-run-summary.json",
        "plan_events": "roder-plan-events.jsonl",
        "plan_stderr": "roder-plan-stderr.txt",
        "plan_last_message": "roder-plan-last-message.txt",
        "plan": "roder-plan.md",
    }
    return {key: (agent_dir / name).as_posix() for key, name in names.items()}


def _build_setup_command(
    agent: Any,
    *,
    provider: str,
    model: str,
    eval_deadline_seconds: int | None,
    paths: dict[str, str],
) -> str:
    touch_targets = " ".join(
        shlex.quote(paths[key])
        for key in (
            "events",
            "stderr",
            "output",
            "last_message",
            "setup_summary",
            "run_summary",
            "plan_events",
            "plan_stderr",
            "plan_last_message",
            "plan",
        )
    )
    return (
        f"mkdir -p {CONFIG_DIR}/auth /logs/agent && "
        f"touch {touch_targets} && "
        f"printf 'Roder run command setup started\\n' >> {shlex.quote(paths['setup_summary'])} && "
        "if [ -f /installed-agent/roder-auth.json ]; then "
        f"cp /installed-agent/roder-auth.json {CONFIG_DIR}/auth/codex.json; "
        "fi && "
        f"cat > {CONFIG_DIR}/config.toml <<'EOF'\n"
        f"provider = {json.dumps(provider)}\n"
        f"model = {json.dumps(model)}\n"
        f"reasoning = {json.dumps(str(agent._reasoning))}\n"
        'runtime_profile = "eval"\n'
        f"{speed_policy_config_toml(enabled=agent._speed_policy_enabled, eval_deadline_seconds=eval_deadline_seconds, reasoning=agent._speed_policy_reasoning)}"
        f"{reliability_config_toml(agent._reliability)}"
        f"{tools_config_toml(agent._tool_allowlist)}"
        "\n"
        "[policy_modes]\n"
        f"default = {json.dumps(str(agent._policy_mode))}\n"
        "warn_on_bypass = false\n"
        "EOF"
    )


def _build_run_script(
    agent: Any,
    *,
    provider: str,
    model: str,
    soft_timeout_sec: int | None,
    eval_deadline_seconds: int | None,
    paths: dict[str, str],
) -> str:
    events_path = paths["events"]
    stderr_path = paths["stderr"]
    output_path = paths["output"]
    last_message_path = paths["last_message"]
    setup_summary_path = paths["setup_summary"]
    run_summary_path = paths["run_summary"]
    return (
        "set -uo pipefail\n"
        f": > {shlex.quote(events_path)}\n"
        f": > {shlex.quote(stderr_path)}\n"
        f": > {shlex.quote(output_path)}\n"
        f": > {shlex.quote(last_message_path)}\n"
        f": > {shlex.quote(run_summary_path)}\n"
        "start_epoch=$(date +%s)\n"
        "started_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')\n"
        "soft_timed_out=0\n"
        "deadline_timed_out=0\n"
        "provider_policy_blocked=0\n"
        "RODER_HARBOR_PLAN_THREAD_ID=\n"
        f"printf 'roder exec starting\\n' >> {shlex.quote(setup_summary_path)}\n"
        + agent._reasoning_shell_fragment(
            config_dir=CONFIG_DIR,
            setup_summary_path=setup_summary_path,
            reasoning=agent._plan_first_reasoning,
            label="plan-first planning",
        )
        + plan_first_shell_fragment(
            enabled=agent._plan_first_enabled,
            events_path=paths["plan_events"],
            stderr_path=paths["plan_stderr"],
            last_message_path=paths["plan_last_message"],
            plan_path=paths["plan"],
            setup_summary_path=setup_summary_path,
            policy_mode=agent._plan_first_policy_mode,
            soft_timeout_sec=agent._plan_first_soft_timeout_sec,
            task_ledger_required=agent._task_ledger_required,
        )
        + agent._reasoning_shell_fragment(
            config_dir=CONFIG_DIR,
            setup_summary_path=setup_summary_path,
            reasoning=str(agent._reasoning),
            label="implementation",
        )
        + roder_exec_shell_fragment(
            events_path=events_path,
            stderr_path=stderr_path,
            last_message_path=last_message_path,
            prompt_env_var="RODER_HARBOR_PROMPT",
            policy_mode=str(agent._policy_mode),
            soft_timeout_sec=soft_timeout_sec,
            task_ledger_required=agent._task_ledger_required,
            resume_thread_env_var=(
                "RODER_HARBOR_PLAN_THREAD_ID" if agent._plan_first_enabled else None
            ),
        )
        + "status=$?\n"
        'case "$status" in 124|130|137|143) soft_timed_out=1 ;; esac\n'
        f"if grep -q 'turn deadline expired' {shlex.quote(stderr_path)}; then "
        "deadline_timed_out=1; soft_timed_out=1; "
        "fi\n"
        f"if grep -q 'flagged for possible cybersecurity risk' {shlex.quote(stderr_path)}; then "
        "provider_policy_blocked=1; "
        "fi\n"
        f"if [ -s {shlex.quote(last_message_path)} ]; then "
        f"cp {shlex.quote(last_message_path)} {shlex.quote(output_path)}; "
        "else "
        f"printf 'roder exec exited with status %s before writing a final message\\n' \"$status\" > {shlex.quote(output_path)}; "
        "fi\n"
        f"if [ -s {shlex.quote(stderr_path)} ]; then "
        f"printf '\\n--- roder stderr ---\\n' >> {shlex.quote(output_path)}; "
        f"cat {shlex.quote(stderr_path)} >> {shlex.quote(output_path)}; "
        "fi\n"
        f"printf 'roder exec finished with status %s\\n' \"$status\" >> {shlex.quote(setup_summary_path)}\n"
        'if [ "$deadline_timed_out" -eq 1 ]; then '
        f"printf 'roder exec hit internal eval deadline before Harbor hard timeout\\n' >> {shlex.quote(setup_summary_path)}; "
        "fi\n"
        + run_summary_shell_fragment(
            provider=provider,
            model=model,
            reasoning=str(agent._reasoning),
            policy_mode=str(agent._policy_mode),
            task_ledger_required=agent._task_ledger_required,
            soft_timeout_sec=soft_timeout_sec,
            eval_deadline_seconds=eval_deadline_seconds,
            config_dir=CONFIG_DIR,
            events_path=events_path,
            stderr_path=stderr_path,
            output_path=output_path,
            last_message_path=last_message_path,
            run_summary_path=run_summary_path,
        )
        + 'if [ "$soft_timed_out" -eq 1 ]; then '
        f"printf 'roder exec soft-timed-out before Harbor hard timeout\\n' >> {shlex.quote(setup_summary_path)}; "
        "exit 0; "
        "fi\n"
        + 'if [ "$provider_policy_blocked" -eq 1 ]; then '
        f"printf 'roder exec provider-policy-blocked; preserving scored trial artifacts\\n' >> {shlex.quote(setup_summary_path)}; "
        "exit 0; "
        "fi\n"
        'exit "$status"\n'
    )
