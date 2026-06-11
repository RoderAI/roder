#!/usr/bin/env python3
"""Hosted SDK live smoke (phase 72, Task 6). Gated: requires a running
hosted gateway and a real credential.

    RODER_HOSTED_SDK_LIVE=1 RODER_HOSTED_URL=ws://... RODER_HOSTED_TOKEN=rk_test_... \
        uv run python sdk/scripts/hosted_live_smoke.py
"""

from __future__ import annotations

import os
import sys

import anyio

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "python", "src"))

from roder_sdk import HostedClient  # noqa: E402


async def main() -> None:
    if os.environ.get("RODER_HOSTED_SDK_LIVE") != "1":
        print("skipped: set RODER_HOSTED_SDK_LIVE=1 to run the hosted live smoke")
        return
    url = os.environ["RODER_HOSTED_URL"]
    token = os.environ["RODER_HOSTED_TOKEN"]

    hosted = await HostedClient.connect(url, token=token)
    whoami = await hosted.whoami()
    print("hosted whoami ok:", whoami["tenant"]["tenantId"], whoami["role"])
    await hosted.client.call("initialize", {})
    print("initialize ok")
    await hosted.close()
    print("hosted live smoke passed")


anyio.run(main)
