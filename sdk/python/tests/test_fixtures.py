from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import anyio
import pytest

from roder_sdk import InMemoryTransport, RoderAgent, RoderRpcClient

FIXTURE_ROOT = Path(__file__).resolve().parents[2] / "fixtures" / "fake-app-server"


@pytest.mark.anyio
async def test_python_sdk_replays_basic_thread_fixture() -> None:
    fixture = load_fixture("basic-thread.jsonl")
    transport = fixture_transport(fixture)
    client = RoderRpcClient(transport)

    assert (await client.call("initialize", {}))["provider"] == "mock"
    assert (await client.methods["providers/list"]())["providers"][0]["id"] == "mock"
    assert (await client.methods["model/list"]())["models"][0]["id"] == "mock"

    agent = await RoderAgent.create(
        transport=transport,
        cwd="/workspace",
        model={"provider": "mock", "id": "mock"},
        tool_allowlist=["edit", "read_file"],
        instructions="You are embedded in a host app.",
    )
    run = await agent.send("hello")
    events: list[str] = []

    async def collect() -> None:
        async for event in run.stream():
            events.append(event["type"])

    async with anyio.create_task_group() as task_group:
        task_group.start_soon(collect)
        await emit_notifications(transport, fixture)

    assert events == ["item.started", "item.delta", "item.completed", "turn.completed"]


@pytest.mark.anyio
async def test_python_sdk_replays_approval_fixture() -> None:
    fixture = load_fixture("approval-flow.jsonl")
    transport = fixture_transport(fixture)
    seen: list[str] = []

    async def on_tool_approval(request: Any) -> dict[str, Any]:
        seen.append(request["approvalId"])
        return {"approved": True}

    agent = await RoderAgent.create(
        transport=transport,
        cwd="/workspace",
        model={"provider": "mock", "id": "mock"},
        approvals={"on_tool_approval": on_tool_approval},
    )

    await agent.send("read file")
    await emit_notifications(transport, fixture)
    await eventually(lambda: "approval-1" in seen and "thread/resolve_approval" in transport.seen_methods)


@pytest.mark.anyio
async def test_python_sdk_replays_runner_thread_fixture() -> None:
    fixture = load_fixture("runner-thread-flow.jsonl")
    transport = fixture_transport(fixture)
    agent = await RoderAgent.create(
        transport=transport,
        cwd="/local/scratch",
        workspace_id="ws-fixture",
        model={"provider": "mock", "id": "mock"},
        runner={
            "providerId": "e2b",
            "config": {"space_id": "space-1", "mode": "readwrite"},
            "workspace": "/workspace",
        },
    )

    run = await agent.send("write a file")
    completed: list[dict[str, Any]] = []

    async def wait() -> None:
        turn = await run.wait()
        if turn is not None:
            completed.append(turn)

    async with anyio.create_task_group() as task_group:
        task_group.start_soon(wait)
        await emit_notifications(transport, fixture)

    # The fixture transport already asserted the runner binding shape on thread/start.
    assert "thread/start" in transport.seen_methods
    assert completed and completed[0]["raw"]["params"]["turn"]["id"] == "turn-runner"


@pytest.mark.anyio
async def test_python_sdk_replays_user_input_and_plan_exit_fixture() -> None:
    fixture = load_fixture("user-input-flow.jsonl")
    transport = fixture_transport(fixture)
    agent = await RoderAgent.create(
        transport=transport,
        cwd="/workspace",
        model={"provider": "mock", "id": "mock"},
        approvals={
            "on_user_input": lambda request: {"answers": "fixture answer"},
            "on_plan_exit": lambda request: {"approved": True},
        },
    )

    await agent.send("ask me")
    await emit_notifications(transport, fixture)
    await eventually(
        lambda: "thread/resolve_user_input" in transport.seen_methods
        and "thread/exit_plan" in transport.seen_methods
    )


