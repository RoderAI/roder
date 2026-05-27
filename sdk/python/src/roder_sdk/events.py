from __future__ import annotations

from typing import Any, Literal, TypedDict

JsonRpcNotification = dict[str, Any]
EventMode = Literal["strict", "permissive"]


class RoderSdkEvent(TypedDict):
    type: str
    raw: JsonRpcNotification


EVENT_TYPES: dict[str, str] = {
    "thread/started": "thread.started",
    "thread/statusChanged": "thread.status.changed",
    "turn/started": "turn.started",
    "turn/delta": "turn.delta",
    "turn/completed": "turn.completed",
    "thread/approvalRequested": "approval.requested",
    "thread/approvalResolved": "approval.resolved",
    "thread/userInputRequested": "user_input.requested",
    "thread/userInputResolved": "user_input.resolved",
    "thread/planExitRequested": "plan_exit.requested",
    "thread/planExitResolved": "plan_exit.resolved",
    "command/outputDelta": "command.output_delta",
}


def normalize_notification(
    raw: JsonRpcNotification,
    mode: EventMode = "permissive",
) -> RoderSdkEvent | None:
    event_type = EVENT_TYPES.get(str(raw.get("method")))
    if event_type:
        return {"type": event_type, "raw": raw}
    if mode == "permissive":
        return {"type": "raw.notification", "raw": raw}
    return None
