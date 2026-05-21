from __future__ import annotations

import json
import sys
from typing import Any

import anyio
import pytest

from roder_sdk import InMemoryTransport, LocalProcessTransport, WebSocketTransport


@pytest.mark.anyio
async def test_in_memory_transport_preserves_notification_order() -> None:
    transport = InMemoryTransport(
        lambda request: {"jsonrpc": "2.0", "id": request["id"], "result": {"ok": True}}
    )
    notifications = transport.notifications()

    await transport.emit({"jsonrpc": "2.0", "method": "first", "params": {"n": 1}})
    await transport.emit({"jsonrpc": "2.0", "method": "second", "params": {"n": 2}})

    assert (await anext(notifications))["method"] == "first"
    assert (await anext(notifications))["method"] == "second"
    await transport.close()


@pytest.mark.anyio
async def test_local_process_transport_exchanges_json_lines_without_roder_binary() -> None:
    script = """
import json
import sys

print(json.dumps({"jsonrpc": "2.0", "method": "process/ready", "params": {}}), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": {"method": request["method"], "params": request.get("params")}}), flush=True)
"""
    transport = await LocalProcessTransport.create(command=sys.executable, args=["-c", script])
    notifications = transport.notifications()

    response = await transport.request(
        {
            "jsonrpc": "2.0",
            "id": "req-1",
            "method": "commands/list",
            "params": {"limit": 1},
        }
    )
    assert (await anext(notifications))["method"] == "process/ready"
    assert response["result"] == {"method": "commands/list", "params": {"limit": 1}}
    await transport.close()


@pytest.mark.anyio
async def test_websocket_transport_sends_bearer_headers_and_resolves_response() -> None:
    socket = FakeSocket()
    calls: list[dict[str, Any]] = []

    async def connector(url: str, **kwargs: Any) -> FakeSocket:
        calls.append({"url": url, **kwargs})
        return socket

    transport = await WebSocketTransport.connect(
        "ws://127.0.0.1:1234",
        token="secret-token",
        connector=connector,
    )
    result: dict[str, Any] | None = None

    async def request() -> None:
        nonlocal result
        result = await transport.request({"jsonrpc": "2.0", "id": 7, "method": "providers/list"})

    async with anyio.create_task_group() as task_group:
        task_group.start_soon(request)
        for _ in range(20):
            if socket.sent:
                break
            await anyio.sleep(0.001)
        assert json.loads(socket.sent[0])["method"] == "providers/list"
        await socket.push({"jsonrpc": "2.0", "id": 7, "result": {"providers": []}})

    assert result is not None
    assert result["result"] == {"providers": []}
    assert calls[0]["additional_headers"] == {"Authorization": "Bearer secret-token"}
    await transport.close()


class FakeSocket:
    def __init__(self) -> None:
        self.sent: list[str] = []
        self._messages: list[str] = []

    async def send(self, data: str) -> None:
        self.sent.append(data)

    async def recv(self) -> str:
        while not self._messages:
            await _sleep()
        return self._messages.pop(0)

    async def close(self) -> None:
        pass

    async def push(self, message: dict[str, Any]) -> None:
        self._messages.append(json.dumps(message))


async def _sleep() -> None:
    import anyio

    await anyio.sleep(0.001)
