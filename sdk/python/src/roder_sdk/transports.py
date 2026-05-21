from __future__ import annotations

import json
from collections.abc import AsyncIterator, Awaitable, Callable
from typing import Any, Protocol, cast

import anyio
from anyio.abc import Process

from .errors import RoderTransportError
from .types_generated import JsonRpcRequest, JsonRpcResponse

JsonRpcNotification = dict[str, Any]
RequestHandler = Callable[[JsonRpcRequest], JsonRpcResponse | Awaitable[JsonRpcResponse]]


class RoderTransport(Protocol):
    async def request(self, request: JsonRpcRequest) -> JsonRpcResponse: ...

    def notifications(self) -> AsyncIterator[JsonRpcNotification]: ...

    async def close(self) -> None: ...


class InMemoryTransport:
    def __init__(self, handler: RequestHandler) -> None:
        self._handler = handler
        self._send, self._receive = anyio.create_memory_object_stream[JsonRpcNotification](100)
        self._closed = False

    async def request(self, request: JsonRpcRequest) -> JsonRpcResponse:
        if self._closed:
            raise RoderTransportError("transport is closed")
        response = self._handler(request)
        if hasattr(response, "__await__"):
            response = await cast(Awaitable[JsonRpcResponse], response)
        return cast(JsonRpcResponse, response)

    async def emit(self, notification: JsonRpcNotification) -> None:
        await self._send.send(notification)

    async def notifications(self) -> AsyncIterator[JsonRpcNotification]:
        async with self._receive.clone() as receive:
            async for notification in receive:
                yield notification

    async def close(self) -> None:
        self._closed = True
        await self._send.aclose()


class LocalProcessTransport:
    def __init__(self, process: Process) -> None:
        self._process = process
        self._send, self._receive = anyio.create_memory_object_stream[JsonRpcNotification](100)
        self._lock = anyio.Lock()
        self._stdout_buffer = bytearray()

    @classmethod
    async def create(
        cls,
        *,
        command: str = "roder",
        args: list[str] | None = None,
        cwd: str | None = None,
        env: dict[str, str] | None = None,
    ) -> "LocalProcessTransport":
        process = await anyio.open_process(
            [command, *(args or ["app-server", "--listen", "stdio://"])],
            cwd=cwd,
            env=env,
        )
        return cls(process)

    async def request(self, request: JsonRpcRequest) -> JsonRpcResponse:
        if request.get("id") is None:
            raise RoderTransportError("requests require a non-null id")
        async with self._lock:
            assert self._process.stdin is not None
            await self._process.stdin.send((json.dumps(request) + "\n").encode())
            while True:
                message = await self._read_message()
                if "id" in message:
                    return cast(JsonRpcResponse, message)
                await self._send.send(message)

    async def notifications(self) -> AsyncIterator[JsonRpcNotification]:
        async with self._receive.clone() as receive:
            async for notification in receive:
                yield notification

    async def close(self) -> None:
        await self._send.aclose()
        self._process.terminate()
        with anyio.move_on_after(1, shield=True):
            await self._process.wait()

    async def _read_message(self) -> dict[str, Any]:
        assert self._process.stdout is not None
        while b"\n" not in self._stdout_buffer:
            self._stdout_buffer.extend(await self._process.stdout.receive(65536))
            if len(self._stdout_buffer) > 1024 * 1024:
                raise RoderTransportError("app-server stdout line exceeded 1MiB")
        line, _, rest = self._stdout_buffer.partition(b"\n")
        self._stdout_buffer = bytearray(rest)
        return cast(dict[str, Any], json.loads(line.decode()))


class WebSocketTransport:
    def __init__(self, socket: Any) -> None:
        self._socket = socket
        self._send, self._receive = anyio.create_memory_object_stream[JsonRpcNotification](100)
        self._lock = anyio.Lock()

    @classmethod
    async def connect(
        cls,
        url: str,
        *,
        token: str | None = None,
        subprotocols: list[str] | None = None,
        connector: Callable[..., Awaitable[Any]] | None = None,
    ) -> "WebSocketTransport":
        import websockets

        headers = {"Authorization": f"Bearer {token}"} if token else None
        connect = connector or websockets.connect
        socket = await connect(
            url,
            additional_headers=headers,
            subprotocols=cast(Any, subprotocols),
        )
        return cls(socket)

    async def request(self, request: JsonRpcRequest) -> JsonRpcResponse:
        if request.get("id") is None:
            raise RoderTransportError("requests require a non-null id")
        async with self._lock:
            await self._socket.send(json.dumps(request))
            while True:
                message = cast(dict[str, Any], json.loads(await self._socket.recv()))
                if "id" in message:
                    return cast(JsonRpcResponse, message)
                await self._send.send(message)

    async def notifications(self) -> AsyncIterator[JsonRpcNotification]:
        async with self._receive.clone() as receive:
            async for notification in receive:
                yield notification

    async def close(self) -> None:
        await self._send.aclose()
        await self._socket.close()
