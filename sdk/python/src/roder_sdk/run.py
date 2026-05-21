from __future__ import annotations

from collections.abc import AsyncIterator
from typing import Any

import anyio

from .client import RoderRpcClient
from .events import EventMode, JsonRpcNotification, RoderSdkEvent, normalize_notification


class RoderRun:
    def __init__(
        self,
        client: RoderRpcClient,
        thread_id: str,
        turn_id: str,
        *,
        event_mode: EventMode = "permissive",
    ) -> None:
        self.client = client
        self.thread_id = thread_id
        self.turn_id = turn_id
        self.event_mode: EventMode = event_mode
        self.cancel_scope = anyio.CancelScope()

    async def stream(self) -> AsyncIterator[RoderSdkEvent]:
        async for notification in self.client.notifications():
            event = normalize_notification(notification, self.event_mode)
            if event is not None:
                yield event
            if event and event["type"] == "turn.completed" and _matches_turn(notification.get("params"), self.turn_id):
                return

    def raw_events(self) -> AsyncIterator[JsonRpcNotification]:
        return self.client.notifications()

    async def wait(self) -> RoderSdkEvent | None:
        async for event in self.stream():
            if event["type"] == "turn.completed":
                return event
        return None

    async def cancel(self, reason: str = "sdk cancel") -> Any:
        self.cancel_scope.cancel()
        return await self.client.call(
            "turn/interrupt",
            {"threadId": self.thread_id, "turnId": self.turn_id, "reason": reason},
        )

    async def result(self) -> Any:
        return await self.client.call("thread/read", {"threadId": self.thread_id})


def _matches_turn(params: Any, turn_id: str) -> bool:
    if not isinstance(params, dict):
        return True
    turn = params.get("turn")
    nested_id = turn.get("id") if isinstance(turn, dict) else None
    direct_id = params.get("turnId")
    if direct_id is None and nested_id is None:
        return True
    return direct_id == turn_id or nested_id == turn_id
