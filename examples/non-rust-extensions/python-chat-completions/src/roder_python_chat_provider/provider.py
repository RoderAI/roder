"""OpenAI-compatible chat-completions provider mapped to Roder's canonical
inference contract.

Maps `AgentInferenceRequest` to `/chat/completions` with `stream: true`,
converts SSE chunks into canonical `InferenceEvent` payloads, and records
host-forwarded Roder events as redacted provider metadata (kind names and
counts only — never prompts, keys, or headers).

Stdlib-only on purpose: offline tests and the Rust e2e proof must not
install packages.
"""

from __future__ import annotations

import json
import urllib.request
from collections import deque
from typing import Callable, Iterator

DEFAULT_BASE_URL = "https://api.openai.com/v1"
MAX_RECORDED_EVENT_KINDS = 50


class ChatCompletionsProvider:
    def __init__(self, api_key: str, base_url: str = DEFAULT_BASE_URL, model: str | None = None):
        if not api_key:
            raise ValueError("PY_CHAT_COMPLETIONS_API_KEY is required")
        self.api_key = api_key
        self.base_url = base_url.rstrip("/")
        self.model = model
        self.observed_event_kinds: deque[str] = deque(maxlen=MAX_RECORDED_EVENT_KINDS)
        self.observed_event_count = 0

    # -- Roder event sink side -------------------------------------------

    def record_roder_event(self, envelope: dict) -> None:
        """Records only the canonical event kind; payloads may contain
        prompts and are never retained."""
        self.observed_event_count += 1
        self.observed_event_kinds.append(str(envelope.get("kind", "unknown")))

    # -- inference side ---------------------------------------------------

    def list_models(self) -> list[dict]:
        model = self.model or "gpt-5.5"
        return [{"id": model, "name": model, "context_window": None}]

    def map_request(self, request: dict) -> dict:
        """Maps the canonical `AgentInferenceRequest` JSON to a
        chat-completions request body."""
        messages: list[dict] = []
        instructions = request.get("instructions") or {}
        if instructions.get("system"):
            messages.append({"role": "system", "content": instructions["system"]})
        if instructions.get("developer"):
            messages.append({"role": "developer", "content": instructions["developer"]})
        for item in request.get("transcript", []):
            if "UserMessage" in item:
                messages.append({"role": "user", "content": item["UserMessage"]["text"]})
            elif "AssistantMessage" in item:
                messages.append(
                    {"role": "assistant", "content": item["AssistantMessage"]["text"]}
                )
            elif "ToolResult" in item:
                result = item["ToolResult"]
                messages.append(
                    {
                        "role": "tool",
                        "tool_call_id": result.get("id", ""),
                        "content": json.dumps(result.get("result")),
                    }
                )
        body: dict = {
            "model": self.model or (request.get("model") or {}).get("model") or "gpt-5.5",
            "messages": messages,
            "stream": True,
            "stream_options": {"include_usage": True},
        }
        output = request.get("output") or {}
        if output.get("max_tokens") is not None:
            body["max_completion_tokens"] = output["max_tokens"]
        if output.get("temperature") is not None:
            body["temperature"] = output["temperature"]
        tools = request.get("tools") or []
        if tools:
            body["tools"] = [
                {
                    "type": "function",
                    "function": {
                        "name": tool["name"],
                        "description": tool.get("description", ""),
                        "parameters": tool.get("parameters", {}),
                    },
                }
                for tool in tools
            ]
        return body

    def stream_turn(self, request: dict, emit: Callable[[dict], None]) -> None:
        """Streams one turn, calling `emit` with canonical InferenceEvent
        JSON values and always terminating with Completed or Failed."""
        try:
            body = self.map_request(request)
            tool_calls: dict[int, dict] = {}
            finish_reason: str | None = None
            usage: dict | None = None
            response_id: str | None = None

            for chunk in self._sse_chunks(body):
                response_id = chunk.get("id") or response_id
                if chunk.get("usage"):
                    usage = chunk["usage"]
                for choice in chunk.get("choices", []):
                    if choice.get("finish_reason"):
                        finish_reason = choice["finish_reason"]
                    delta = choice.get("delta") or {}
                    if delta.get("content"):
                        emit({"MessageDelta": {"text": delta["content"]}})
                    for call in delta.get("tool_calls") or []:
                        slot = tool_calls.setdefault(
                            call.get("index", 0), {"id": "", "name": "", "arguments": ""}
                        )
                        if call.get("id"):
                            slot["id"] = call["id"]
                        function = call.get("function") or {}
                        if function.get("name"):
                            slot["name"] = function["name"]
                        if function.get("arguments"):
                            slot["arguments"] += function["arguments"]

            for slot in tool_calls.values():
                emit(
                    {
                        "ToolCallCompleted": {
                            "id": slot["id"],
                            "name": slot["name"],
                            "arguments": slot["arguments"],
                        }
                    }
                )
            emit(
                {
                    "ProviderMetadata": {
                        "provider": "python-chat-completions",
                        "roder_events_observed": self.observed_event_count,
                        "roder_event_kinds": sorted(set(self.observed_event_kinds)),
                    }
                }
            )
            if usage:
                emit(
                    {
                        "Usage": {
                            "prompt_tokens": usage.get("prompt_tokens", 0),
                            "completion_tokens": usage.get("completion_tokens", 0),
                            "total_tokens": usage.get("total_tokens", 0),
                        }
                    }
                )
            emit(
                {
                    "Completed": {
                        "stop_reason": finish_reason,
                        "provider_response_id": response_id,
                    }
                }
            )
        except Exception as error:  # noqa: BLE001 - terminal failure path
            emit({"Failed": {"message": _redact(str(error), self.api_key)}})

    def _sse_chunks(self, body: dict) -> Iterator[dict]:
        request = urllib.request.Request(
            f"{self.base_url}/chat/completions",
            data=json.dumps(body).encode("utf-8"),
            headers={
                "content-type": "application/json",
                "authorization": f"Bearer {self.api_key}",
                "accept": "text/event-stream",
            },
            method="POST",
        )
        with urllib.request.urlopen(request) as response:
            for raw in response:
                line = raw.decode("utf-8", "replace").strip()
                if not line.startswith("data:"):
                    continue
                data = line[len("data:") :].strip()
                if data == "[DONE]":
                    break
                try:
                    yield json.loads(data)
                except json.JSONDecodeError:
                    continue


def _redact(message: str, secret: str) -> str:
    return message.replace(secret, "<redacted>") if secret else message
