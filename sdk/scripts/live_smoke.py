from __future__ import annotations

import asyncio
import json
import os
import sys


async def main() -> None:
    if os.environ.get("RODER_SDK_LIVE") != "1":
        print("skipped: set RODER_SDK_LIVE=1 to run the Python live smoke")
        return

    if os.environ.get("RODER_REMOTE_URL") and os.environ.get("RODER_REMOTE_TOKEN"):
        import websockets

        async with websockets.connect(
            os.environ["RODER_REMOTE_URL"],
            additional_headers={"Authorization": f"Bearer {os.environ['RODER_REMOTE_TOKEN']}"},
        ) as socket:
            await socket.send(json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}))
            response = json.loads(await socket.recv())
            if "error" in response:
                raise RuntimeError(response["error"]["message"])
            print(f"python remote live smoke ok: {response['result'].get('provider', 'provider')}")
            return

    if os.environ.get("RODER_BIN"):
        command = [os.environ["RODER_BIN"], "app-server", "--listen", "stdio://"]
    else:
        command = [
            "cargo",
            "run",
            "-p",
            "roder",
            "--bin",
            "roder",
            "--",
            "app-server",
            "--listen",
            "stdio://",
        ]

    process = await asyncio.create_subprocess_exec(
        *command,
        stdin=asyncio.subprocess.PIPE,
        stdout=asyncio.subprocess.PIPE,
        stderr=sys.stderr,
    )
    next_id = 1

    async def call(method: str, params: dict) -> dict:
        nonlocal next_id
        request_id = next_id
        next_id += 1
        request = {"jsonrpc": "2.0", "id": request_id, "method": method, "params": params}
        assert process.stdin is not None
        assert process.stdout is not None
        process.stdin.write((json.dumps(request) + "\n").encode())
        await process.stdin.drain()
        while True:
            line = await process.stdout.readline()
            message = json.loads(line)
            if message.get("id") != request_id:
                continue
            if "error" in message:
                raise RuntimeError(f"{method}: {message['error']['message']}")
            return message

    try:
        init = await call("initialize", {})
        thread = await call(
            "thread/start",
            {"cwd": os.getcwd(), "modelProvider": "mock", "model": "mock"},
        )
        thread_id = thread["result"].get("thread", {}).get("id") or thread["result"].get("threadId") or thread["result"].get("id")
        turn = await call(
            "turn/start",
            {"threadId": thread_id, "input": [{"type": "text", "text": "live smoke"}]},
        )
        turn_id = turn["result"].get("turn", {}).get("id") or turn["result"].get("turnId") or turn["result"].get("id")
        await call("turn/interrupt", {"threadId": thread_id, "turnId": turn_id, "reason": "live smoke complete"})
        print(f"python live smoke ok: {init['result'].get('provider', 'provider')} {thread_id}")
    finally:
        process.terminate()
        await process.wait()


asyncio.run(main())
