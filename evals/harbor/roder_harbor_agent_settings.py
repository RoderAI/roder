"""Kwarg and environment parsing for the Harbor Roder agent."""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any

from roder_harbor_agent_config import (
    optional_bool,
    optional_float,
    optional_int,
    optional_int_list,
    str_tuple,
)

DEFAULT_MODEL = "codex/gpt-5.5"
DEFAULT_SOURCE_ROOTS = ("Cargo.toml", "Cargo.lock", ".cargo", "crates")
DEFAULT_PLAN_FIRST_SOFT_TIMEOUT_SEC = 360
DEFAULT_POLICY_BLOCK_MAX_RETRIES = 2


@dataclass(frozen=True)
class RoderAgentSettings:
    provider: str | None
    reasoning: str
    policy_mode: str
    source_dir: Path
    auth_file: Path
    include_local_source: bool
    include_prebuilt_binary: bool
    prebuilt_binary: Path
    prebuilt_binary_amd64: Path
    prebuilt_binary_arm64: Path
    benchmark_guidance_enabled: bool
    task_ledger_required: bool
    plan_first_enabled: bool
    plan_first_policy_mode: str
    plan_first_reasoning: str
    plan_first_soft_timeout_sec: int | None
    source_roots: tuple[str, ...]
    soft_timeout_sec: int | None
    per_task_deadlines: bool
    agent_timeout_multiplier_hint: float | None
    task_cache_dir: Path | None
    policy_block_max_retries: int
    speed_policy_enabled: bool | None
    speed_policy_eval_deadline_seconds: int | None
    speed_policy_reasoning: dict[str, Any]
    reliability: dict[str, Any]
    tool_allowlist: tuple[str, ...]


