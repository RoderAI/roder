#!/usr/bin/env python3
"""Fake process-extension child for offline host tests.

Implements the newline-delimited JSON-RPC process-extension protocol:
initialize echo (manifest checksum via FNV-1a), model listing, a small
streamed inference turn, events/handle recording (reported back through an
extension/event notification), and graceful shutdown.
"""

import json
import os
import sys


def fnv1a(data: bytes) -> str:
    h = 0xCBF29CE484222325
    for byte in data:
        h ^= byte
        h = (h * 0x00000100000001B3) % (1 << 64)
    return f"{h:016x}"


def reply(msg_id, result):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "id": msg_id, "result": result}) + "\n")
    sys.stdout.flush()


def reply_error(msg_id, message):
    sys.stdout.write(
        json.dumps({"jsonrpc": "2.0", "id": msg_id, "error": {"code": -32000, "message": message}})
        + "\n"
    )
    sys.stdout.flush()


def notify(method, params):
    sys.stdout.write(json.dumps({"jsonrpc": "2.0", "method": method, "params": params}) + "\n")
    sys.stdout.flush()


def main() -> None:
    manifest_path = os.environ["FAKE_CHILD_MANIFEST"]
    with open(manifest_path, "r", encoding="utf-8") as fh:
        manifest_toml = fh.read()
    extension_id = os.environ.get("FAKE_CHILD_ID", "roder-ext-fake-child")
    checksum = (
        os.environ.get("FAKE_CHILD_BAD_CHECKSUM")
        or fnv1a(manifest_toml.encode("utf-8"))
    )
    handled_events = []

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        msg = json.loads(line)
        method = msg.get("method")
        msg_id = msg.get("id")
        params = msg.get("params") or {}

        if method == "extension/initialize":
            reply(
                msg_id,
                {
                    "protocolVersion": params["protocolVersion"],
                    "extensionId": extension_id,
                    "services": [
                        {"type": "inference_engine", "id": "fake-process-engine"},
                        {"type": "event_sink", "id": "fake-process-events"},
                    ],
                    "manifestChecksum": checksum,
                },
            )
        elif method == "inference/listModels":
            reply(
                msg_id,
                {
                    "models": [
                        {"id": "fake-model", "name": "Fake Process Model", "context_window": 4096}
                    ]
                },
            )
        elif method == "inference/streamTurn":
            sid = params["streamId"]
            reply(msg_id, {"streamId": sid})
            prompt_items = params["request"]["transcript"]
            notify(
                "inference/event",
                {"streamId": sid, "event": {"MessageDelta": {"text": "hello from "}}},
            )
            notify(
                "inference/event",
                {"streamId": sid, "event": {"MessageDelta": {"text": "the fake child"}}},
            )
            notify(
                "inference/event",
                {
                    "streamId": sid,
                    "event": {
                        "ProviderMetadata": {
                            "transcript_items": len(prompt_items),
                            "events_seen": len(handled_events),
                        }
                    },
                },
            )
            notify(
                "inference/event",
                {
                    "streamId": sid,
                    "event": {
                        "Usage": {
                            "prompt_tokens": 7,
                            "completion_tokens": 4,
                            "total_tokens": 11,
                        }
                    },
                },
            )
            notify(
                "inference/event",
                {
                    "streamId": sid,
                    "event": {
                        "Completed": {"stop_reason": "stop", "provider_response_id": "fake-1"}
                    },
                },
            )
        elif method == "events/handle":
            handled_events.append(params["envelope"]["kind"])
            notify(
                "extension/event",
                {
                    "extensionId": extension_id,
                    "eventKind": "fake.events_observed",
                    "schemaVersion": 1,
                    "payload": {"kinds": handled_events},
                },
            )
        elif method == "extension/shutdown":
            reply(msg_id, {})
            return
        elif msg_id is not None:
            reply_error(msg_id, f"unknown method {method}")


if __name__ == "__main__":
    main()
