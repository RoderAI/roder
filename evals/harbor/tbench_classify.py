#!/usr/bin/env python3
"""Trial classification for Harbor Terminal-Bench analysis.

Split out of ``analyze_tbench_run.py``. A trial may receive multiple classes;
the subset in ``HARNESS_ERROR_CLASSES`` flips a run from clean to dirty. This
module is intentionally behaviour-preserving — do not change class triggers here
without updating the analyzer tests that pin clean-run semantics.
"""

from __future__ import annotations

import re

from tbench_trial import Trial, read_text


def classify_trial(trial: Trial) -> set[str]:
    classes: set[str] = set()
    text = trial.combined_text

    if trial.reward == 1.0 and not trial.exception_info:
        classes.add("pass")
    if trial.reward == 0.0:
        classes.add("scored_fail")

    if "registry-1.docker.io" in text and "Bad Gateway" in text:
        classes.add("docker_registry_bad_gateway")
    elif "Bad Gateway" in text and re.search(r"\bImage\b|\bdocker\b", text, re.I):
        classes.add("docker_registry_bad_gateway")

    if trial.exception_type == "AgentTimeoutError" or "Agent execution timed out" in text:
        classes.add("agent_timeout")
        if trial.roder_exit_status() == 0 and trial.has_nonempty_agent_artifact(
            "roder-last-message.txt"
        ):
            classes.add("agent_exec_finished_but_harbor_timeout")
        elif trial.has_nonempty_agent_artifact(
            "roder-events.jsonl"
        ) and not trial.has_nonempty_agent_artifact("roder-run-summary.json"):
            classes.add("agent_exec_timeout_no_summary")
    if trial.exception_info and re.search(
        r"Roder command failed with status (124|130|137|143)", text
    ):
        classes.add("agent_exec_signal_terminated_no_summary")
    if trial.run_summary.get("soft_timed_out") is True or "roder exec soft-timed-out" in text:
        classes.add("soft_timeout")
        if trial.reward == 1.0:
            classes.add("soft_timeout_pass")
        elif trial.reward == 0.0:
            classes.add("soft_timeout_fail")
    if (
        trial.run_summary.get("deadline_timed_out") is True
        or trial.run_summary.get("provider_error_kind") == "turn_deadline_expired"
        or "roder exec hit internal eval deadline before Harbor hard timeout" in text
        or "turn deadline expired" in text
    ):
        classes.add("internal_deadline_timeout")
    if (
        trial.run_summary.get("deadline_finalized") is True
        or "deadline finalization" in text.lower()
        or "before the deadline" in text.lower()
    ) and "turn deadline expired" not in text:
        classes.add("deadline_finalized")

    provider_error_kind = trial.run_summary.get("provider_error_kind")
    roder_status = trial.roder_exit_status()
    soft_or_deadline_timeout = (
        trial.run_summary.get("soft_timed_out") is True
        or trial.run_summary.get("deadline_timed_out") is True
        or trial.run_summary.get("provider_error_kind") == "turn_deadline_expired"
    )
    if (
        roder_status not in (None, 0, 124, 130, 137, 143)
        and not soft_or_deadline_timeout
        and provider_error_kind not in {"policy_block", "auth_refresh_token_reused"}
    ):
        classes.add("roder_exec_error_status")
    if provider_error_kind == "invalid_tool_name" or (
        "Invalid 'input[" in text and "string does not match pattern" in text
    ):
        classes.add("provider_api_invalid_tool_name")
    if provider_error_kind == "auth_refresh_token_reused" or "refresh_token_reused" in text:
        classes.add("provider_auth_refresh_token_reused")
    if provider_error_kind == "stream_decode_error" or "error decoding response body" in text:
        classes.add("provider_stream_decode_error")
    if (
        provider_error_kind == "stream_incomplete"
        or "stream closed before response.completed" in text
    ):
        classes.add("provider_stream_incomplete")
    if provider_error_kind == "policy_block" or "flagged for possible cybersecurity risk" in text:
        classes.add("provider_policy_block")

    setup_return = read_text(trial.path / "agent" / "setup" / "return-code.txt").strip()
    if (
        "Agent setup failed" in text
        or (setup_return and setup_return != "0" and not trial.has_agent_started())
    ):
        classes.add("agent_setup_failed")
    if (
        "Uploaded /installed-agent/roder is not executable in this task container" in text
        or "Selected Roder binary is not executable in this task container" in text
        or "No prebuilt Roder binary matched task container architecture" in text
        or "Dynamic loader not found: /lib64/ld-linux-x86-64.so.2" in text
        or re.search(r"\bnot a dynamic executable\b", text)
    ):
        classes.add("setup_arch_mismatch")

    if "Failed to download artifact" in trial.trial_log:
        classes.add("missing_artifacts")
    elif trial.has_agent_started() and trial.missing_expected_artifacts():
        classes.add("missing_artifacts")

    if trial.exception_info and not classes.intersection(
        {
            "docker_registry_bad_gateway",
            "agent_timeout",
            "agent_exec_signal_terminated_no_summary",
            "agent_setup_failed",
            "provider_auth_refresh_token_reused",
            "setup_arch_mismatch",
        }
    ):
        if trial.exception_type and "Verifier" in trial.exception_type:
            classes.add("verifier_error")
        else:
            classes.add("unknown_error")

    if not classes:
        classes.add("unknown")
    return classes