def parse_agent_settings(kwargs: dict[str, Any], *, source_dir_default: Path) -> RoderAgentSettings:
    guidance = optional_bool(_kwarg_or_env(kwargs, "benchmark_guidance_enabled"))
    task_ledger = optional_bool(_kwarg_or_env(kwargs, "task_ledger_required"))
    plan_first = optional_bool(_kwarg_or_env(kwargs, "plan_first_enabled"))
    policy_mode = str(kwargs.get("policy_mode", "bypass"))
    reasoning = str(kwargs.get("reasoning", "medium"))
    plan_first_enabled = False if plan_first is None else plan_first
    plan_first_soft_timeout_sec = optional_int(
        kwargs.get("plan_first_soft_timeout_sec")
        or os.environ.get("RODER_HARBOR_PLAN_FIRST_SOFT_TIMEOUT_SEC")
    )
    if plan_first_enabled and plan_first_soft_timeout_sec is None:
        plan_first_soft_timeout_sec = DEFAULT_PLAN_FIRST_SOFT_TIMEOUT_SEC
    soft_timeout = kwargs.get("soft_timeout_sec") or os.environ.get(
        "RODER_HARBOR_SOFT_TIMEOUT_SEC"
    )
    per_task_deadlines = optional_bool(_kwarg_or_env(kwargs, "per_task_deadlines"))
    multiplier_hint = optional_float(
        _kwarg_or_env(kwargs, "agent_timeout_multiplier_hint")
    )
    task_cache_dir = kwargs.get("task_cache_dir") or os.environ.get(
        "RODER_HARBOR_TASK_CACHE_DIR"
    )
    policy_block_max_retries = optional_int(
        _kwarg_or_env(kwargs, "policy_block_max_retries")
    )
    return RoderAgentSettings(
        provider=kwargs.get("provider"),
        reasoning=reasoning,
        policy_mode=policy_mode,
        source_dir=_path_setting(
            kwargs.get("source_dir") or os.environ.get("RODER_HARBOR_SOURCE_DIR"),
            default=source_dir_default,
        ),
        auth_file=_path_setting(
            kwargs.get("auth_file") or os.environ.get("RODER_HARBOR_AUTH_FILE"),
            default=Path("~/.roder/auth/codex.json"),
        ),
        include_local_source=_flag_not_disabled(kwargs.get("include_local_source", "true")),
        include_prebuilt_binary=_flag_not_disabled(
            kwargs.get("include_prebuilt_binary", "true")
        ),
        prebuilt_binary=_path_setting(
            kwargs.get("prebuilt_binary")
            or os.environ.get("RODER_HARBOR_PREBUILT_BINARY"),
            default=source_dir_default / "evals/harbor/artifacts/roder-linux-amd64",
        ),
        prebuilt_binary_amd64=_path_setting(
            kwargs.get("prebuilt_binary_amd64")
            or os.environ.get("RODER_HARBOR_PREBUILT_BINARY_AMD64"),
            default=source_dir_default / "evals/harbor/artifacts/roder-linux-amd64",
        ),
        prebuilt_binary_arm64=_path_setting(
            kwargs.get("prebuilt_binary_arm64")
            or os.environ.get("RODER_HARBOR_PREBUILT_BINARY_ARM64"),
            default=source_dir_default / "evals/harbor/artifacts/roder-linux-arm64",
        ),
        benchmark_guidance_enabled=True if guidance is None else guidance,
        task_ledger_required=False if task_ledger is None else task_ledger,
        plan_first_enabled=plan_first_enabled,
        plan_first_policy_mode=str(
            kwargs.get("plan_first_policy_mode")
            or os.environ.get("RODER_HARBOR_PLAN_FIRST_POLICY_MODE")
            or policy_mode
        ),
        plan_first_reasoning=str(
            kwargs.get("plan_first_reasoning")
            or os.environ.get("RODER_HARBOR_PLAN_FIRST_REASONING")
            or reasoning
        ),
        plan_first_soft_timeout_sec=plan_first_soft_timeout_sec,
        source_roots=tuple(kwargs.get("source_roots", DEFAULT_SOURCE_ROOTS)),
        soft_timeout_sec=int(float(soft_timeout)) if soft_timeout else None,
        per_task_deadlines=False if per_task_deadlines is None else per_task_deadlines,
        agent_timeout_multiplier_hint=multiplier_hint,
        task_cache_dir=Path(str(task_cache_dir)).expanduser() if task_cache_dir else None,
        policy_block_max_retries=(
            DEFAULT_POLICY_BLOCK_MAX_RETRIES
            if policy_block_max_retries is None
            else policy_block_max_retries
        ),
        speed_policy_enabled=optional_bool(_kwarg_or_env(kwargs, "speed_policy_enabled")),
        speed_policy_eval_deadline_seconds=optional_int(
            _kwarg_or_env(kwargs, "speed_policy_eval_deadline_seconds")
        ),
        speed_policy_reasoning={
            "orientation_reasoning": kwargs.get("speed_policy_orientation_reasoning")
            or os.environ.get("RODER_HARBOR_SPEED_POLICY_ORIENTATION_REASONING"),
            "execution_reasoning": kwargs.get("speed_policy_execution_reasoning")
            or os.environ.get("RODER_HARBOR_SPEED_POLICY_EXECUTION_REASONING"),
            "verification_reasoning": kwargs.get("speed_policy_verification_reasoning")
            or os.environ.get("RODER_HARBOR_SPEED_POLICY_VERIFICATION_REASONING"),
            "recovery_reasoning": kwargs.get("speed_policy_recovery_reasoning")
            or os.environ.get("RODER_HARBOR_SPEED_POLICY_RECOVERY_REASONING"),
        },
        reliability={
            "provider_retry_max_attempts": optional_int(
                _kwarg_or_env(kwargs, "reliability_provider_retry_max_attempts")
            ),
            "provider_retry_initial_backoff_ms": optional_int(
                _kwarg_or_env(kwargs, "reliability_provider_retry_initial_backoff_ms")
            ),
            "provider_retry_backoff_factor": optional_float(
                _kwarg_or_env(kwargs, "reliability_provider_retry_backoff_factor")
            ),
            "provider_retry_status_codes": optional_int_list(
                _kwarg_or_env(kwargs, "reliability_provider_retry_status_codes")
            ),
            "retry_empty_provider_body": optional_bool(
                _kwarg_or_env(kwargs, "reliability_retry_empty_provider_body")
            ),
            "max_consecutive_tool_failures": optional_int(
                _kwarg_or_env(kwargs, "reliability_max_consecutive_tool_failures")
            ),
            "max_tool_failures_per_turn": optional_int(
                _kwarg_or_env(kwargs, "reliability_max_tool_failures_per_turn")
            ),
            "max_model_calls_per_turn": optional_int(
                _kwarg_or_env(kwargs, "reliability_max_model_calls_per_turn")
            ),
        },
        tool_allowlist=str_tuple(_kwarg_or_env(kwargs, "tool_allowlist")),
    )


def _kwarg_or_env(kwargs: dict[str, Any], key: str) -> Any:
    if key in kwargs:
        return kwargs[key]
    return os.environ.get(f"RODER_HARBOR_{key.upper()}")


def _path_setting(value: Any, *, default: Path) -> Path:
    return Path(value).expanduser() if value else Path(default).expanduser()


def _flag_not_disabled(value: Any) -> bool:
    return str(value).lower() not in {"0", "false", "no"}
