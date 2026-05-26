"""Live file checks shared by Harbor pre-eval validators."""

from __future__ import annotations

import hashlib
import json
import os
from pathlib import Path
from typing import Any, Iterable


REQUIRED_AUTH_STRING_FIELDS = ("access", "refresh", "account_id", "type")


def file_sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def combined_file_digest(entries: list[dict[str, Any]]) -> str:
    digest = hashlib.sha256()
    for entry in sorted(entries, key=lambda item: str(item.get("path") or "")):
        digest.update(str(entry.get("path") or "").encode())
        digest.update(b"\0")
        digest.update(str(entry.get("sha256") or "").encode())
        digest.update(b"\0")
    return digest.hexdigest()


def validate_harbor_config_files(
    issues: list[str],
    entries: Any,
    *,
    required_paths: Iterable[Path] | Iterable[str] | None = None,
) -> None:
    validate_required_paths(
        issues,
        entries,
        required_paths,
        label="harborConfigs",
    )
    validate_file_entries(
        issues,
        entries,
        missing_message="harborConfigs entries are missing",
        object_message="harborConfigs entry is not an object",
        path_message="harborConfigs entry path is missing",
        sha_message_prefix="harborConfigs entry SHA-256 is missing",
        duplicate_message_prefix="harborConfigs duplicate file entry",
        read_message_prefix="Harbor config file cannot be read",
        mismatch_message="Harbor config SHA-256 mismatch",
    )


def validate_harbor_harness_files(
    issues: list[str],
    entries: Any,
    expected_combined: Any,
    *,
    required_paths: Iterable[Path] | Iterable[str] | None = None,
) -> None:
    validate_required_paths(
        issues,
        entries,
        required_paths,
        label="harborHarness",
    )
    current_entries = validate_file_entries(
        issues,
        entries,
        missing_message="harborHarness entries are missing",
        object_message="harborHarness entry is not an object",
        path_message="harborHarness entry path is missing",
        sha_message_prefix="harborHarness entry SHA-256 is missing",
        duplicate_message_prefix="harborHarness duplicate file entry",
        read_message_prefix="Harbor harness file cannot be read",
        mismatch_message="Harbor harness file SHA-256 mismatch",
    )
    if not isinstance(expected_combined, str) or not expected_combined:
        return
    if current_entries and combined_file_digest(current_entries) != expected_combined:
        issues.append("Harbor harness combined SHA-256 mismatch")


def validate_required_paths(
    issues: list[str],
    entries: Any,
    required_paths: Iterable[Path] | Iterable[str] | None,
    *,
    label: str,
) -> None:
    if required_paths is None or not isinstance(entries, list):
        return
    seen = {
        str(entry.get("path") or "")
        for entry in entries
        if isinstance(entry, dict)
    }
    for path in required_paths:
        normalized = str(path)
        if normalized not in seen:
            issues.append(f"{label} required file missing: {normalized}")


def validate_file_entries(
    issues: list[str],
    entries: Any,
    *,
    missing_message: str,
    object_message: str,
    path_message: str,
    sha_message_prefix: str,
    read_message_prefix: str,
    mismatch_message: str,
    duplicate_message_prefix: str | None = None,
) -> list[dict[str, Any]]:
    current_entries: list[dict[str, Any]] = []
    seen_paths: set[str] = set()
    if not isinstance(entries, list) or not entries:
        issues.append(missing_message)
        return current_entries

    for entry in entries:
        if not isinstance(entry, dict):
            issues.append(object_message)
            continue
        path = entry.get("path")
        expected = entry.get("sha256")
        if not isinstance(path, str) or not path:
            issues.append(path_message)
            continue
        if duplicate_message_prefix is not None and path in seen_paths:
            issues.append(f"{duplicate_message_prefix}: {path}")
            continue
        seen_paths.add(path)
        if not isinstance(expected, str) or not expected:
            issues.append(f"{sha_message_prefix}: {path}")
            continue
        try:
            actual = file_sha256(Path(path))
        except OSError as exc:
            issues.append(f"{read_message_prefix}: {exc}")
            continue
        current_entries.append({"path": path, "sha256": actual})
        if actual != expected:
            issues.append(mismatch_message)
    return current_entries


def validate_prebuilt_file(issues: list[str], value: Any) -> None:
    prebuilt = dict_value(value)
    path = prebuilt.get("path")
    expected = prebuilt.get("sha256")
    if not isinstance(path, str) or not path:
        issues.append("prebuilt binary path is missing")
        return
    if not isinstance(expected, str) or not expected:
        issues.append("prebuilt binary SHA-256 is missing")
        return
    try:
        current_path = Path(path).expanduser()
        actual = file_sha256(current_path)
    except OSError as exc:
        issues.append(f"prebuilt binary file cannot be read: {exc}")
        return
    if actual != expected:
        issues.append("prebuilt binary SHA-256 mismatch")
    if not os.access(current_path, os.X_OK):
        issues.append("prebuilt binary is not executable")
    if not is_linux_x86_64_elf(current_path):
        issues.append("prebuilt binary is not Linux x86-64 ELF")


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


def validate_auth_file(issues: list[str], value: Any) -> None:
    auth = dict_value(value)
    path = auth.get("path")
    if not isinstance(path, str) or not path:
        issues.append("auth file path is missing")
        return
    try:
        data = json.loads(Path(path).expanduser().read_text())
    except json.JSONDecodeError:
        issues.append("auth file JSON is invalid")
        return
    except OSError as exc:
        issues.append(f"auth file cannot be read: {exc}")
        return
    if not isinstance(data, dict):
        issues.append("auth file JSON is not an object")
        return
    missing_strings = [
        field
        for field in REQUIRED_AUTH_STRING_FIELDS
        if not isinstance(data.get(field), str) or not data.get(field)
    ]
    if missing_strings:
        issues.append(
            "auth file missing required auth field(s): "
            + ", ".join(missing_strings)
        )
    if "expires" not in data:
        issues.append("auth file missing required auth field(s): expires")
    elif not isinstance(data.get("expires"), (int, float, str)):
        issues.append("auth file has invalid expires field")


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}
