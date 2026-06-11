"""Hosted SDK helpers (phase 72, Task 6): bearer auth at the handshake,
typed hosted/* helpers, token-provider reconnect without request replay,
and raw JSON-RPC access. Fully offline against a fake socket."""

from __future__ import annotations

import json
from typing import Any

import pytest

from roder_sdk import HostedClient


class AutoRespondingSocket:
    def __init__(self) -> None:
        self.sent: list[str] = []
        self._messages: list[str] = []

    async def send(self, data: str) -> None:
        self.sent.append(data)
        request = json.loads(data)
        result: Any
        method = request["method"]
        if method == "hosted/whoami":
            result = {
                "tenant": {"tenantId": "acme"},
                "principal": {"kind": "user", "user_id": "ops"},
                "role": "tenant_admin",
                "scopes": ["read", "write", "admin"],
            }
        elif method == "hosted/service_accounts/create":
            result = {"keyId": "k1", "token": "rk_sa_k1.secret"}
        elif method == "hosted/service_accounts/revoke":
            result = {"revoked": True}
        elif method == "hosted/hooks/list":
            result = {"hooks": []}
        else:
            result = {"echoed": method}
        self._messages.append(
            json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result})
        )

    async def recv(self) -> str:
        import anyio

        while not self._messages:
            await anyio.sleep(0.001)
        return self._messages.pop(0)

    async def close(self) -> None:
        pass


@pytest.mark.anyio
async def test_hosted_client_authenticates_and_serves_typed_helpers() -> None:
    sockets: list[AutoRespondingSocket] = []
    handshakes: list[dict[str, Any]] = []

    async def connector(url: str, **kwargs: Any) -> AutoRespondingSocket:
        handshakes.append({"url": url, **kwargs})
        socket = AutoRespondingSocket()
        sockets.append(socket)
        return socket

    hosted = await HostedClient.connect(
        "wss://roder.example.test",
        token="rk_test_sdk_token",
        connector=connector,
    )
    assert handshakes[0]["additional_headers"] == {
        "Authorization": "Bearer rk_test_sdk_token"
    }
    # Credentials never appear in the URL.
    assert "rk_test" not in handshakes[0]["url"]

    whoami = await hosted.whoami()
    assert whoami["tenant"]["tenantId"] == "acme"

    minted = await hosted.create_service_account("ci")
    assert minted["token"].startswith("rk_sa_")
    assert (await hosted.revoke_service_account(minted["keyId"]))["revoked"] is True
    assert (await hosted.list_hooks())["hooks"] == []

    # Raw JSON-RPC stays available for forward-compatible hosted methods.
    raw = await hosted.client.call("hosted/tenants/list", {})
    assert raw == {"echoed": "hosted/tenants/list"}
    await hosted.close()


@pytest.mark.anyio
async def test_hosted_client_reconnects_with_fresh_token_without_replay() -> None:
    sockets: list[AutoRespondingSocket] = []
    issued: list[str] = []

    async def connector(url: str, **kwargs: Any) -> AutoRespondingSocket:
        socket = AutoRespondingSocket()
        sockets.append(socket)
        issued.append(kwargs["additional_headers"]["Authorization"])
        return socket

    serial = 0

    def token_provider() -> str:
        nonlocal serial
        serial += 1
        return f"rk_test_rotating_{serial}"

    hosted = await HostedClient.connect(
        "wss://roder.example.test",
        token_provider=token_provider,
        connector=connector,
    )
    await hosted.whoami()
    first_sent = len(sockets[0].sent)

    await hosted.reconnect()
    assert issued == ["Bearer rk_test_rotating_1", "Bearer rk_test_rotating_2"]
    # Nothing from the old connection is replayed on the new one.
    assert len(sockets[0].sent) == first_sent
    assert sockets[1].sent == []

    await hosted.whoami()
    assert len(sockets[1].sent) == 1
    await hosted.close()


@pytest.mark.anyio
async def test_hosted_client_requires_a_credential_source() -> None:
    async def connector(url: str, **kwargs: Any) -> AutoRespondingSocket:
        return AutoRespondingSocket()

    with pytest.raises(ValueError, match="token, token_provider, or an Authorization"):
        await HostedClient.connect("wss://roder.example.test", connector=connector)

    # Externally supplied auth headers are accepted as-is.
    handshakes: list[dict[str, Any]] = []

    async def recording_connector(url: str, **kwargs: Any) -> AutoRespondingSocket:
        handshakes.append(kwargs)
        return AutoRespondingSocket()

    hosted = await HostedClient.connect(
        "wss://roder.example.test",
        headers={"Authorization": "Bearer external-token"},
        connector=recording_connector,
    )
    assert handshakes[0]["additional_headers"] == {"Authorization": "Bearer external-token"}
    await hosted.close()
