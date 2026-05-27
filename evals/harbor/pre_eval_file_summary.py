"""File metadata helpers for pre-eval summaries."""

from __future__ import annotations

import hashlib
import json
import os
import subprocess
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


def prebuilt_summary(path: Path, required: bool) -> dict[str, Any]:
    path = path.expanduser()
    exists = path.exists()
    summary: dict[str, Any] = {
        "path": str(path),
        "required": required,
        "exists": exists,
        "executable": bool(exists and os.access(path, os.X_OK)),
    }
    if not exists:
        return summary
    stat = path.stat()
    summary.update(
        {
            "sizeBytes": stat.st_size,
            "modifiedAt": datetime.fromtimestamp(
                stat.st_mtime, timezone.utc
            ).isoformat(),
            "sha256": sha256(path),
            "fileType": file_type(path),
            "linuxX8664Elf": is_linux_x86_64_elf(path),
        }
    )
    return summary


def prebuilt_is_blocked(value: Any) -> bool:
    if not isinstance(value, dict) or value.get("required") is not True:
        return False
    return (
        value.get("exists") is not True
        or value.get("executable") is not True
        or value.get("linuxX8664Elf") is not True
    )


def auth_summary(path: Path, required: bool) -> dict[str, Any]:
    path = path.expanduser()
    exists = path.exists()
    summary: dict[str, Any] = {
        "path": str(path),
        "required": required,
        "exists": exists,
    }
    if not exists:
        return summary
    stat = path.stat()
    summary.update(
        {
            "sizeBytes": stat.st_size,
            "modifiedAt": datetime.fromtimestamp(
                stat.st_mtime, timezone.utc
            ).isoformat(),
        }
    )
    try:
        data = json.loads(path.read_text())
    except Exception:
        summary["validJson"] = False
        return summary
    summary["validJson"] = isinstance(data, dict)
    if isinstance(data, dict):
        summary["jsonFields"] = sorted(str(key) for key in data.keys())
    return summary


def auth_is_blocked(value: Any) -> bool:
    if not isinstance(value, dict) or value.get("required") is not True:
        return False
    return value.get("exists") is not True or value.get("validJson") is not True


def is_linux_x86_64_elf(path: Path) -> bool:
    try:
        header = path.read_bytes()[:64]
    except OSError:
        return False
    if len(header) < 20 or header[:4] != b"\x7fELF":
        return False
    return (
        header[4] == 2
        and header[5] == 1
        and int.from_bytes(header[18:20], "little") == 0x3E
        and header[7] in (0, 3)
    )


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def file_type(path: Path) -> str | None:
    try:
        result = subprocess.run(
            ["file", str(path)],
            check=True,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
    except Exception:
        return None
    return result.stdout.strip()
