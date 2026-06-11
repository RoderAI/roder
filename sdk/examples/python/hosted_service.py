"""Hosted multi-tenant service example (roadmap phase 72): connect with a
bearer credential, read hosted/whoami, start a thread, stream a run, and
close. Gated behind RODER_HOSTED_SDK_LIVE=1 because it needs a running
hosted gateway (see docs/roder-hosted-service.md)."""

from __future__ import annotations

import os

import anyio

from roder_sdk import HostedClient


async def main() -> None:
    if os.environ.get("RODER_HOSTED_SDK_LIVE") != "1":
        print(
            "Set RODER_HOSTED_SDK_LIVE=1, RODER_HOSTED_URL, and RODER_HOSTED_TOKEN "
            "to run this example."
        )
        return
    url = os.environ["RODER_HOSTED_URL"]
    token = os.environ["RODER_HOSTED_TOKEN"]

    hosted = await HostedClient.connect(url, token=token)
    whoami = await hosted.whoami()
    print(f"tenant {whoami['tenant']['tenantId']} as {whoami['principal']}")

    started = await hosted.client.call(
        "thread/start",
        {
            "workspaceId": os.environ.get("RODER_HOSTED_WORKSPACE_ID"),
            "model": "mock",
        },
    )
    thread_id = started["thread"]["id"]
    turn = await hosted.client.call(
        "turn/start",
        {"threadId": thread_id, "prompt": "Say hello from the hosted SDK example."},
    )
    print("turn:", turn)

    async for notification in hosted.notifications():
        print("notification:", notification.get("method"))
        if notification.get("method") in ("turn/completed", "turn/failed"):
            break

    await hosted.close()


anyio.run(main)
