import type {
  ExternalToolCall,
  Thread,
  ThreadItem,
  ThreadItemDelta,
  ThreadStatus,
  Turn,
} from "./protocol.js";
import type { JsonRpcNotification } from "./transports.js";

export interface ThreadStartedEvent {
  type: "thread.started";
  thread: Thread;
  raw: JsonRpcNotification;
}

export interface ThreadStatusChangedEvent {
  type: "thread.status.changed";
  threadId: string;
  status: ThreadStatus;
  raw: JsonRpcNotification;
}

export interface TurnStartedEvent {
  type: "turn.started";
  threadId: string;
  turn: Turn;
  raw: JsonRpcNotification;
}

export interface TurnCompletedEvent {
  type: "turn.completed";
  threadId: string;
  turn: Turn;
  raw: JsonRpcNotification;
}

/** roder-protocol `ThreadItemEvent` envelope shared by item.* events. */
interface ItemEventBase {
  seq: number;
  eventId: string;
  threadId: string;
  turnId: string;
  /** RFC3339. */
  timestamp: string;
  raw: JsonRpcNotification;
}

export interface ItemStartedEvent extends ItemEventBase {
  type: "item.started";
  item: ThreadItem;
}

export interface ItemCompletedEvent extends ItemEventBase {
  type: "item.completed";
  item: ThreadItem;
}

/**
 * Covers item/agentMessage/delta and the item/reasoning/* delta notifications;
 * `delta.type` carries the per-method distinction.
 */
export interface ItemDeltaEvent extends ItemEventBase {
  type: "item.delta";
  itemId: string;
  delta: ThreadItemDelta;
}

export interface ToolExecutionRequestedEvent {
  type: "tool_execution.requested";
  threadId: string;
  turnId: string;
  requestId: string;
  call: ExternalToolCall;
  raw: JsonRpcNotification;
}

export interface ToolExecutionResolvedEvent {
  type: "tool_execution.resolved";
  threadId: string;
  turnId: string;
  requestId: string;
  toolId: string;
  toolName: string;
  /** "resolved" | "timedOut" | "cancelled". */
  outcome: string;
  isError: boolean;
  raw: JsonRpcNotification;
}

/** Known notifications that pass through without a typed payload. */
export type RawEventType =
  | "approval.requested"
  | "approval.resolved"
  | "user_input.requested"
  | "user_input.resolved"
  | "plan_exit.requested"
  | "plan_exit.resolved"
  | "command.output_delta"
  | "raw.notification";

export interface RawNotificationEvent {
  type: RawEventType;
  raw: JsonRpcNotification;
}

export type RoderSdkEvent =
  | ThreadStartedEvent
  | ThreadStatusChangedEvent
  | TurnStartedEvent
  | TurnCompletedEvent
  | ItemStartedEvent
  | ItemCompletedEvent
  | ItemDeltaEvent
  | ToolExecutionRequestedEvent
  | ToolExecutionResolvedEvent
  | RawNotificationEvent;

export type EventMode = "strict" | "permissive";

const PASSTHROUGH_TYPES: Record<string, Exclude<RawEventType, "raw.notification">> = {
  "thread/approvalRequested": "approval.requested",
  "thread/approvalResolved": "approval.resolved",
  "thread/userInputRequested": "user_input.requested",
  "thread/userInputResolved": "user_input.resolved",
  "thread/planExitRequested": "plan_exit.requested",
  "thread/planExitResolved": "plan_exit.resolved",
  "command/exec/outputDelta": "command.output_delta",
};

/**
 * Maps a notification onto the typed event surface. Payload parsing is
 * shallow: discriminants and identifiers are validated, nested shapes follow
 * the Rust serializers mirrored in protocol.ts. A known method whose payload
 * fails parsing degrades to `raw.notification` in permissive mode and is
 * dropped in strict mode, same as an unknown method.
 */
export function normalizeNotification(
  raw: JsonRpcNotification,
  mode: EventMode = "permissive",
): RoderSdkEvent | undefined {
  const typed = parseTypedEvent(raw);
  if (typed) {
    return typed;
  }
  const passthrough = PASSTHROUGH_TYPES[raw.method];
  if (passthrough) {
    return { type: passthrough, raw };
  }
  return mode === "permissive" ? { type: "raw.notification", raw } : undefined;
}

