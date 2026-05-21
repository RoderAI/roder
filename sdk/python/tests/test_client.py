from __future__ import annotations

import pytest

from roder_sdk import APP_SERVER_METHODS, InMemoryTransport, RoderRpcClient, RoderRpcError


@pytest.mark.anyio
async def test_client_calls_manifest_method_helpers() -> None:
    seen: list[dict] = []

    async def handler(request: dict) -> dict:
        seen.append(request)
        return {
            "jsonrpc": "2.0",
            "id": request["id"],
            "result": {"method": request["method"]},
        }

    client = RoderRpcClient(InMemoryTransport(handler))

    result = await client.methods["providers/list"]()

    assert result == {"method": "providers/list"}
    assert seen[0]["method"] == "providers/list"
    assert "thread/start" in APP_SERVER_METHODS


@pytest.mark.anyio
async def test_client_preserves_json_rpc_errors() -> None:
    async def handler(request: dict) -> dict:
        return {
            "jsonrpc": "2.0",
            "id": request["id"],
            "error": {"code": -32602, "message": "bad params", "data": {"field": "threadId"}},
        }

    client = RoderRpcClient(InMemoryTransport(handler))

    with pytest.raises(RoderRpcError) as exc:
        await client.call("thread/read", {"threadId": ""})

    assert exc.value.code == -32602
    assert exc.value.method == "thread/read"
    assert exc.value.request_id == 1
    assert exc.value.data == {"field": "threadId"}
