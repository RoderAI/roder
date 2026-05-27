"""Dependency snapshot checks for Harbor launch-plan validation."""

from __future__ import annotations

from typing import Any


def validate_required_sha256(
    issues: list[str],
    value: dict[str, Any],
    field: str,
    *,
    label: str | None = None,
) -> None:
    name = label or field
    digest = value.get(field)
    if not isinstance(digest, str) or not digest:
        issues.append(f"{name} is missing")
    elif len(digest) != 64 or any(char not in "0123456789abcdef" for char in digest):
        issues.append(f"{name} is not a lowercase SHA-256 hex digest")


def validate_config_hashes_match(issues: list[str], plan: dict[str, Any]) -> None:
    harbor_config_sha = plan.get("harborConfigSha256")
    pre_eval_config_sha = plan.get("preEvalHarborConfigSha256")
    if (
        isinstance(harbor_config_sha, str)
        and isinstance(pre_eval_config_sha, str)
        and harbor_config_sha
        and pre_eval_config_sha
        and harbor_config_sha != pre_eval_config_sha
    ):
        issues.append("pre-eval Harbor config SHA-256 mismatch")


def validate_required_dependency_snapshots(
    issues: list[str],
    plan: dict[str, Any],
) -> None:
    validate_prebuilt_snapshot(issues, dict_value(plan.get("prebuiltBinary")))
    validate_auth_snapshot(issues, dict_value(plan.get("authFile")))
    validate_harness_snapshot(issues, dict_value(plan.get("harborHarness")))


def validate_prebuilt_snapshot(issues: list[str], snapshot: dict[str, Any]) -> None:
    if not snapshot:
        issues.append("prebuiltBinary is missing")
        return
    if not isinstance(snapshot.get("path"), str) or not snapshot.get("path"):
        issues.append("prebuiltBinary.path is missing")
    validate_required_sha256(issues, snapshot, "sha256", label="prebuiltBinary.sha256")


def validate_auth_snapshot(issues: list[str], snapshot: dict[str, Any]) -> None:
    if not snapshot:
        issues.append("authFile is missing")
        return
    if not isinstance(snapshot.get("path"), str) or not snapshot.get("path"):
        issues.append("authFile.path is missing")
    if snapshot.get("validJson") is not True:
        issues.append("authFile.validJson is not true")


def validate_harness_snapshot(issues: list[str], snapshot: dict[str, Any]) -> None:
    if not snapshot:
        issues.append("harborHarness is missing")
        return
    if snapshot.get("status") != "passed":
        issues.append(f"harborHarness status is {snapshot.get('status') or '<missing>'}")
    validate_required_sha256(
        issues,
        snapshot,
        "combinedSha256",
        label="harborHarness.combinedSha256",
    )
    if not list_value(snapshot.get("entries")):
        issues.append("harborHarness.entries is missing")


def dict_value(value: Any) -> dict[str, Any]:
    return value if isinstance(value, dict) else {}


def list_value(value: Any) -> list[Any]:
    return value if isinstance(value, list) else []
