"""Summarize git state for pre-eval handoff reports."""

from __future__ import annotations

import subprocess
from typing import Any


def git_value(*args: str) -> str | None:
    try:
        result = subprocess.run(
            ["git", *args],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
    except Exception:
        return None
    return result.stdout.strip()


def git_summary(dirty_path_limit: int = 200) -> dict[str, Any]:
    return git_summary_from_status(
        head=git_value("rev-parse", "HEAD"),
        status=git_value("status", "--short"),
        dirty_path_limit=dirty_path_limit,
    )


def git_summary_from_status(
    *,
    head: str | None,
    status: str | None,
    dirty_path_limit: int,
) -> dict[str, Any]:
    if status is None:
        return {
            "head": head,
            "dirtyPathCount": None,
            "dirtyPaths": [],
            "dirtyPathLimit": dirty_path_limit,
            "dirtyPathTruncated": False,
        }
    paths = dirty_paths_from_status(status)
    return {
        "head": head,
        "dirtyPathCount": len(paths),
        "dirtyPaths": paths[:dirty_path_limit],
        "dirtyPathLimit": dirty_path_limit,
        "dirtyPathTruncated": len(paths) > dirty_path_limit,
    }


def dirty_paths_from_status(status: str) -> list[str]:
    paths: list[str] = []
    for line in status.splitlines():
        if not line.strip():
            continue
        paths.append(dirty_path_from_status_line(line))
    return paths


def dirty_path_from_status_line(line: str) -> str:
    if len(line) > 3 and line[2] == " ":
        return line[3:]
    if len(line) > 2 and line[1] == " ":
        return line[2:]
    return line.strip()
