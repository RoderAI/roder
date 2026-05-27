"""Shell command rendering for Harbor `roder exec` runs."""

from __future__ import annotations

import shlex


def roder_exec_shell_fragment(
    *,
    events_path: str,
    stderr_path: str,
    last_message_path: str,
    prompt_env_var: str,
    policy_mode: str,
    soft_timeout_sec: int | None,
    task_ledger_required: bool,
    resume_thread_env_var: str | None = None,
) -> str:
    ledger_flag = " --task-ledger-required" if task_ledger_required else ""
    base_args = (
        f"--json --profile eval --mode {shlex.quote(str(policy_mode))} "
        f"--skip-git-repo-check{ledger_flag} "
        f"--output-last-message {shlex.quote(last_message_path)} - "
        f">{shlex.quote(events_path)} 2>{shlex.quote(stderr_path)}"
    )
    new_command = f"roder exec {base_args}"
    if resume_thread_env_var:
        resume_command = f"roder exec resume \"${resume_thread_env_var}\" {base_args}"
        return (
            f"if [ -n \"${{{resume_thread_env_var}:-}}\" ]; then\n"
            + _roder_exec_invocation(
                command=resume_command,
                prompt_env_var=prompt_env_var,
                soft_timeout_sec=soft_timeout_sec,
                stderr_path=stderr_path,
                indent="  ",
            )
            + "else\n"
            + _roder_exec_invocation(
                command=new_command,
                prompt_env_var=prompt_env_var,
                soft_timeout_sec=soft_timeout_sec,
                stderr_path=stderr_path,
                indent="  ",
            )
            + "fi\n"
        )
    return _roder_exec_invocation(
        command=new_command,
        prompt_env_var=prompt_env_var,
        soft_timeout_sec=soft_timeout_sec,
        stderr_path=stderr_path,
    )


def _roder_exec_invocation(
    *,
    command: str,
    prompt_env_var: str,
    soft_timeout_sec: int | None,
    stderr_path: str,
    indent: str = "",
) -> str:
    prompt = f"${prompt_env_var}"
    if not soft_timeout_sec:
        return f"{indent}printf '%s' \"{prompt}\" | {command}\n"
    timeout = shlex.quote(f"{soft_timeout_sec}s")
    return (
        f"{indent}if command -v timeout >/dev/null 2>&1; then\n"
        f"{indent}  printf '%s' \"{prompt}\" | timeout -k 5s -s INT {timeout} {command}\n"
        f"{indent}else\n"
        f"{indent}  printf 'warning: timeout command unavailable; running without soft timeout\\n' "
        f">> {shlex.quote(stderr_path)}\n"
        f"{indent}  printf '%s' \"{prompt}\" | {command}\n"
        f"{indent}fi\n"
    )