@pytest.mark.anyio
async def test_python_sdk_replays_command_output_and_interrupt_fixture() -> None:
    fixture = load_fixture("command-output-flow.jsonl")
    transport = fixture_transport(fixture)
    agent = await RoderAgent.create(
        transport=transport,
        cwd="/workspace",
        model={"provider": "mock", "id": "mock"},
    )
    run = await agent.send("run command")
    events: list[str] = []

    async def collect() -> None:
        async for event in run.stream():
            events.append(event["type"])

    async with anyio.create_task_group() as task_group:
        task_group.start_soon(collect)
        await emit_notifications(transport, fixture)
        assert await run.cancel("fixture stop") == {"interrupted": True}

    assert events == ["command.output_delta", "turn.completed"]


@pytest.mark.anyio
async def test_python_sdk_replays_workspace_files_fixture() -> None:
    fixture = load_fixture("workspace-files-flow.jsonl")
    transport = fixture_transport(fixture)
    client = RoderRpcClient(transport)

    status = await client.call("workspace/files/status", {"workspaceId": "ws_files"})
    assert status["status"]["state"] == "missing"

    rebuild = await client.call("workspace/files/rebuild", {"workspaceId": "ws_files"})
    assert rebuild["status"]["state"] == "ready"
    assert rebuild["status"]["fileCount"] == 3

    root_children = await client.call(
        "workspace/files/children",
        {"workspaceId": "ws_files", "rootId": "root_repo"},
    )
    assert [entry["name"] for entry in root_children["entries"]] == ["roadmap", "src"]

    roadmap_children = await client.call(
        "workspace/files/children",
        {"workspaceId": "ws_files", "rootId": "root_repo", "path": "roadmap"},
    )
    assert roadmap_children["entries"][0]["kind"] == "file"

    query = await client.call(
        "workspace/files/query",
        {"workspaceId": "ws_files", "query": "desktop custom", "limit": 5},
    )
    assert query["matches"][0]["entry"]["path"] == "roadmap/001-desktop-custom-user-extensions.md"

    read = await client.call(
        "workspace/files/read",
        {
            "workspaceId": "ws_files",
            "rootId": "root_repo",
            "path": "roadmap/001-desktop-custom-user-extensions.md",
            "limit": 17,
        },
    )
    assert read["encoding"] == "utf8"
    assert read["text"] == "# Desktop Custom "

    assert [notification["method"] for notification in fixture["notifications"]] == [
        "workspace/files/statusChanged",
        "workspace/files/statusChanged",
    ]


class FixtureTransport(InMemoryTransport):
    def __init__(self, fixture: dict[str, list[dict[str, Any]]]) -> None:
        self.requests = list(fixture["requests"])
        self.responses = list(fixture["responses"])
        self.seen_methods: list[str] = []
        super().__init__(self._handle)

    def _handle(self, request: dict[str, Any]) -> dict[str, Any]:
        expected = self.requests.pop(0)
        response = self.responses.pop(0)
        assert request["method"] == expected["method"]
        assert request.get("params", {}) == expected.get("params", {})
        self.seen_methods.append(str(request["method"]))
        response = dict(response)
        response["id"] = request["id"]
        return response


def fixture_transport(fixture: dict[str, list[dict[str, Any]]]) -> FixtureTransport:
    return FixtureTransport(fixture)


def load_fixture(name: str) -> dict[str, list[dict[str, Any]]]:
    records = [json.loads(line) for line in (FIXTURE_ROOT / name).read_text().splitlines() if line]
    return {
        "requests": [record["request"] for record in records if record["kind"] == "api.request"],
        "responses": [record["response"] for record in records if record["kind"] == "api.response"],
        "notifications": [
            record["notification"] for record in records if record["kind"] == "api.notification"
        ],
    }


async def emit_notifications(
    transport: InMemoryTransport,
    fixture: dict[str, list[dict[str, Any]]],
) -> None:
    for notification in fixture["notifications"]:
        await transport.emit(notification)


async def eventually(assertion: Any) -> None:
    for _ in range(20):
        if assertion():
            return
        await anyio.sleep(0.005)
    assert assertion()
