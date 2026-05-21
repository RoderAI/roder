from __future__ import annotations

import os

import anyio

from roder_sdk import RoderAgent


async def main() -> None:
    if os.environ.get("RODER_SDK_LIVE") != "1":
        print("Set RODER_SDK_LIVE=1, RODER_REMOTE_URL, and RODER_REMOTE_TOKEN to run this example.")
        return
    url = os.environ["RODER_REMOTE_URL"]
    token = os.environ["RODER_REMOTE_TOKEN"]
    agent = await RoderAgent.create(remote={"url": url, "token": token})
    print(await agent.list_providers())
    await agent.close()


anyio.run(main)
