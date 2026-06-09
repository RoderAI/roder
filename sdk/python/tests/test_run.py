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
    await transport.emit(
        {
            "jsonrpc": "2.0",
            "method": "item/agentMessage/delta",
            "params": {
                "seq": 1,
                "eventId": "turn-1-item-event-1",
                "threadId": "thread-1",
                "turnId": "turn-1",
                "timestamp": "1970-01-01T00:00:00Z",
                "event": {
                    "type": "itemDelta",
                    "itemId": "turn-1-agent-final_answer",
                    "delta": {"type": "agentMessageText", "delta": "hello"},
                },
            },
        }
    )
    await transport.emit(
        {
            "jsonrpc": "2.0",
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {"id": "turn-1", "items": [], "itemsView": "default", "status": "completed"},
            },
        }
    )
    await task

    assert events == ["item.delta", "turn.completed"]


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
