#!/usr/bin/env python3
"""Validate local Harbor/Roder eval config invariants before a new run."""

from __future__ import annotations

import argparse
import json
import os
import sys
from pathlib import Path
from typing import Any

from tbench_deadline_policy import TBENCH_DEADLINE_POLICY


DEFAULT_CONFIGS = (
    Path("evals/harbor/tbench-full-gpt55-medium.json"),
    Path("evals/harbor/tbench-smoke.json"),
    Path("evals/harbor/tbench-gemini35-flash-validation.json"),
)
REQUIRED_GITIGNORE_ENTRIES = (
    "evals/harbor/artifacts/",
    "evals/harbor/jobs/",
    "evals/reports/",
)
REQUIRED_ARTIFACTS = (
    "/logs/agent/roder-cli.txt",
    "/logs/agent/roder-events.jsonl",
    "/logs/agent/roder-stderr.txt",
    "/logs/agent/roder-last-message.txt",
    "/logs/agent/setup-summary.txt",
    "/logs/agent/roder-run-summary.json",
)
REQUIRED_AUTH_STRING_FIELDS = ("access", "refresh", "account_id", "type")


def validate_config(path: Path, config: dict[str, Any]) -> list[str]:
    issues: list[str] = []
    agent = first_agent(path, config, issues)
    kwargs = agent.get("kwargs") if isinstance(agent.get("kwargs"), dict) else {}
    job_name = str(config.get("job_name") or path.stem)
    is_smoke = "smoke" in job_name

    expect_equal(issues, path, "timeout_multiplier", config.get("timeout_multiplier"), 1.0)
    expect_equal(
        issues,
        path,
        "environment.delete",
        nested(config, "environment", "delete"),
        False,
    )
    expect_equal(
        issues,
        path,
        "agents[0].override_timeout_sec",
        agent.get("override_timeout_sec"),
        TBENCH_DEADLINE_POLICY.override_timeout_sec,
    )
    expect_equal(
        issues,
        path,
        "agents[0].kwargs.soft_timeout_sec",
        kwargs.get("soft_timeout_sec"),
        TBENCH_DEADLINE_POLICY.soft_timeout_sec,
    )
    expect_equal(
        issues,
        path,
        "agents[0].kwargs.speed_policy_eval_deadline_seconds",
        kwargs.get("speed_policy_eval_deadline_seconds"),
        TBENCH_DEADLINE_POLICY.eval_deadline_seconds,
    )
    expect_equal(
        issues,
        path,
        "agents[0].kwargs.speed_policy_enabled",
        kwargs.get("speed_policy_enabled"),
        False,
    )
    expect_equal(
        issues,
        path,
        "agents[0].kwargs.task_ledger_required",
        kwargs.get("task_ledger_required"),
        True,
    )
    expect_equal(
        issues,
        path,
        "agents[0].kwargs.benchmark_guidance_enabled",
        kwargs.get("benchmark_guidance_enabled"),
        True,
    )
    expect_equal(
        issues,
        path,
        "agents[0].kwargs.policy_mode",
        kwargs.get("policy_mode"),
        "bypass",
    )
    expect_equal(
        issues,
        path,
        "agents[0].kwargs.include_prebuilt_binary",
        kwargs.get("include_prebuilt_binary"),
        "true",
    )
    expect_equal(
        issues,
        path,
        "agents[0].kwargs.include_local_source",
        kwargs.get("include_local_source"),
        "false",
    )
    expect_equal(
        issues,
        path,
        "orchestrator.n_concurrent_trials",
        nested(config, "orchestrator", "n_concurrent_trials"),
        1 if is_smoke else 4,
    )

    artifacts = config.get("artifacts")
    if not isinstance(artifacts, list):
        issues.append(f"{path}: artifacts must be a list")
    else:
        missing = [artifact for artifact in REQUIRED_ARTIFACTS if artifact not in artifacts]
        for artifact in missing:
            issues.append(f"{path}: missing artifact {artifact}")

    return issues


def validate_gitignore(text: str) -> list[str]:
    lines = {line.strip() for line in text.splitlines()}
    return [
        f".gitignore: missing ignored generated output {entry}"
        for entry in REQUIRED_GITIGNORE_ENTRIES
        if entry not in lines
    ]


