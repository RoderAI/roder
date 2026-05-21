from __future__ import annotations

from typing import Any

import anyio
import pytest

from roder_sdk import InMemoryTransport, RoderAgent


@pytest.mark.anyio
async def test_agent_resolves_approval_user_input_and_plan_exit_callbacks() -> None:
    methods: list[str] = []
    transport = InMemoryTransport(
        lambda request: methods.append(str(request["method"]))
        or {"jsonrpc": "2.0", "id": request["id"], "result": {"ok": True}}
    )

    async def on_tool_approval(request: Any) -> dict[str, Any]:
        assert request == {"approvalId": "approval-1", "toolName": "fs/readFile"}
        return {"approved": True, "message": "read-only"}

    agent = await RoderAgent.create(
        transport=transport,
        approvals={
            "on_tool_approval": on_tool_approval,
            "on_user_input": lambda request: {"response": "answer"},
            "on_plan_exit": lambda request: {"accepted": True, "message": "done"},
        },
    )

    await transport.emit(
        {
            "jsonrpc": "2.0",
            "method": "session/approvalRequested",
            "params": {"approvalId": "approval-1", "toolName": "fs/readFile"},
        }
    )
    await transport.emit(
        {"jsonrpc": "2.0", "method": "session/userInputRequested", "params": {"requestId": "input-1"}}
    )
    await transport.emit(
        {"jsonrpc": "2.0", "method": "session/planExitRequested", "params": {"requestId": "plan-1"}}
    )

    for _ in range(20):
        if {
            "session/resolve_approval",
            "session/resolve_user_input",
            "session/exit_plan",
        }.issubset(methods):
            break
        await anyio.sleep(0.005)

    assert "session/resolve_approval" in methods
    assert "session/resolve_user_input" in methods
    assert "session/exit_plan" in methods
    await agent.close()
