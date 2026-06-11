"""Hosted multi-tenant connection helpers (roadmap phase 72, Task 6).

Hosted Roder authenticates at the WebSocket handshake with a bearer
credential in the ``Authorization`` header — the gateway always rejects
query-string credentials. :class:`HostedClient` wraps the standard RPC
client with typed helpers for ``hosted/*`` methods; raw JSON-RPC access
stays available via ``client.raw_request`` for forward-compatible hosted
methods.

Token refresh/reconnect: connections authenticate once at handshake time,
so refreshing a token means reconnecting. :meth:`HostedClient.reconnect`
builds a fresh transport using the token provider. Requests in flight when
a connection drops fail with a transport error and are NEVER replayed
automatically — callers retry mutating requests themselves because only
they know whether the operation is idempotent.
"""

from __future__ import annotations

import inspect
from collections.abc import Awaitable, Callable
from typing import Any

from .client import RoderRpcClient
from .transports import WebSocketTransport

TokenProvider = Callable[[], "str | Awaitable[str]"]


class HostedClient:
    def __init__(self, options: dict[str, Any], client: RoderRpcClient) -> None:
        self._options = options
        self.client = client

    @classmethod
    async def connect(
        cls,
        url: str,
        *,
        token: str | None = None,
        token_provider: TokenProvider | None = None,
        headers: dict[str, str] | None = None,
        connector: Callable[..., Awaitable[Any]] | None = None,
    ) -> "HostedClient":
        """Connects and authenticates against a hosted Roder gateway."""
        options = {
            "url": url,
            "token": token,
            "token_provider": token_provider,
            "headers": headers,
            "connector": connector,
        }
        client = await _hosted_rpc_client(options)
        return cls(options, client)

    async def reconnect(self) -> None:
        """Re-authenticates with a fresh credential and replaces the
        connection. In-flight requests on the old connection fail; nothing
        is replayed."""
        next_client = await _hosted_rpc_client(self._options)
        previous = self.client
        self.client = next_client
        await previous.close()

    async def whoami(self) -> dict[str, Any]:
        return await self.client.call("hosted/whoami", {})

    async def create_service_account(self, display_name: str) -> dict[str, Any]:
        return await self.client.call(
            "hosted/service_accounts/create", {"displayName": display_name}
        )

    async def revoke_service_account(self, key_id: str) -> dict[str, Any]:
        return await self.client.call("hosted/service_accounts/revoke", {"keyId": key_id})

    async def list_hooks(self) -> dict[str, Any]:
        return await self.client.call("hosted/hooks/list", {})

    async def create_hook(self, hook: dict[str, Any]) -> dict[str, Any]:
        return await self.client.call("hosted/hooks/create", {"hook": hook})

    async def delete_hook(self, hook_id: str) -> dict[str, Any]:
        return await self.client.call("hosted/hooks/delete", {"hookId": hook_id})

    async def audit_list(self) -> dict[str, Any]:
        return await self.client.call("hosted/audit/list", {})

    def notifications(self):
        return self.client.notifications()

    async def close(self) -> None:
        await self.client.close()


async def _resolve_token(options: dict[str, Any]) -> str | None:
    provider: TokenProvider | None = options.get("token_provider")
    if provider is not None:
        token = provider()
        if inspect.isawaitable(token):
            token = await token
        return str(token)
    return options.get("token")


async def _hosted_rpc_client(options: dict[str, Any]) -> RoderRpcClient:
    token = await _resolve_token(options)
    headers: dict[str, str] | None = options.get("headers")
    has_external_auth = bool(headers) and any(
        key.lower() == "authorization" for key in headers
    )
    if not token and not has_external_auth:
        raise ValueError(
            "hosted connections require a token, token_provider, or an Authorization header"
        )
    transport = await WebSocketTransport.connect(
        options["url"],
        token=token,
        headers=headers,
        connector=options.get("connector"),
    )
    return RoderRpcClient(transport)