def validate_prebuilt_binary(path: Path, required: bool) -> list[str]:
    if not required:
        return []
    configured = os.environ.get("RODER_HARBOR_PREBUILT_BINARY")
    binary = Path(configured).expanduser() if configured else path
    if not binary.exists():
        return [f"prebuilt Roder binary missing: {binary}"]
    if not os.access(binary, os.X_OK):
        return [f"prebuilt Roder binary is not executable: {binary}"]
    if not is_linux_x86_64_elf(binary):
        return [f"prebuilt Roder binary must be a Linux x86-64 ELF: {binary}"]
    return []


def validate_auth_file(path: Path, required: bool) -> list[str]:
    if not required:
        return []
    configured = os.environ.get("RODER_HARBOR_AUTH_FILE")
    auth_file = Path(configured).expanduser() if configured else path.expanduser()
    if not auth_file.exists():
        return [f"auth file missing: {auth_file}"]
    try:
        data = json.loads(auth_file.read_text())
    except Exception as exc:
        return [f"auth file is not valid JSON: {auth_file}: {exc}"]
    if not isinstance(data, dict):
        return [f"auth file must contain a JSON object: {auth_file}"]
    missing_strings = [
        field
        for field in REQUIRED_AUTH_STRING_FIELDS
        if not isinstance(data.get(field), str) or not data.get(field)
    ]
    if missing_strings:
        return [
            f"auth file missing required auth field(s): {', '.join(missing_strings)}: {auth_file}"
        ]
    if "expires" not in data:
        return [f"auth file missing required auth field(s): expires: {auth_file}"]
    if not isinstance(data.get("expires"), (int, float, str)):
        return [f"auth file has invalid expires field: {auth_file}"]
    return []


def is_linux_x86_64_elf(path: Path) -> bool:
    try:
        header = path.read_bytes()[:64]
    except OSError:
        return False
    if len(header) < 20:
        return False
    if header[:4] != b"\x7fELF":
        return False
    elf_class = header[4]
    data_encoding = header[5]
    os_abi = header[7]
    machine = int.from_bytes(header[18:20], "little")
    return (
        elf_class == 2
        and data_encoding == 1
        and machine == 0x3E
        and os_abi in (0, 3)
    )


def load_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text())


def first_agent(path: Path, config: dict[str, Any], issues: list[str]) -> dict[str, Any]:
    agents = config.get("agents")
    if not isinstance(agents, list) or not agents:
        issues.append(f"{path}: agents must contain at least one agent")
        return {}
    agent = agents[0]
    if not isinstance(agent, dict):
        issues.append(f"{path}: agents[0] must be an object")
        return {}
    return agent


def nested(value: dict[str, Any], *keys: str) -> Any:
    current: Any = value
    for key in keys:
        if not isinstance(current, dict):
            return None
        current = current.get(key)
    return current


def expect_equal(
    issues: list[str], path: Path, name: str, actual: Any, expected: Any
) -> None:
    if actual != expected:
        issues.append(f"{path}: {name} expected {expected!r}, got {actual!r}")


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--config",
        type=Path,
        action="append",
        default=[],
        help="Harbor config to validate; defaults to checked-in smoke and full configs.",
    )
    parser.add_argument("--gitignore", type=Path, default=Path(".gitignore"))
    parser.add_argument(
        "--require-prebuilt",
        action="store_true",
        help="Require the injected Roder binary to already exist and be executable.",
    )
    parser.add_argument(
        "--prebuilt-binary",
        type=Path,
        default=Path("evals/harbor/artifacts/roder-linux-amd64"),
    )
    parser.add_argument(
        "--require-auth",
        action="store_true",
        help="Require the Codex auth file that Harbor will upload for roder.",
    )
    parser.add_argument(
        "--auth-file",
        type=Path,
        default=Path("~/.roder/auth/codex.json"),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    config_paths = tuple(args.config) or DEFAULT_CONFIGS
    issues: list[str] = []

    for path in config_paths:
        try:
            issues.extend(validate_config(path, load_json(path)))
        except Exception as exc:
            issues.append(f"{path}: failed to load config: {exc}")

    try:
        issues.extend(validate_gitignore(args.gitignore.read_text()))
    except Exception as exc:
        issues.append(f"{args.gitignore}: failed to read gitignore: {exc}")

    issues.extend(validate_prebuilt_binary(args.prebuilt_binary, args.require_prebuilt))
    issues.extend(validate_auth_file(args.auth_file, args.require_auth))

    if issues:
        print("Harbor readiness validation failed:", file=sys.stderr)
        for issue in issues:
            print(f"- {issue}", file=sys.stderr)
        return 1

    print(f"Harbor readiness validation passed: {len(config_paths)} configs")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