function parseTypedEvent(raw: JsonRpcNotification): RoderSdkEvent | undefined {
  const params = raw.params;
  switch (raw.method) {
    case "thread/started": {
      if (!isRecord(params)) {
        return undefined;
      }
      const thread = parseThread(params.thread);
      return thread ? { type: "thread.started", thread, raw } : undefined;
    }
    case "thread/status/changed": {
      if (!isRecord(params) || typeof params.threadId !== "string") {
        return undefined;
      }
      const status = parseThreadStatus(params.status);
      return status
        ? { type: "thread.status.changed", threadId: params.threadId, status, raw }
        : undefined;
    }
    case "turn/started":
    case "turn/completed": {
      if (!isRecord(params) || typeof params.threadId !== "string") {
        return undefined;
      }
      const turn = parseTurn(params.turn);
      if (!turn) {
        return undefined;
      }
      const type = raw.method === "turn/started" ? "turn.started" : "turn.completed";
      return { type, threadId: params.threadId, turn, raw };
    }
    case "item/started":
    case "item/completed": {
      const meta = parseItemEventMeta(params);
      if (!meta) {
        return undefined;
      }
      const item = parseItem(meta.event.item);
      if (!item) {
        return undefined;
      }
      const type = raw.method === "item/started" ? "item.started" : "item.completed";
      return { type, ...meta.base, item, raw };
    }
    case "item/agentMessage/delta":
    case "item/reasoning/textDelta":
    case "item/reasoning/summaryPartAdded":
    case "item/reasoning/summaryTextDelta": {
      const meta = parseItemEventMeta(params);
      if (!meta || typeof meta.event.itemId !== "string") {
        return undefined;
      }
      const delta = parseDelta(meta.event.delta);
      if (!delta) {
        return undefined;
      }
      return { type: "item.delta", ...meta.base, itemId: meta.event.itemId, delta, raw };
    }
    case "thread/toolExecutionRequested": {
      if (
        !isRecord(params) ||
        typeof params.threadId !== "string" ||
        typeof params.turnId !== "string" ||
        typeof params.requestId !== "string" ||
        !isRecord(params.call) ||
        typeof params.call.id !== "string" ||
        typeof params.call.name !== "string"
      ) {
        return undefined;
      }
      return {
        type: "tool_execution.requested",
        threadId: params.threadId,
        turnId: params.turnId,
        requestId: params.requestId,
        call: { id: params.call.id, name: params.call.name, arguments: params.call.arguments },
        raw,
      };
    }
    case "thread/toolExecutionResolved": {
      if (
        !isRecord(params) ||
        typeof params.threadId !== "string" ||
        typeof params.turnId !== "string" ||
        typeof params.requestId !== "string" ||
        typeof params.toolId !== "string" ||
        typeof params.toolName !== "string" ||
        typeof params.outcome !== "string" ||
        typeof params.isError !== "boolean"
      ) {
        return undefined;
      }
      return {
        type: "tool_execution.resolved",
        threadId: params.threadId,
        turnId: params.turnId,
        requestId: params.requestId,
        toolId: params.toolId,
        toolName: params.toolName,
        outcome: params.outcome,
        isError: params.isError,
        raw,
      };
    }
    default:
      return undefined;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseThread(value: unknown): Thread | undefined {
  if (!isRecord(value) || typeof value.id !== "string" || !parseThreadStatus(value.status)) {
    return undefined;
  }
  return value as unknown as Thread;
}

function parseThreadStatus(value: unknown): ThreadStatus | undefined {
  if (!isRecord(value) || typeof value.type !== "string") {
    return undefined;
  }
  return value as unknown as ThreadStatus;
}

function parseTurn(value: unknown): Turn | undefined {
  if (!isRecord(value) || typeof value.id !== "string" || typeof value.status !== "string") {
    return undefined;
  }
  return value as unknown as Turn;
}

const ITEM_TYPES = new Set([
  "userMessage",
  "agentMessage",
  "reasoning",
  "toolExecution",
  "compaction",
  "error",
  "raw",
]);

function parseItem(value: unknown): ThreadItem | undefined {
  if (
    !isRecord(value) ||
    typeof value.id !== "string" ||
    typeof value.type !== "string" ||
    !ITEM_TYPES.has(value.type)
  ) {
    return undefined;
  }
  if (
    value.type === "toolExecution" &&
    (typeof value.toolCallId !== "string" || typeof value.toolName !== "string")
  ) {
    return undefined;
  }
  return value as unknown as ThreadItem;
}

const DELTA_TYPES = new Set([
  "agentMessageText",
  "reasoningText",
  "reasoningSummaryPartAdded",
  "reasoningSummaryText",
]);

function parseDelta(value: unknown): ThreadItemDelta | undefined {
  if (!isRecord(value) || typeof value.type !== "string" || !DELTA_TYPES.has(value.type)) {
    return undefined;
  }
  return value as unknown as ThreadItemDelta;
}

function parseItemEventMeta(
  params: unknown,
): { base: Omit<ItemEventBase, "raw">; event: Record<string, unknown> } | undefined {
  if (
    !isRecord(params) ||
    typeof params.seq !== "number" ||
    typeof params.eventId !== "string" ||
    typeof params.threadId !== "string" ||
    typeof params.turnId !== "string" ||
    typeof params.timestamp !== "string" ||
    !isRecord(params.event)
  ) {
    return undefined;
  }
  return {
    base: {
      seq: params.seq,
      eventId: params.eventId,
      threadId: params.threadId,
      turnId: params.turnId,
      timestamp: params.timestamp,
    },
    event: params.event,
  };
}
