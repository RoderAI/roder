from __future__ import annotations

import anyio

from roder_sdk import InMemoryTransport, RoderAgent


async def main() -> None:
    async def handler(request: dict) -> dict:
        if request["method"] == "thread/start":
            return {"jsonrpc": "2.0", "id": request["id"], "result": {"thread": {"id": "thread-example"}}}
        if request["method"] == "turn/start":
            return {"jsonrpc": "2.0", "id": request["id"], "result": {"turn": {"id": "turn-example"}}}
        return {"jsonrpc": "2.0", "id": request["id"], "result": {}}

    async with await RoderAgent.create(transport=InMemoryTransport(handler), cwd="/workspace") as agent:
        await agent.send("Summarize this repository")


anyio.run(main)
