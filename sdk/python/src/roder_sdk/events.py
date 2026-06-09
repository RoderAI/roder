from __future__ import annotations

from typing import Any, Literal, TypedDict

JsonRpcNotification = dict[str, Any]
EventMode = Literal["strict", "permissive"]


class RoderSdkEvent(TypedDict):
    type: str
    raw: JsonRpcNotification


EVENT_TYPES: dict[str, str] = {
    "thread/started": "thread.started",
    "thread/status/changed": "thread.status.changed",
    "turn/started": "turn.started",
    "turn/completed": "turn.completed",
    "item/started": "item.started",
    "item/completed": "item.completed",
    "item/agentMessage/delta": "item.delta",
    "item/reasoning/textDelta": "item.delta",
    "item/reasoning/summaryPartAdded": "item.delta",
    "item/reasoning/summaryTextDelta": "item.delta",
    "thread/toolExecutionRequested": "tool_execution.requested",
    "thread/toolExecutionResolved": "tool_execution.resolved",
    "thread/approvalRequested": "approval.requested",
    "thread/approvalResolved": "approval.resolved",
    "thread/userInputRequested": "user_input.requested",
    "thread/userInputResolved": "user_input.resolved",
    "thread/planExitRequested": "plan_exit.requested",
    "thread/planExitResolved": "plan_exit.resolved",
    "command/exec/outputDelta": "command.output_delta",
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
