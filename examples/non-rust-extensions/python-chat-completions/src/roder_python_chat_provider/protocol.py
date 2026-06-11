"""Newline-delimited JSON-RPC 2.0 stdio plumbing for Roder process
extensions.

The Rust host owns request ids; this module reads host messages from stdin,
routes them to a handler, and writes responses/notifications to stdout.
Diagnostics go to stderr only — stdout is reserved for protocol frames.
"""

from __future__ import annotations

import json
import sys
from typing import Any, Callable

PROTOCOL_VERSION = "0.1.0"

#: Sentinel a handler returns after replying to the request itself (e.g. to
#: acknowledge a stream before emitting its events).
ALREADY_REPLIED = object()

METHOD_INITIALIZE = "extension/initialize"
METHOD_LIST_MODELS = "inference/listModels"
METHOD_STREAM_TURN = "inference/streamTurn"
METHOD_INFERENCE_EVENT = "inference/event"
METHOD_EVENTS_HANDLE = "events/handle"
METHOD_EXTENSION_EVENT = "extension/event"
METHOD_SHUTDOWN = "extension/shutdown"


def fnv1a_checksum(data: bytes) -> str:
    """FNV-1a 64-bit hex checksum matching the Rust host implementation."""
    value = 0xCBF29CE484222325
    for byte in data:
        value ^= byte
        value = (value * 0x00000100000001B3) % (1 << 64)
    return f"{value:016x}"


class StdioRpc:
    """Blocking stdio JSON-RPC loop. `handler(method, params, rpc)` returns
    a result dict for requests; notifications return None implicitly."""

    def __init__(self, stdin=None, stdout=None) -> None:
        self._stdin = stdin or sys.stdin
        self._stdout = stdout or sys.stdout

    def reply(self, msg_id: Any, result: Any) -> None:
        self._write({"jsonrpc": "2.0", "id": msg_id, "result": result})

    def reply_error(self, msg_id: Any, message: str) -> None:
        self._write(
            {"jsonrpc": "2.0", "id": msg_id, "error": {"code": -32000, "message": message}}
        )

    def notify(self, method: str, params: Any) -> None:
        self._write({"jsonrpc": "2.0", "method": method, "params": params})

    def emit_inference_event(self, stream_id: str, event: dict) -> None:
        self.notify(METHOD_INFERENCE_EVENT, {"streamId": stream_id, "event": event})

    def emit_extension_event(
        self, extension_id: str, event_kind: str, payload: dict, schema_version: int = 1
    ) -> None:
        self.notify(
            METHOD_EXTENSION_EVENT,
            {
                "extensionId": extension_id,
                "eventKind": event_kind,
                "schemaVersion": schema_version,
                "payload": payload,
            },
        )

    def run(self, handler: Callable[[str, dict, "StdioRpc", Any], Any]) -> None:
        for line in self._stdin:
            line = line.strip()
            if not line:
                continue
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                print("dropped non-JSON host line", file=sys.stderr)
                continue
            method = message.get("method")
            msg_id = message.get("id")
            params = message.get("params") or {}
            if method is None:
                continue
            try:
                result = handler(method, params, self, msg_id)
            except ShutdownRequested:
                if msg_id is not None:
                    self.reply(msg_id, {})
                return
            except Exception as error:  # noqa: BLE001 - protocol boundary
                if msg_id is not None:
                    self.reply_error(msg_id, str(error))
                else:
                    print(f"notification {method} failed: {error}", file=sys.stderr)
                continue
            if msg_id is not None and result is not ALREADY_REPLIED:
                self.reply(msg_id, result if result is not None else {})

    def _write(self, message: dict) -> None:
        self._stdout.write(json.dumps(message) + "\n")
        self._stdout.flush()


class ShutdownRequested(Exception):
    """Raised by handlers to end the protocol loop gracefully."""
