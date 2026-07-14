#!/usr/bin/env python3
"""Streaming iteration, redaction, truncation, and feature scanning for Roder event logs.

Shared by ``roder_trajectory_export.py`` (ATIF trajectory conversion) and
``tbench_hygiene.py`` (analyzer completion-hygiene labels). Everything here is
streaming-safe: event logs can be multiple megabytes, so callers iterate line by
line and never load the whole file into memory.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterator

# ---------------------------------------------------------------------------
# Redaction
# ---------------------------------------------------------------------------

# Order matters: specific token shapes first, broad high-entropy catch-all last,
# so a value already replaced by a named rule is not re-counted by the catch-all.
_REDACTION_RULES: tuple[tuple[re.Pattern[str], str], ...] = (
    # JSON web tokens (header.payload.signature) — access tokens are often JWTs.
    (re.compile(r"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{4,}"), "[REDACTED_JWT]"),
    # Bearer authorization values.
    (re.compile(r"(?i)bearer\s+[A-Za-z0-9._~+/=-]{12,}"), "Bearer [REDACTED]"),
    # Authorization / Proxy-Authorization headers.
    (re.compile(r"(?i)\b(proxy-)?authorization\s*[:=]\s*[^\s,'\"]+"), "authorization: [REDACTED]"),
    # OpenAI-style secret keys (sk-..., sk-proj-...).
    (re.compile(r"sk-[A-Za-z0-9_-]{16,}"), "[REDACTED_API_KEY]"),
    # GitHub personal / app tokens.
    (re.compile(r"gh[posru]_[A-Za-z0-9]{20,}"), "[REDACTED_TOKEN]"),
    # AWS access key ids.
    (re.compile(r"AKIA[0-9A-Z]{16}"), "[REDACTED_AWS_KEY]"),
    # Cookie / Set-Cookie header lines.
    (re.compile(r"(?i)\b(set-)?cookie\s*[:=]\s*[^\r\n]+"), "cookie: [REDACTED]"),
    # key/token/secret assignments in JSON, env, query, or CLI form.
    (
        re.compile(
            r"(?i)\b(api[_-]?key|apikey|access[_-]?token|refresh[_-]?token|"
            r"secret[_-]?key|client[_-]?secret|access|refresh|token|secret|"
            r"password|passwd|pwd)"
            r"(\s*[:=]\s*|\"\s*:\s*\")"
            r"([A-Za-z0-9._~+/=-]{6,})"
        ),
        r"\1\2[REDACTED]",
    ),
    # Credentials embedded in query strings.
    (
        re.compile(r"(?i)([?&](?:key|token|access_token|api_key|sig|signature)=)[A-Za-z0-9._~+/=-]+"),
        r"\1[REDACTED]",
    ),
    # High-entropy base64-ish blobs (catches long token bodies with no delimiter).
    (re.compile(r"[A-Za-z0-9_/+-]{100,}={0,2}"), "[REDACTED_LONG_TOKEN]"),
)

# Per-string cap; longer content is truncated head+tail so trajectories stay small.
MAX_FIELD_CHARS = 12000
_TRUNCATE_HEAD = 6000
_TRUNCATE_TAIL = 4000


@dataclass
class Redactor:
    """Applies secret redaction and giant-output truncation, counting both."""

    redactions: int = 0
    truncations: int = 0
    max_field_chars: int = MAX_FIELD_CHARS

    def text(self, value: str) -> str:
        if not value:
            return value
        redacted = value
        for pattern, replacement in _REDACTION_RULES:
            redacted, count = pattern.subn(replacement, redacted)
            self.redactions += count
        return self.truncate(redacted)

    def truncate(self, value: str) -> str:
        if len(value) <= self.max_field_chars:
            return value
        self.truncations += 1
        removed = len(value) - _TRUNCATE_HEAD - _TRUNCATE_TAIL
        head = value[:_TRUNCATE_HEAD]
        tail = value[-_TRUNCATE_TAIL:]
        return f"{head}\n...[TRUNCATED {removed} chars]...\n{tail}"

    def value(self, obj: Any) -> Any:
        """Recursively redact string values inside dicts/lists (e.g. tool arguments)."""
        if isinstance(obj, str):
            return self.text(obj)
        if isinstance(obj, dict):
            return {key: self.value(val) for key, val in obj.items()}
        if isinstance(obj, list):
            return [self.value(item) for item in obj]
        return obj


# ---------------------------------------------------------------------------
# Streaming event iteration
# ---------------------------------------------------------------------------

# Incremental streaming deltas: only the terminal item.completed carries final
# content, so skipping updated lines is both correct and much cheaper.
_SKIP_PREFIXES = ('{"type":"item.updated"', '{"type": "item.updated"')


def iter_events(path: Path) -> Iterator[dict[str, Any]]:
    """Yield parsed events from a Roder JSONL log, skipping deltas and bad lines."""
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            line = line.strip()
            if not line or line.startswith(_SKIP_PREFIXES):
                continue
            try:
                event = json.loads(line)
            except (json.JSONDecodeError, ValueError):
                continue
            if isinstance(event, dict):
                yield event


def item_type(item: Any) -> str | None:
    if isinstance(item, dict):
        value = item.get("type")
        return str(value) if value else None
    return None


# ---------------------------------------------------------------------------
# Feature scanning
# ---------------------------------------------------------------------------

# Validation-shaped command fragments (generic, not task-specific).
_VALIDATION_MARKERS = (
    "pytest",
    "unittest",
    "make test",
    "make check",
    "run_tests",
    "run-tests",
    "./test",
    "bats ",
    "cargo test",
    "go test",
    "npm test",
    "npm run test",
    "ctest",
    "tox",
    "nose",
    "python -m pytest",
    "assert",
    " diff ",
    "diff -",
    "cmp ",
    "verify",
    "validate",
)

# Write-shaped command fragments — a file was (re)written to disk.
_WRITE_MARKERS = (
    " > ",
    ">>",
    "tee ",
    "cp ",
    "mv ",
    "install ",
    "cat >",
    "cat <<",
    "<<'",
    '<<"',
    "sed -i",
    "touch ",
    "dd ",
    "make ",
)

_WRITE_TOOLS = {"apply_patch", "write_file", "write_stdin"}


def _command_text(payload: Any) -> str:
    if not isinstance(payload, dict):
        return ""
    for key in ("cmd", "command", "patch", "script"):
        value = payload.get(key)
        if isinstance(value, str) and value:
            return value
    return ""


@dataclass
class TrajectoryFeatures:
    """Compact, evaluation-neutral behavioural summary of one Roder trajectory."""

    model_calls: int = 0
    tool_calls: int = 0
    failed_tools: int = 0
    verification_reviews: int = 0
    web_searches: int = 0
    agent_messages: int = 0
    reasoning_blocks: int = 0
    tool_counts: dict[str, int] = field(default_factory=dict)
    tool_failures: dict[str, int] = field(default_factory=dict)
    last_tool_name: str | None = None
    last_tool_status: str | None = None
    last_successful_action: str | None = None
    tools_since_last_success: int | None = None
    turn_failed: bool = False
    error_texts: list[str] = field(default_factory=list)
    has_validation_after_last_write: bool = False
    validation_commands: int = 0

    def as_dict(self) -> dict[str, Any]:
        data: dict[str, Any] = {
            "model_calls": self.model_calls,
            "tool_calls": self.tool_calls,
            "failed_tools": self.failed_tools,
            "verification_reviews": self.verification_reviews,
            "web_searches": self.web_searches,
            "agent_messages": self.agent_messages,
            "reasoning_blocks": self.reasoning_blocks,
            # Distinguishes "local checks attempted but not after the last write"
            # from "no local checks visible" (both otherwise fold into no_local_validation).
            "validation_commands": self.validation_commands,
            "validated_after_last_write": self.has_validation_after_last_write,
        }
        if self.tool_counts:
            data["tool_counts"] = dict(sorted(self.tool_counts.items()))
        if self.last_tool_name is not None:
            data["last_tool_name"] = self.last_tool_name
        if self.last_tool_status is not None:
            data["last_tool_status"] = self.last_tool_status
        if self.last_successful_action is not None:
            data["last_successful_action"] = self.last_successful_action
        if self.tools_since_last_success is not None:
            data["tools_since_last_success"] = self.tools_since_last_success
        if self.turn_failed:
            data["turn_failed"] = True
        if self.error_texts:
            data["error_texts"] = self.error_texts
        return data


def scan_features(events_path: Path, redactor: Redactor | None = None) -> TrajectoryFeatures:
    """Single streaming pass over a Roder event log producing behavioural counts."""
    redactor = redactor or Redactor()
    features = TrajectoryFeatures()

    tool_ordinal = -1
    last_success_ordinal: int | None = None
    last_write_ordinal = -1
    last_validation_after_write = False
    saw_validation = False
    # Command arguments live on item.started; item.completed carries the output.
    started_payloads: dict[str, Any] = {}

    for event in iter_events(events_path):
        etype = event.get("type")
        if etype == "turn.failed":
            features.turn_failed = True
            continue
        if etype == "item.started":
            item = event.get("item")
            if item_type(item) == "toolExecution":
                key = str(item.get("id") or item.get("tool_call_id") or "")
                if key:
                    started_payloads[key] = item.get("payload")
            continue
        if etype != "item.completed":
            continue
        item = event.get("item")
        if not isinstance(item, dict):
            continue
        kind = item.get("type")

        if kind == "reasoning":
            features.reasoning_blocks += 1
            continue
        if kind == "agentMessage":
            features.agent_messages += 1
            continue
        if kind == "error":
            raw = item.get("text")
            if isinstance(raw, str) and raw.strip():
                features.error_texts.append(redactor.text(raw))
            continue
        if kind == "raw":
            payload = item.get("payload")
            provider = payload.get("ProviderMetadata") if isinstance(payload, dict) else None
            if isinstance(provider, dict) and provider.get("segment") == "assistant":
                features.model_calls += 1
            continue
        if kind != "toolExecution":
            continue

        # --- tool execution ---
        tool_ordinal += 1
        name = str(item.get("tool_name") or "unknown")
        status = str(item.get("status") or "")
        features.tool_calls += 1
        features.tool_counts[name] = features.tool_counts.get(name, 0) + 1
        features.last_tool_name = name
        features.last_tool_status = status or None
        if name == "verification_review":
            features.verification_reviews += 1
        elif name == "web_search":
            features.web_searches += 1

        failed = status == "failed"
        if failed:
            features.failed_tools += 1
            features.tool_failures[name] = features.tool_failures.get(name, 0) + 1
        else:
            last_success_ordinal = tool_ordinal
            snippet = (item.get("text") or "").strip().splitlines()
            head = snippet[0][:120] if snippet else ""
            features.last_successful_action = f"{name}: {head}".strip().rstrip(":")

        key = str(item.get("id") or item.get("tool_call_id") or "")
        payload = started_payloads.pop(key, None) or item.get("payload")
        command = _command_text(payload)
        is_write = name in _WRITE_TOOLS or _matches(command, _WRITE_MARKERS)
        is_validation = (
            (name in ("exec_command", "shell") and _matches(command, _VALIDATION_MARKERS))
            or (name == "verification_review" and status != "failed")
        )
        if is_validation:
            saw_validation = True
            features.validation_commands += 1
        if is_write:
            last_write_ordinal = tool_ordinal
            last_validation_after_write = False
        elif is_validation and tool_ordinal > last_write_ordinal:
            last_validation_after_write = True

    if last_success_ordinal is not None:
        features.tools_since_last_success = tool_ordinal - last_success_ordinal
    if last_write_ordinal >= 0:
        features.has_validation_after_last_write = last_validation_after_write
    else:
        features.has_validation_after_last_write = saw_validation
    if features.model_calls == 0:
        features.model_calls = features.agent_messages
    return features


def _matches(text: str, markers: tuple[str, ...]) -> bool:
    lowered = text.lower()
    return any(marker in lowered for marker in markers)
