"""Config parsing and TOML rendering helpers for the Harbor Roder agent."""

from __future__ import annotations

import json
from typing import Any


def optional_bool(value: Any) -> bool | None:
    if value is None:
        return None
    if isinstance(value, bool):
        return value
    text = str(value).strip().lower()
    if text in {"1", "true", "yes", "on"}:
        return True
    if text in {"0", "false", "no", "off"}:
        return False
    raise ValueError(f"invalid boolean value: {value!r}")


def optional_int(value: Any) -> int | None:
    if value is None or value == "":
        return None
    return int(float(value))


def optional_float(value: Any) -> float | None:
    if value is None or value == "":
        return None
    return float(value)


def optional_int_list(value: Any) -> list[int] | None:
    if value is None or value == "":
        return None
    if isinstance(value, (list, tuple)):
        return [int(item) for item in value]
    return [int(part.strip()) for part in str(value).split(",") if part.strip()]


def str_tuple(value: Any) -> tuple[str, ...]:
    """Parse a comma-separated string or list/tuple into a tuple of names."""
    if value is None or value == "":
        return ()
    if isinstance(value, (list, tuple)):
        return tuple(str(item).strip() for item in value if str(item).strip())
    return tuple(part.strip() for part in str(value).split(",") if part.strip())


def tools_config_toml(allowlist: tuple[str, ...]) -> str:
    """Render a ``[tools]`` section pinning the advertised tool allow-list.

    Roder gates tools by allow-list: empty means every tool is advertised;
    a non-empty list restricts the model to exactly those tools. Excluding the
    failure-prone auxiliary tools (``media_attach``, ``chrome_*``) keeps repeated
    tool-call failures from tripping ``max_consecutive_tool_failures`` and ending
    the turn early. Empty allowlist renders nothing (default behavior).
    """
    if not allowlist:
        return ""
    rendered = ", ".join(json.dumps(name) for name in allowlist)
    return f"\n[tools]\nallowlist = [{rendered}]\n"


def speed_policy_config_toml(
    *,
    enabled: bool | None,
    eval_deadline_seconds: int | None,
    reasoning: dict[str, Any],
) -> str:
    lines: list[str] = []
    if enabled is not None:
        lines.append(f"enabled = {str(enabled).lower()}")
    if eval_deadline_seconds is not None:
        lines.append(f"eval_deadline_seconds = {eval_deadline_seconds}")
    for key, value in reasoning.items():
        if value:
            lines.append(f"{key} = {json.dumps(str(value))}")
    if not lines:
        return ""
    return "\n[speed_policy]\n" + "\n".join(lines) + "\n"


def reliability_config_toml(values: dict[str, Any]) -> str:
    lines = [
        f"{key} = {toml_value(value)}"
        for key, value in values.items()
        if value is not None
    ]
    if not lines:
        return ""
    return "\n[reliability]\n" + "\n".join(lines) + "\n"


def toml_value(value: Any) -> str:
    if isinstance(value, bool):
        return str(value).lower()
    return json.dumps(value)
