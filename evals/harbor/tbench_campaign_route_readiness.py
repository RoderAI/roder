"""Route agent-readiness checks for generated Terminal-Bench campaigns."""

from __future__ import annotations

from typing import Any

from tbench_deadline_policy import TBENCH_DEADLINE_POLICY


def validate_route_config_agent_readiness(
    result: Any,
    name: str,
    agent: dict[str, Any],
    kwargs: dict[str, Any],
) -> None:
    expected = (
        (
            "agents[0].override_timeout_sec",
            agent.get("override_timeout_sec"),
            TBENCH_DEADLINE_POLICY.override_timeout_sec,
        ),
        (
            "agents[0].kwargs.soft_timeout_sec",
            kwargs.get("soft_timeout_sec"),
            TBENCH_DEADLINE_POLICY.soft_timeout_sec,
        ),
        (
            "agents[0].kwargs.speed_policy_eval_deadline_seconds",
            kwargs.get("speed_policy_eval_deadline_seconds"),
            TBENCH_DEADLINE_POLICY.eval_deadline_seconds,
        ),
        ("agents[0].kwargs.speed_policy_enabled", kwargs.get("speed_policy_enabled"), False),
        ("agents[0].kwargs.task_ledger_required", kwargs.get("task_ledger_required"), True),
        (
            "agents[0].kwargs.benchmark_guidance_enabled",
            kwargs.get("benchmark_guidance_enabled"),
            True,
        ),
        ("agents[0].kwargs.policy_mode", kwargs.get("policy_mode"), "bypass"),
        (
            "agents[0].kwargs.include_prebuilt_binary",
            kwargs.get("include_prebuilt_binary"),
            "true",
        ),
        (
            "agents[0].kwargs.include_local_source",
            kwargs.get("include_local_source"),
            "false",
        ),
    )
    for field, actual, expected_value in expected:
        if actual != expected_value:
            result.add(
                f"route {name} {field} expected {expected_value!r}, got {actual!r}"
            )
