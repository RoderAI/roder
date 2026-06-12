"""Offline tests for the Python tools process-extension example.

Runs with the standard library only (`python3 -m unittest discover -s tests`).
Drives the stdio JSON-RPC loop with in-memory streams — no host, no network.
"""

from __future__ import annotations

import io
import json
import os
import sys
import tomllib
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from main import (  # noqa: E402
    PROTOCOL_VERSION,
    PythonToolsExtension,
    default_manifest_path,
    fnv1a_checksum,
    serve,
)


def run_protocol(messages: list[dict]) -> list[dict]:
    stdin = io.StringIO("".join(json.dumps(message) + "\n" for message in messages))
    stdout = io.StringIO()
    serve(PythonToolsExtension(default_manifest_path()), stdin=stdin, stdout=stdout)
    return [json.loads(line) for line in stdout.getvalue().strip().split("\n") if line]


class ProtocolTests(unittest.TestCase):
    def test_initialize_echoes_manifest_identity_services_and_checksum(self) -> None:
        with open(default_manifest_path(), "rb") as fh:
            manifest_bytes = fh.read()
        manifest = tomllib.loads(manifest_bytes.decode("utf-8"))

        replies = run_protocol(
            [{"jsonrpc": "2.0", "id": 1, "method": "extension/initialize", "params": {}}]
        )
        result = replies[0]["result"]
        self.assertEqual(result["protocolVersion"], PROTOCOL_VERSION)
        self.assertEqual(PROTOCOL_VERSION, "0.2.0")
        self.assertEqual(result["extensionId"], "python-tools")
        self.assertEqual(result["services"], manifest["provides"])
        self.assertEqual(result["manifestChecksum"], fnv1a_checksum(manifest_bytes))

    def test_word_count_tool_counts_words(self) -> None:
        replies = run_protocol(
            [
                {
                    "jsonrpc": "2.0",
                    "id": 7,
                    "method": "tools/call",
                    "params": {
                        "providerId": "python-tools",
                        "toolName": "word_count",
                        "callId": "call-1",
                        "threadId": "thread-1",
                        "turnId": "turn-1",
                        "arguments": {"text": "  counting words is   fun "},
                    },
                }
            ]
        )
        self.assertEqual(replies[0]["result"], {"content": "4 words", "isError": False})

    def test_unknown_tool_returns_json_rpc_error(self) -> None:
        replies = run_protocol(
            [
                {
                    "jsonrpc": "2.0",
                    "id": 8,
                    "method": "tools/call",
                    "params": {"toolName": "nope", "arguments": {}},
                }
            ]
        )
        self.assertIn("unknown tool", replies[0]["error"]["message"])

    def test_shutdown_replies_and_ends_the_loop(self) -> None:
        replies = run_protocol(
            [
                {"jsonrpc": "2.0", "id": 9, "method": "extension/shutdown", "params": {}},
                {"jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": {}},
            ]
        )
        self.assertEqual(len(replies), 1, "no replies after shutdown")
        self.assertEqual(replies[0]["result"], {})


if __name__ == "__main__":
    unittest.main()
