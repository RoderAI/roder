"""Shell rendering for recovering signal-terminated Roder Harbor runs."""

from __future__ import annotations

import shlex

from roder_run_summary_fragment import run_summary_shell_fragment


def signal_recovery_shell_fragment(
    *,
    signal_status: int,
    provider: str,
    model: str,
    reasoning: str,
    policy_mode: str,
    task_ledger_required: bool,
    soft_timeout_sec: int | None,
    eval_deadline_seconds: int | None,
    config_dir: str,
    events_path: str,
    stderr_path: str,
    output_path: str,
    last_message_path: str,
    setup_summary_path: str,
    run_summary_path: str,
) -> str:
    quoted_output = shlex.quote(output_path)
    quoted_last_message = shlex.quote(last_message_path)
    quoted_setup_summary = shlex.quote(setup_summary_path)
    return (
        "set -u\n"
        "start_epoch=$(date +%s)\n"
        "started_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')\n"
        f"status={int(signal_status)}\n"
        "soft_timed_out=1\n"
        "deadline_timed_out=0\n"
        f"printf 'roder exec signal-terminated with status %s; recovery finalizer running\\n' \"$status\" >> {quoted_setup_summary}\n"
        f"if [ ! -s {quoted_output} ]; then "
        f"printf 'roder exec was signal-terminated with status %s before writing normal output\\n' \"$status\" > {quoted_output}; "
        "fi\n"
        f"if [ ! -s {quoted_last_message} ]; then "
        f"printf 'roder exec was signal-terminated with status %s before writing a final message\\n' \"$status\" > {quoted_last_message}; "
        "fi\n"
        + run_summary_shell_fragment(
            provider=provider,
            model=model,
            reasoning=reasoning,
            policy_mode=policy_mode,
            task_ledger_required=task_ledger_required,
            soft_timeout_sec=soft_timeout_sec,
            eval_deadline_seconds=eval_deadline_seconds,
            config_dir=config_dir,
            events_path=events_path,
            stderr_path=stderr_path,
            output_path=output_path,
            last_message_path=last_message_path,
            run_summary_path=run_summary_path,
        )
        + f"printf 'roder exec signal recovery finalized with status %s\\n' \"$status\" >> {quoted_setup_summary}\n"
    )
