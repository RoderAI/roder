"""Offline tests for the Python chat-completions provider POC.

Runs with the standard library only (`python3 -m unittest discover tests`);
pytest also collects these tests. A local fake HTTP server stands in for
the OpenAI-compatible endpoint — no network access or credentials.
"""

from __future__ import annotations

import io
import json
import os
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, HTTPServer

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "src"))

from roder_python_chat_provider import (  # noqa: E402
    ChatCompletionsProvider,
    StdioRpc,
    fnv1a_checksum,
)
from roder_python_chat_provider.protocol import ALREADY_REPLIED  # noqa: E402


def sample_request() -> dict:
    return {
        "model": {"provider": "python-chat-completions", "model": "gpt-5.5"},
        "instructions": {"system": "be terse", "developer": "obey the harness"},
        "transcript": [
            {"UserMessage": {"text": "hello there"}},
            {"AssistantMessage": {"text": "hi!"}},
            {"UserMessage": {"text": "stream me"}},
        ],
        "tools": [
            {
                "name": "read_file",
                "description": "Read a file",
                "parameters": {"type": "object", "properties": {}},
            }
        ],
        "tool_choice": "Auto",
        "reasoning": {"enabled": False, "level": None},
        "output": {"max_tokens": 64, "temperature": 0.2},
        "runtime": {},
        "metadata": {},
    }


class RequestMappingTests(unittest.TestCase):
    def test_maps_canonical_request_fields(self) -> None:
        provider = ChatCompletionsProvider(api_key="test-key")
        body = provider.map_request(sample_request())

        self.assertEqual(body["model"], "gpt-5.5")
        self.assertTrue(body["stream"])
        self.assertEqual(body["max_completion_tokens"], 64)
        self.assertEqual(body["temperature"], 0.2)
        roles = [message["role"] for message in body["messages"]]
        self.assertEqual(roles, ["system", "developer", "user", "assistant", "user"])
        self.assertEqual(body["messages"][2]["content"], "hello there")
        self.assertEqual(body["tools"][0]["function"]["name"], "read_file")

    def test_model_override_wins(self) -> None:
        provider = ChatCompletionsProvider(api_key="k", model="custom-model")
        body = provider.map_request(sample_request())
        self.assertEqual(body["model"], "custom-model")
        self.assertEqual(provider.list_models()[0]["id"], "custom-model")


class FakeOpenAiHandler(BaseHTTPRequestHandler):
    captured: list[dict] = []

    def do_POST(self) -> None:  # noqa: N802 - http.server API
        length = int(self.headers.get("content-length", "0"))
        body = json.loads(self.rfile.read(length))
        type(self).captured.append(
            {"path": self.path, "auth": self.headers.get("authorization"), "body": body}
        )
        chunks = [
            {"id": "chatcmpl-1", "choices": [{"delta": {"content": "Hello "}}]},
            {"id": "chatcmpl-1", "choices": [{"delta": {"content": "world"}}]},
            {
                "id": "chatcmpl-1",
                "choices": [
                    {
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call-1",
                                    "function": {"name": "read_file", "arguments": "{\"pa"},
                                }
                            ]
                        }
                    }
                ],
            },
            {
                "id": "chatcmpl-1",
                "choices": [
                    {
                        "delta": {
                            "tool_calls": [
                                {"index": 0, "function": {"arguments": "th\":\"a.txt\"}"}}
                            ]
                        },
                        "finish_reason": "tool_calls",
                    }
                ],
            },
            {
                "id": "chatcmpl-1",
                "choices": [],
                "usage": {"prompt_tokens": 12, "completion_tokens": 5, "total_tokens": 17},
            },
        ]
        payload = "".join(f"data: {json.dumps(chunk)}\n\n" for chunk in chunks) + "data: [DONE]\n\n"
        encoded = payload.encode("utf-8")
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("content-length", str(len(encoded)))
        self.end_headers()
        self.wfile.write(encoded)

    def log_message(self, *args) -> None:  # noqa: D102 - silence test server
        return


