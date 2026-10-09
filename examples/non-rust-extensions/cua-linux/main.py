#!/usr/bin/env python3
"""Limited Cua/Blaxel process-extension spike, not a production desktop backend."""

from __future__ import annotations

import json
import os
from pathlib import Path
import sys
import tomllib

from transport import BlaxelTransport, image_result

TOOLS = {
    "cua_list_windows": "list_windows",
    "cua_get_window_state": "get_window_state",
    "cua_get_desktop_state": "get_desktop_state",
    "cua_click": "click",
    "cua_press_key": "press_key",
}


class CuaExtension:
    def __init__(self, transport, manifest_path=None, allow_input=False):
        manifest_path = manifest_path or Path(__file__).with_name("roder-extension.toml")
        raw = Path(manifest_path).read_bytes()
        manifest = tomllib.loads(raw.decode())
        checksum = 0xCBF29CE484222325
        for byte in raw:
            checksum = ((checksum ^ byte) * 0x00000100000001B3) % (1 << 64)
        self.identity = {"protocolVersion": "0.2.0", "extensionId": manifest["id"],
                         "services": manifest["provides"], "manifestChecksum": f"{checksum:016x}"}
        self.transport = transport
        self.thread_id = None
        self.allow_input = allow_input

    def call_tool(self, params):
        tool = params.get("toolName")
        arguments = params.get("arguments", {})
        thread = params.get("threadId")
        if tool not in TOOLS or not isinstance(arguments, dict):
            raise ValueError("unknown tool or invalid arguments")
        if tool in ("cua_click", "cua_press_key") and not self.allow_input:
            raise ValueError("input disabled; the trusted disposable-evaluation launcher must opt in")
        if not isinstance(thread, str) or not thread:
            raise ValueError("threadId is required")
        # One child is deliberately bound to one thread/sandbox in this spike.
        # Reject a second thread rather than silently sharing its desktop.
        if self.thread_id is not None and self.thread_id != thread:
            raise ValueError("prototype is bound to another thread; launch a separate child/sandbox")
        allowed = {
            "cua_list_windows": set(),
            "cua_get_desktop_state": {"max_image_dimension"},
            "cua_get_window_state": {"pid", "window_id", "max_image_dimension"},
            "cua_click": {"pid", "window_id", "x", "y", "element_token", "capture_id", "delivery_mode"},
            "cua_press_key": {"pid", "window_id", "key", "delivery_mode"},
        }[tool]
        if set(arguments) - allowed:
            raise ValueError("unknown arguments; session and sandbox routing are host-owned")
        if tool in ("cua_get_window_state", "cua_click", "cua_press_key"):
            for field in ("pid", "window_id"):
                value = arguments.get(field)
                if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
                    raise ValueError(f"{field} must be a positive integer")
        if tool == "cua_click":
            token = arguments.get("element_token")
            if not token:
                for field in ("x", "y"):
                    value = arguments.get(field)
                    if isinstance(value, bool) or not isinstance(value, (float, int)) or not 0 <= value < 32768:
                        raise ValueError(f"{field} must be a finite nonnegative coordinate")
        if tool == "cua_press_key" and not isinstance(arguments.get("key"), str):
            raise ValueError("key is required")
        if arguments.get("delivery_mode", "background") not in ("background", "foreground"):
            raise ValueError("invalid delivery_mode")
        self.thread_id = thread
        # Repeat the explicit label so snapshot/element tokens survive separate
        # CLI calls, which otherwise get disposable implicit sessions.
        observation, failed = self.transport.call(TOOLS[tool], {**arguments, "session": "roder-cua-spike"})
        result = image_result(observation, failed)
        if tool in ("cua_get_window_state", "cua_get_desktop_state") and not failed:
            if "__view_image" not in result["data"]:
                raise RuntimeError("Cua capture returned no screenshot")
        return result


def serve(extension, stdin=sys.stdin, stdout=sys.stdout):
    for line in stdin:
        message = {}
        method = None
        try:
            decoded = json.loads(line)
            if not isinstance(decoded, dict):
                raise ValueError("host frame must be an object")
            message = decoded
            identifier = message.get("id")
            method = message.get("method")
            if identifier is None:
                continue
            if method == "extension/initialize":
                result = extension.identity
            elif method == "tools/call":
                result = extension.call_tool(message.get("params", {}))
            elif method == "extension/shutdown":
                result = {}
            else:
                raise ValueError("unsupported method")
            response = {"jsonrpc": "2.0", "id": identifier, "result": result}
        except Exception as error:  # Protocol boundary; diagnostics contain no credentials.
            response = {"jsonrpc": "2.0", "id": message.get("id"),
                        "error": {"code": -32000, "message": str(error)}}
        if response["id"] is None:
            print("dropped malformed host frame", file=sys.stderr)
            continue
        stdout.write(json.dumps(response) + "\n")
        stdout.flush()
        if method == "extension/shutdown":
            break


if __name__ == "__main__":
    transport = BlaxelTransport(os.environ["RODER_CUA_SANDBOX"], os.environ["RODER_CUA_WORKSPACE"])
    serve(CuaExtension(transport, os.environ.get("RODER_EXTENSION_MANIFEST"),
                       allow_input=os.environ.get("RODER_CUA_ALLOW_INPUT") == "1"))
