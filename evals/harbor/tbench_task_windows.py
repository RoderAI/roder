"""Resolve per-task Terminal-Bench agent windows from Harbor's local task cache.

Harbor enforces one hard agent timeout per trial, computed from the task's
`task.toml [agent] timeout_sec` times the job's agent timeout multiplier. It
never tells the agent that number, so the Roder adapter resolves it host-side:
the trial directory name embeds the task name, and Harbor's dataset cache
(`~/.cache/harbor/tasks/*/<task>/task.toml`) holds the unmodified per-task
timeout. Reading the declared timeout keeps Roder inside the benchmark's own
window; it does not modify task resources or expose task internals.
"""

from __future__ import annotations

import tomllib
from pathlib import Path

HARBOR_TASK_CACHE_DIR = Path.home() / ".cache" / "harbor" / "tasks"


def task_name_from_logs_dir(logs_dir: Path | str | None) -> str | None:
    """Extract the Harbor task name from a per-trial agent logs dir.

    Trial layout is `<jobs_dir>/<job>/<task>__<suffix>/agent`; Harbor appends
    the random suffix with a double underscore.
    """
    if not logs_dir:
        return None
    trial_dir_name = Path(logs_dir).parent.name
    if "__" not in trial_dir_name:
        return None
    task_name = trial_dir_name.rsplit("__", 1)[0]
    return task_name or None


def lookup_task_agent_timeout_sec(
    task_name: str | None,
    *,
    cache_dir: Path | str | None = None,
) -> float | None:
    """Return the task's declared `[agent] timeout_sec` (canonical copy preferred).

    Harbor keeps two kinds of `task.toml` under the cache: the canonical
    content-addressed dataset copy at
    ``packages/<dataset>/<task>/<hash>/task.toml`` (the real window Harbor
    enforces for the active dataset), and per-job snapshot copies at
    ``<jobhash>/<task>/task.toml`` that can be from an older/shorter dataset
    version. The old glob ``*/<task>/task.toml`` matched only the shallow
    per-job copies (the canonical one is one level deeper) and picked the
    newest by mtime, so it silently returned a stale shorter window (e.g.
    crack-7z-hash 900s instead of the real 1800s, caffe-cifar-10 1200s instead
    of 3600s). Prefer the canonical ``packages/`` copy; fall back to the
    per-job snapshots only when no canonical copy exists.
    """
    if not task_name:
        return None
    cache = Path(cache_dir).expanduser() if cache_dir else HARBOR_TASK_CACHE_DIR
    if not cache.is_dir():
        return None
    for pattern in (f"packages/*/{task_name}/*/task.toml", f"*/{task_name}/task.toml"):
        candidates = sorted(cache.glob(pattern), key=_safe_mtime, reverse=True)
        for path in candidates:
            timeout = _read_agent_timeout_sec(path)
            if timeout is not None:
                return timeout
    return None


def _read_agent_timeout_sec(path: Path) -> float | None:
    try:
        with path.open("rb") as handle:
            data = tomllib.load(handle)
    except (OSError, tomllib.TOMLDecodeError):
        return None
    agent = data.get("agent")
    if not isinstance(agent, dict):
        return None
    timeout = agent.get("timeout_sec")
    if isinstance(timeout, bool) or not isinstance(timeout, (int, float)):
        return None
    if timeout <= 0:
        return None
    return float(timeout)


def _safe_mtime(path: Path) -> float:
    try:
        return path.stat().st_mtime
    except OSError:
        return 0.0
