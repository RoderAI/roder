from __future__ import annotations

import asyncio
import inspect
from collections.abc import Awaitable, Callable
from typing import Any, cast

from .client import RoderRpcClient
from .events import EventMode
from .run import RoderRun
from .transports import InMemoryTransport, LocalProcessTransport, RoderTransport, WebSocketTransport

ApprovalCallback = Callable[[Any], dict[str, Any] | Awaitable[dict[str, Any]]]


class RoderAgent:
    def __init__(
        self,
        transport: RoderTransport,
        *,
        cwd: str | None = None,
        model: dict[str, str] | None = None,
        thread_id: str | None = None,
        approvals: dict[str, ApprovalCallback] | None = None,
        event_mode: EventMode = "permissive",
    ) -> None:
        self.transport = transport
        self.client = RoderRpcClient(transport)
        self.cwd = cwd
        self.model = model or {}
        self.thread_id = thread_id
        self.approvals = approvals or {}
        self.event_mode: EventMode = event_mode
        self._callback_task: asyncio.Task[None] | None = None

    @classmethod
    async def create(
        cls,
        *,
        local: dict[str, Any] | None = None,
        remote: dict[str, Any] | None = None,
        transport: RoderTransport | None = None,
        cwd: str | None = None,
        model: dict[str, str] | None = None,
        thread_id: str | None = None,
        approvals: dict[str, ApprovalCallback] | None = None,
        event_mode: EventMode = "permissive",
    ) -> "RoderAgent":
        resolved = await _resolve_transport(local=local, remote=remote, transport=transport, cwd=cwd)
        agent = cls(
            resolved,
            cwd=cwd,
            model=model,
            thread_id=thread_id,
            approvals=approvals,
            event_mode=event_mode,
        )
        agent._start_callback_loop()
        return agent

    async def __aenter__(self) -> "RoderAgent":
        return self

    async def __aexit__(self, exc_type: object, exc: object, tb: object) -> None:
        await self.close()

    async def send(self, input: str | list[dict[str, Any]]) -> RoderRun:
        thread_id = self.thread_id or await self._start_thread()
        self.thread_id = thread_id
        result = await self.client.call(
            "turn/start",
            {"threadId": thread_id, "input": _normalize_input(input)},
        )
        turn_id = _extract_id(result, "turn") or _extract_string(result, "turnId") or _extract_string(result, "id")
        if not turn_id:
            raise RuntimeError("turn/start response did not include a turn id")
        return RoderRun(self.client, thread_id, turn_id, event_mode=self.event_mode)

    async def list_models(self) -> Any:
        return await self.client.call("model/list")

    async def list_providers(self) -> Any:
        return await self.client.call("providers/list")

    async def read_thread(self, thread_id: str | None = None) -> Any:
        selected = thread_id or self.thread_id
        if not selected:
            raise RuntimeError("read_thread requires a thread id")
        return await self.client.call("thread/read", {"threadId": selected})

    async def list_threads(self) -> Any:
        return await self.client.call("thread/list")

    async def list_tools(self) -> Any:
        return await self.client.call("tools/list")

    async def list_commands(self) -> Any:
        return await self.client.call("commands/list")

    async def close(self) -> None:
        if self._callback_task:
            self._callback_task.cancel()
        await self.client.close()

    async def _start_thread(self) -> str:
        result = await self.client.call(
            "thread/start",
            {
                "cwd": self.cwd,
                "model": self.model.get("id"),
                "modelProvider": self.model.get("provider"),
            },
        )
        thread_id = _extract_id(result, "thread") or _extract_string(result, "threadId") or _extract_string(result, "id")
        if not thread_id:
            raise RuntimeError("thread/start response did not include a thread id")
        return thread_id

    def _start_callback_loop(self) -> None:
        if not self.approvals:
            return
        self._callback_task = asyncio.create_task(self._callback_loop())

    async def _callback_loop(self) -> None:
        async for notification in self.client.notifications():
            await self._handle_callback_notification(str(notification.get("method")), notification.get("params"))

    async def _handle_callback_notification(self, method: str, params: Any) -> None:
        if method == "session/approvalRequested" and "on_tool_approval" in self.approvals:
            decision = await _maybe_await(self.approvals["on_tool_approval"](params))
            await self.client.call(
                "session/resolve_approval",
                {
                    "approvalId": _extract_string(params, "approvalId"),
                    "approved": bool(decision.get("approved")),
                    "message": decision.get("message"),
                },
            )
        elif method == "session/userInputRequested" and "on_user_input" in self.approvals:
            decision = await _maybe_await(self.approvals["on_user_input"](params))
            await self.client.call(
                "session/resolve_user_input",
                {"requestId": _extract_string(params, "requestId"), "response": decision.get("response")},
            )
        elif method == "session/planExitRequested" and "on_plan_exit" in self.approvals:
            decision = await _maybe_await(self.approvals["on_plan_exit"](params))
            await self.client.call(
                "session/exit_plan",
                {
                    "requestId": _extract_string(params, "requestId"),
                    "accepted": bool(decision.get("accepted")),
                    "message": decision.get("message"),
                },
            )


async def _resolve_transport(
    *,
    local: dict[str, Any] | None,
    remote: dict[str, Any] | None,
    transport: RoderTransport | None,
    cwd: str | None,
) -> RoderTransport:
    if transport:
        return transport
    if remote:
        return await WebSocketTransport.connect(**remote)
    if local:
        return await LocalProcessTransport.create(
            command=local.get("command", "roder"),
            args=local.get("args"),
            cwd=local.get("cwd", cwd),
            env=local.get("env"),
        )
    return InMemoryTransport(
        lambda request: {
            "jsonrpc": "2.0",
            "id": request.get("id"),
            "error": {"code": -32000, "message": "no transport configured"},
        }
    )


def _normalize_input(input: str | list[dict[str, Any]]) -> list[dict[str, Any]]:
    return [{"type": "text", "text": input}] if isinstance(input, str) else input


def _extract_id(value: Any, key: str) -> str | None:
    if isinstance(value, dict) and isinstance(value.get(key), dict):
        return _extract_string(value[key], "id")
    return None


def _extract_string(value: Any, key: str) -> str | None:
    if isinstance(value, dict) and isinstance(value.get(key), str):
        return str(value[key])
    return None


async def _maybe_await(value: dict[str, Any] | Awaitable[dict[str, Any]]) -> dict[str, Any]:
    if inspect.isawaitable(value):
        return await cast(Awaitable[dict[str, Any]], value)
    return cast(dict[str, Any], value)