class StreamingTests(unittest.TestCase):
    def setUp(self) -> None:
        FakeOpenAiHandler.captured = []
        self.server = HTTPServer(("127.0.0.1", 0), FakeOpenAiHandler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.base_url = f"http://127.0.0.1:{self.server.server_port}"

    def tearDown(self) -> None:
        self.server.shutdown()

    def test_streams_canonical_events_and_redacts_secrets(self) -> None:
        provider = ChatCompletionsProvider(api_key="super-secret-key", base_url=self.base_url)
        provider.record_roder_event({"kind": "turn.started"})
        provider.record_roder_event({"kind": "inference.started"})

        events: list[dict] = []
        provider.stream_turn(sample_request(), events.append)

        text = "".join(
            event["MessageDelta"]["text"] for event in events if "MessageDelta" in event
        )
        self.assertEqual(text, "Hello world")

        tool_calls = [event["ToolCallCompleted"] for event in events if "ToolCallCompleted" in event]
        self.assertEqual(len(tool_calls), 1)
        self.assertEqual(tool_calls[0]["name"], "read_file")
        self.assertEqual(json.loads(tool_calls[0]["arguments"]), {"path": "a.txt"})

        metadata = next(event["ProviderMetadata"] for event in events if "ProviderMetadata" in event)
        self.assertEqual(metadata["roder_events_observed"], 2)
        self.assertEqual(
            metadata["roder_event_kinds"], ["inference.started", "turn.started"]
        )

        usage = next(event["Usage"] for event in events if "Usage" in event)
        self.assertEqual(usage["total_tokens"], 17)
        completed = events[-1]["Completed"]
        self.assertEqual(completed["stop_reason"], "tool_calls")
        self.assertEqual(completed["provider_response_id"], "chatcmpl-1")

        # The wire request used bearer auth; no secrets appear in events.
        self.assertEqual(FakeOpenAiHandler.captured[0]["auth"], "Bearer super-secret-key")
        self.assertNotIn("super-secret-key", json.dumps(events))

    def test_http_failures_terminate_with_redacted_failed_event(self) -> None:
        provider = ChatCompletionsProvider(
            api_key="super-secret-key", base_url="http://127.0.0.1:9"
        )
        events: list[dict] = []
        provider.stream_turn(sample_request(), events.append)
        self.assertIn("Failed", events[-1])
        self.assertNotIn("super-secret-key", json.dumps(events))


class ProtocolTests(unittest.TestCase):
    def test_stdio_rpc_routes_requests_and_already_replied(self) -> None:
        stdin = io.StringIO(
            json.dumps({"jsonrpc": "2.0", "id": 1, "method": "x/echo", "params": {"v": 1}})
            + "\n"
            + json.dumps({"jsonrpc": "2.0", "id": 2, "method": "x/self", "params": {}})
            + "\n"
        )
        stdout = io.StringIO()
        rpc = StdioRpc(stdin=stdin, stdout=stdout)

        def handler(method, params, rpc_in, msg_id):
            if method == "x/echo":
                return {"echo": params["v"]}
            rpc_in.reply(msg_id, {"custom": True})
            return ALREADY_REPLIED

        rpc.run(handler)
        lines = [json.loads(line) for line in stdout.getvalue().strip().split("\n")]
        self.assertEqual(lines[0]["result"], {"echo": 1})
        self.assertEqual(lines[1]["result"], {"custom": True})
        self.assertEqual(len(lines), 2, "ALREADY_REPLIED must not double-reply")

    def test_checksum_matches_rust_fnv1a(self) -> None:
        # Mirrors roder_api::process_extension::manifest_checksum("roder").
        self.assertEqual(fnv1a_checksum(b""), "cbf29ce484222325")
        self.assertEqual(len(fnv1a_checksum(b"roder")), 16)


if __name__ == "__main__":
    unittest.main()
