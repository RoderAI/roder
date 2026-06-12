#!/usr/bin/env python3
"""Roder process extension (stdlib only) contributing model-callable tools.

Speaks the Roder process-extension protocol 0.2.0 over newline-delimited
JSON-RPC on stdio: `extension/initialize` echoes the manifest identity,
declared services, and FNV-1a manifest checksum; `tools/call` serves the
`word_count` tool the manifest declares statically; `extension/shutdown`
ends the loop. Diagnostics go to stderr only — stdout carries protocol
frames.

The manifest path comes from `RODER_EXTENSION_MANIFEST` (default:
`roder-extension.toml` next to this file), matching the
python-chat-completions example conventions.
"""

from __future__ import annotations

import json
import os
import sys
import tomllib

PROTOCOL_VERSION = "0.2.0"

METHOD_INITIALIZE = "extension/initialize"
METHOD_TOOLS_CALL = "tools/call"
METHOD_SHUTDOWN = "extension/shutdown"


def fnv1a_checksum(data: bytes) -> str:
    """FNV-1a 64-bit hex checksum matching the Rust host implementation."""
    value = 0xCBF29CE484222325
    for byte in data:
        value ^= byte
        value = (value * 0x00000100000001B3) % (1 << 64)
    return f"{value:016x}"


def default_manifest_path() -> str:
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "roder-extension.toml")


class PythonToolsExtension:
    """Reads the manifest it ships with and serves the tools it declares.

    Echoing `provides` straight from the manifest keeps the initialize echo
    in lockstep with what the host parsed — the host fails closed on any
    drift in id, services, or checksum.
    """

    def __init__(self, manifest_path: str) -> None:
        with open(manifest_path, "rb") as fh:
            manifest_bytes = fh.read()
        manifest = tomllib.loads(manifest_bytes.decode("utf-8"))
        self.extension_id = manifest["id"]
        self.services = manifest["provides"]
        self.manifest_checksum = fnv1a_checksum(manifest_bytes)

    def initialize(self) -> dict:
        return {
            "protocolVersion": PROTOCOL_VERSION,
            "extensionId": self.extension_id,
            "services": self.services,
            "manifestChecksum": self.manifest_checksum,
        }

    def call_tool(self, params: dict) -> dict:
        tool_name = params.get("toolName")
        arguments = params.get("arguments") or {}
        if tool_name == "word_count":
            count = len(str(arguments.get("text", "")).split())
            return {"content": f"{count} words", "isError": False}
        raise ValueError(f"unknown tool {tool_name!r}")


def serve(extension: PythonToolsExtension, stdin=None, stdout=None) -> None:
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout

    def write(message: dict) -> None:
        stdout.write(json.dumps(message) + "\n")
        stdout.flush()

    for line in stdin:
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
            if method == METHOD_INITIALIZE:
                result = extension.initialize()
            elif method == METHOD_TOOLS_CALL:
                result = extension.call_tool(params)
            elif method == METHOD_SHUTDOWN:
                if msg_id is not None:
                    write({"jsonrpc": "2.0", "id": msg_id, "result": {}})
                return
            else:
                raise ValueError(f"unknown method {method}")
        except Exception as error:  # noqa: BLE001 - protocol boundary
            if msg_id is not None:
                write(
                    {
                        "jsonrpc": "2.0",
                        "id": msg_id,
                        "error": {"code": -32000, "message": str(error)},
                    }
                )
            else:
                print(f"notification {method} failed: {error}", file=sys.stderr)
            continue
        if msg_id is not None:
            write({"jsonrpc": "2.0", "id": msg_id, "result": result})


def main() -> None:
    manifest_path = os.environ.get("RODER_EXTENSION_MANIFEST", default_manifest_path())
    serve(PythonToolsExtension(manifest_path))


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
