from __future__ import annotations

from typing import Any

import pytest

from roder_sdk import InMemoryTransport, RoderRpcClient, RoderRun


@pytest.mark.anyio
async def test_run_streams_until_turn_completed() -> None:
    transport = InMemoryTransport(lambda request: {"jsonrpc": "2.0", "id": request["id"], "result": {}})
    run = RoderRun(RoderRpcClient(transport), "thread-1", "turn-1")
    events: list[str] = []

    async def collect() -> None:
        async for event in run.stream():
            events.append(event["type"])

    import asyncio

    task = asyncio.create_task(collect())
    await transport.emit({"jsonrpc": "2.0", "method": "turn/delta", "params": {"turnId": "turn-1"}})
    await transport.emit({"jsonrpc": "2.0", "method": "turn/completed", "params": {"turnId": "turn-1"}})
    await task

    assert events == ["turn.delta", "turn.completed"]


@pytest.mark.anyio
async def test_run_cancel_maps_to_turn_interrupt() -> None:
    interrupt_params: Any = None

    async def handler(request: dict[str, Any]) -> dict[str, Any]:
        nonlocal interrupt_params
        if request["method"] == "turn/interrupt":
            interrupt_params = request.get("params")
        return {"jsonrpc": "2.0", "id": request["id"], "result": {"interrupted": True}}

    run = RoderRun(RoderRpcClient(InMemoryTransport(handler)), "thread-1", "turn-1")

    assert await run.cancel("stop") == {"interrupted": True}
    assert interrupt_params == {"threadId": "thread-1", "turnId": "turn-1", "reason": "stop"}
