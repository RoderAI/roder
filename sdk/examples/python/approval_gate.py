from __future__ import annotations

import anyio

from roder_sdk import InMemoryTransport, RoderAgent


async def main() -> None:
    async def handler(request: dict) -> dict:
        if request["method"] == "thread/start":
            return {"jsonrpc": "2.0", "id": request["id"], "result": {"thread": {"id": "thread-approval"}}}
        if request["method"] == "turn/start":
            return {"jsonrpc": "2.0", "id": request["id"], "result": {"turn": {"id": "turn-approval"}}}
        return {"jsonrpc": "2.0", "id": request["id"], "result": {"ok": True}}

    async def on_tool_approval(request: dict) -> dict:
        return {"approved": request.get("toolName") == "fs/readFile", "message": "example policy"}

    agent = await RoderAgent.create(
        transport=InMemoryTransport(handler),
        approvals={"on_tool_approval": on_tool_approval},
    )
    await agent.send("Read a file only if policy allows it")
    await agent.close()


anyio.run(main)
