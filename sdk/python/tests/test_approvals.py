from __future__ import annotations

from typing import Any

import anyio
import pytest

from roder_sdk import InMemoryTransport, RoderAgent


@pytest.mark.anyio
async def test_agent_resolves_approval_user_input_and_plan_exit_callbacks() -> None:
    methods: list[str] = []
    requests: list[dict[str, Any]] = []

    def handle_request(request: dict[str, Any]) -> dict[str, Any]:
        methods.append(str(request["method"]))
        requests.append({"method": request["method"], "params": request.get("params")})
        return {"jsonrpc": "2.0", "id": request["id"], "result": {"ok": True}}

    transport = InMemoryTransport(
        handle_request
    )

    async def on_tool_approval(request: Any) -> dict[str, Any]:
        assert request == {"approvalId": "approval-1", "toolName": "fs/readFile"}
        return {"approved": True}

    agent = await RoderAgent.create(
        transport=transport,
        approvals={
            "on_tool_approval": on_tool_approval,
            "on_user_input": lambda request: {"answers": "answer"},
            "on_plan_exit": lambda request: {"approved": True},
        },
    )

    await transport.emit(
        {
            "jsonrpc": "2.0",
            "method": "thread/approvalRequested",
            "params": {"approvalId": "approval-1", "toolName": "fs/readFile"},
        }
    )
    await transport.emit(
        {"jsonrpc": "2.0", "method": "thread/userInputRequested", "params": {"requestId": "input-1"}}
    )
    await transport.emit(
        {"jsonrpc": "2.0", "method": "thread/planExitRequested", "params": {"requestId": "plan-1"}}
    )

    for _ in range(20):
        if {
            "thread/resolve_approval",
            "thread/resolve_user_input",
            "thread/exit_plan",
        }.issubset(methods):
            break
        await anyio.sleep(0.005)

    assert "thread/resolve_approval" in methods
    assert "thread/resolve_user_input" in methods
    assert "thread/exit_plan" in methods
    assert requests == [
        {
            "method": "thread/resolve_approval",
            "params": {"approvalId": "approval-1", "approved": True},
        },
        {
            "method": "thread/resolve_user_input",
            "params": {"requestId": "input-1", "answers": "answer"},
        },
        {
            "method": "thread/exit_plan",
            "params": {"requestId": "plan-1", "approved": True},
        },
    ]
    await agent.close()
