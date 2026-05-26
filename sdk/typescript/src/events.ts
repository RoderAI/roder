import type { JsonRpcNotification } from "./transports.js";

export type RoderSdkEvent =
  | { type: "thread.started"; raw: JsonRpcNotification }
  | { type: "thread.status.changed"; raw: JsonRpcNotification }
  | { type: "turn.started"; raw: JsonRpcNotification }
  | { type: "turn.delta"; raw: JsonRpcNotification }
  | { type: "turn.completed"; raw: JsonRpcNotification }
  | { type: "approval.requested"; raw: JsonRpcNotification }
  | { type: "approval.resolved"; raw: JsonRpcNotification }
  | { type: "user_input.requested"; raw: JsonRpcNotification }
  | { type: "user_input.resolved"; raw: JsonRpcNotification }
  | { type: "plan_exit.requested"; raw: JsonRpcNotification }
  | { type: "plan_exit.resolved"; raw: JsonRpcNotification }
  | { type: "command.output_delta"; raw: JsonRpcNotification }
  | { type: "raw.notification"; raw: JsonRpcNotification };

export type EventMode = "strict" | "permissive";

const EVENT_TYPES: Record<string, RoderSdkEvent["type"]> = {
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
};

export function normalizeNotification(
  raw: JsonRpcNotification,
  mode: EventMode = "permissive",
): RoderSdkEvent | undefined {
  const type = EVENT_TYPES[raw.method];
  if (type) {
    return { type, raw } as RoderSdkEvent;
  }
  return mode === "permissive" ? { type: "raw.notification", raw } : undefined;
}
