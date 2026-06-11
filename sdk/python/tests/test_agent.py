from __future__ import annotations

from typing import Any

import pytest

from roder_sdk import InMemoryTransport, RoderAgent


@pytest.mark.anyio
async def test_agent_send_starts_thread_and_turn() -> None:
    requests: list[dict[str, Any]] = []

    async def handler(request: dict[str, Any]) -> dict[str, Any]:
        requests.append(request)
        if request["method"] == "workspace/list":
            return {"jsonrpc": "2.0", "id": request["id"], "result": {"workspaces": []}}
        if request["method"] == "workspace/create":
            return {"jsonrpc": "2.0", "id": request["id"], "result": {"workspace": {"id": "ws-1"}}}
        if request["method"] == "thread/start":
            return {"jsonrpc": "2.0", "id": request["id"], "result": {"thread": {"id": "thread-1"}}}
        if request["method"] == "turn/start":
            return {"jsonrpc": "2.0", "id": request["id"], "result": {"turn": {"id": "turn-1"}}}
        return {"jsonrpc": "2.0", "id": request["id"], "result": {}}

    agent = await RoderAgent.create(
        transport=InMemoryTransport(handler),
        cwd="/workspace",
        model={"provider": "openai", "id": "gpt-5.5"},
    )

    run = await agent.send("hello")

    assert run.thread_id == "thread-1"
    assert run.turn_id == "turn-1"
    assert requests[0]["method"] == "workspace/list"
    assert requests[1]["method"] == "workspace/create"
    assert requests[1]["params"] == {"roots": [{"path": "/workspace"}]}
    assert requests[2]["method"] == "thread/start"
    assert requests[2]["params"] == {
        "cwd": "/workspace",
        "model": "gpt-5.5",
        "modelProvider": "openai",
        "workspaceId": "ws-1",
    }
    assert requests[3]["method"] == "turn/start"
    assert requests[3]["params"] == {
        "threadId": "thread-1",
        "input": [{"type": "text", "text": "hello"}],
    }


@pytest.mark.anyio
async def test_agent_send_passes_developer_context_for_that_turn_only() -> None:
    requests: list[dict[str, Any]] = []

    async def handler(request: dict[str, Any]) -> dict[str, Any]:
        requests.append(request)
        if request["method"] == "thread/start":
            return {"jsonrpc": "2.0", "id": request["id"], "result": {"thread": {"id": "thread-1"}}}
        if request["method"] == "turn/start":
            return {"jsonrpc": "2.0", "id": request["id"], "result": {"turn": {"id": "turn-1"}}}
        return {"jsonrpc": "2.0", "id": request["id"], "result": {}}

    agent = await RoderAgent.create(
        transport=InMemoryTransport(handler),
        cwd="/workspace",
        workspace_id="ws-1",
    )

    await agent.send("hello", developer_context="Connected accounts: example-service.")
    await agent.send("and now?")

    turn_starts = [request for request in requests if request["method"] == "turn/start"]
    assert turn_starts[0]["params"] == {
        "threadId": "thread-1",
        "input": [{"type": "text", "text": "hello"}],
        "developerContext": "Connected accounts: example-service.",
    }
    # Omitting the argument keeps the param off the wire entirely.
    assert turn_starts[1]["params"] == {
        "threadId": "thread-1",
        "input": [{"type": "text", "text": "and now?"}],
    }


@pytest.mark.anyio
async def test_agent_safe_read_only_helpers() -> None:
    methods: list[str] = []
    agent = await RoderAgent.create(
        thread_id="thread-1",
        transport=InMemoryTransport(
            lambda request: methods.append(str(request["method"]))
            or {"jsonrpc": "2.0", "id": request["id"], "result": {"ok": True}}
        ),
    )

    await agent.list_models()
    await agent.list_providers()
    await agent.read_thread()
    await agent.list_threads()
    await agent.list_tools()
    await agent.list_commands()

    assert methods == [
        "model/list",
        "providers/list",
        "thread/read",
        "thread/list",
        "tools/list",
        "commands/list",
    ]
