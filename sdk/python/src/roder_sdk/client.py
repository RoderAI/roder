from __future__ import annotations

from collections.abc import Callable
from typing import Any

from .errors import RoderRpcError
from .transports import JsonRpcNotification, RoderTransport
from .types_generated import APP_SERVER_METHODS, AppServerMethod, JsonRpcRequest, JsonRpcResponse


class RoderRpcClient:
    def __init__(self, transport: RoderTransport) -> None:
        self._transport = transport
        self._next_id = 1
        self.methods: dict[str, Callable[[Any | None], Any]] = {
            method: self._method_helper(method) for method in APP_SERVER_METHODS
        }

    async def call(self, method: AppServerMethod, params: Any = None) -> Any:
        request: JsonRpcRequest = {
            "jsonrpc": "2.0",
            "id": self._allocate_id(),
            "method": method,
        }
        if params is not None:
            request["params"] = params
        response = await self.raw_request(request)
        error = response.get("error")
        if error:
            raise RoderRpcError(
                code=int(error.get("code", -32000)),
                message=str(error.get("message", "JSON-RPC error")),
                data=error.get("data"),
                method=method,
                request_id=response.get("id"),
            )
        return response.get("result")

    async def raw_request(self, request: JsonRpcRequest) -> JsonRpcResponse:
        return await self._transport.request(request)

    def notifications(self):
        return self._transport.notifications()

    async def close(self) -> None:
        await self._transport.close()

    def _allocate_id(self) -> int:
        request_id = self._next_id
        self._next_id += 1
        return request_id

    def _method_helper(self, method: str) -> Callable[[Any | None], Any]:
        async def helper(params: Any = None) -> Any:
            return await self.call(method, params)  # type: ignore[arg-type]

        return helper


__all__ = ["JsonRpcNotification", "RoderRpcClient"]
