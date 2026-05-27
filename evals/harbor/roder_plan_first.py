"""Prompt and artifact helpers for Harbor plan-first eval runs."""

from __future__ import annotations

import shlex

from roder_exec_shell import roder_exec_shell_fragment

PLAN_FIRST_ARTIFACTS = [
    "/logs/agent/roder-plan-events.jsonl",
    "/logs/agent/roder-plan-stderr.txt",
    "/logs/agent/roder-plan-last-message.txt",
    "/logs/agent/roder-plan.md",
]

PLANNING_PROMPT = """Plan-first Terminal-Bench mode:
You are in the planning turn for a benchmark task. Your job is to inspect the
local task context, identify the scoring contract, and produce a concrete
implementation and verification plan for the next turn.

Rules for this planning turn:
- Do not create, edit, delete, compile into, or otherwise mutate the requested
  task output artifacts.
- Do not prototype the solution, compile candidate code, run test suites,
  install packages, start services, train models, brute-force searches, or run
  verifier-like long commands. Those belong in the implementation turn.
- Use at most three quick local inspection tool calls before writing the plan.
  If the contract is still uncertain after three probes, write the best plan
  with an explicit uncertainty note instead of continuing to inspect.
- Keep inspection local first: task files, /app, /tests, fixtures, and visible
  verifier code are higher value than web or broad research.
- Avoid long-running commands. Prefer quick file listings, targeted reads,
  greps, and small one-off probes that clarify the verifier contract.
- Identify exact required paths, formats, constants, thresholds, and validators.
- Call out any validation artifacts that must be kept outside required output
  directories during the implementation turn.
- End with a concise plan that the next turn can execute directly.
"""

IMPLEMENTATION_PROMPT = """Plan-first implementation turn:
The previous turn in this same thread was planning-only. Now implement the plan
against the actual benchmark workspace.

Implementation rules:
- Use the prior plan as the starting point, but correct it if local evidence
  proves part of the plan wrong.
- Do not spend the task budget restating the plan. Start implementing.
- Keep temporary validation artifacts outside required output directories, or
  remove them before final verification.
- Validate the exact final artifacts that the verifier will score, not only
  intermediate variables or scratch files.
"""


def plan_prompt_for_instruction(instruction: str) -> str:
    return f"{PLANNING_PROMPT}\n\nTerminal-Bench task instruction:\n{instruction}"


def implementation_prompt_for_instruction(
    *, terminal_bench_guidance: str, instruction: str
) -> str:
    guidance = terminal_bench_guidance.strip()
    parts = [part for part in (guidance, IMPLEMENTATION_PROMPT.strip()) if part]
    parts.append(f"Terminal-Bench task instruction:\n{instruction}")
    return "\n\n".join(parts)


def plan_first_shell_fragment(
    *,
    enabled: bool,
    events_path: str,
    stderr_path: str,
    last_message_path: str,
    plan_path: str,
    setup_summary_path: str,
    policy_mode: str,
    soft_timeout_sec: int | None,
    task_ledger_required: bool,
) -> str:
    if not enabled:
        return ""
    return (
        f": > {shlex.quote(events_path)}\n"
        f": > {shlex.quote(stderr_path)}\n"
        f": > {shlex.quote(last_message_path)}\n"
        f": > {shlex.quote(plan_path)}\n"
        f"printf 'roder plan-first planning turn starting\\n' >> {shlex.quote(setup_summary_path)}\n"
        + roder_exec_shell_fragment(
            events_path=events_path,
            stderr_path=stderr_path,
            last_message_path=last_message_path,
            prompt_env_var="RODER_HARBOR_PLAN_PROMPT",
            policy_mode=policy_mode,
            soft_timeout_sec=soft_timeout_sec,
            task_ledger_required=task_ledger_required,
        )
        + "plan_status=$?\n"
        "case \"$plan_status\" in 124|130|137|143) "
        f"printf 'roder plan-first planning turn soft-timed-out\\n' >> {shlex.quote(setup_summary_path)}; "
        ";; esac\n"
        f"if [ -s {shlex.quote(last_message_path)} ]; then "
        f"cp {shlex.quote(last_message_path)} {shlex.quote(plan_path)}; "
        "else "
        f"printf 'plan-first planning turn exited with status %s before writing a final message\\n' \"$plan_status\" > {shlex.quote(plan_path)}; "
        "fi\n"
        "RODER_HARBOR_PLAN_THREAD_ID=$(sed -n "
        + shlex.quote(r's/.*"thread_id":"\([^"]*\)".*/\1/p')
        + f" {shlex.quote(events_path)} | head -n 1)\n"
        "export RODER_HARBOR_PLAN_THREAD_ID\n"
        f"if [ -n \"${{RODER_HARBOR_PLAN_THREAD_ID:-}}\" ]; then "
        f"printf 'roder plan-first planning thread %s\\n' \"$RODER_HARBOR_PLAN_THREAD_ID\" >> {shlex.quote(setup_summary_path)}; "
        "else "
        f"printf 'roder plan-first planning thread id missing; implementation will start a new thread\\n' >> {shlex.quote(setup_summary_path)}; "
        "fi\n"
        f"printf 'roder plan-first planning turn finished with status %s\\n' \"$plan_status\" >> {shlex.quote(setup_summary_path)}\n"
    )
