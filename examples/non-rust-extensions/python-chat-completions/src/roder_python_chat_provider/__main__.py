"""Entry point: `python -m roder_python_chat_provider`.

Speaks the Roder process-extension protocol over stdio and serves the
OpenAI-compatible chat-completions provider. Configuration comes from
explicit env vars only (the host forwards an allowlist):

- `PY_CHAT_COMPLETIONS_API_KEY` (required for real turns)
- `PY_CHAT_COMPLETIONS_BASE_URL` (default `https://api.openai.com/v1`)
- `PY_CHAT_COMPLETIONS_MODEL` (optional model override)
- `RODER_EXTENSION_MANIFEST` (manifest path; default `roder-extension.toml`
  relative to the configured cwd)
"""

from __future__ import annotations

import os
import sys

from .protocol import (
    ALREADY_REPLIED,
    METHOD_EVENTS_HANDLE,
    METHOD_INITIALIZE,
    METHOD_LIST_MODELS,
    METHOD_SHUTDOWN,
    METHOD_STREAM_TURN,
    PROTOCOL_VERSION,
    ShutdownRequested,
    StdioRpc,
    fnv1a_checksum,
)
from .provider import DEFAULT_BASE_URL, ChatCompletionsProvider

EXTENSION_ID = "roder-ext-python-chat-completions"
SERVICES = [
    {"type": "inference_engine", "id": "python-chat-completions"},
    {"type": "event_sink", "id": "python-chat-completions-events"},
]


def main() -> None:
    manifest_path = os.environ.get("RODER_EXTENSION_MANIFEST", "roder-extension.toml")
    with open(manifest_path, "rb") as fh:
        manifest_checksum = fnv1a_checksum(fh.read())

    provider = ChatCompletionsProvider(
        api_key=os.environ.get("PY_CHAT_COMPLETIONS_API_KEY", "unset"),
        base_url=os.environ.get("PY_CHAT_COMPLETIONS_BASE_URL", DEFAULT_BASE_URL),
        model=os.environ.get("PY_CHAT_COMPLETIONS_MODEL"),
    )

    def handler(method: str, params: dict, rpc: StdioRpc, msg_id):
        if method == METHOD_INITIALIZE:
            return {
                "protocolVersion": PROTOCOL_VERSION,
                "extensionId": EXTENSION_ID,
                "services": SERVICES,
                "manifestChecksum": manifest_checksum,
            }
        if method == METHOD_LIST_MODELS:
            return {"models": provider.list_models()}
        if method == METHOD_STREAM_TURN:
            # Acknowledge the stream before emitting events so the host's
            # request future resolves immediately and events flow into the
            # already-registered stream receiver.
            stream_id = params["streamId"]
            rpc.reply(msg_id, {"streamId": stream_id})
            provider.stream_turn(
                params.get("request") or {},
                lambda event: rpc.emit_inference_event(stream_id, event),
            )
            return ALREADY_REPLIED
        if method == METHOD_EVENTS_HANDLE:
            provider.record_roder_event(params.get("envelope") or {})
            rpc.emit_extension_event(
                EXTENSION_ID,
                "provider.events_observed",
                {"count": provider.observed_event_count},
            )
            return None
        if method == METHOD_SHUTDOWN:
            raise ShutdownRequested()
        raise ValueError(f"unknown method {method}")

    StdioRpc().run(handler)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        sys.exit(130)
