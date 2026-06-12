import type { RoderSdkEvent } from "./events.js";
import type { ThreadItemStatus } from "./protocol.js";

/**
 * Display-ordered part derived from the raw event stream. Each part has a
 * stable `id`; text parts that bracket a tool call get distinct ids so a UI
 * renders them on either side of the tool rather than merged above it.
 */
export type AgentPart =
  | { type: "text-start"; id: string; itemId: string }
  | { type: "text-delta"; id: string; itemId: string; text: string }
  | { type: "text-end"; id: string; itemId: string }
  | { type: "reasoning-start"; id: string; itemId: string }
  | { type: "reasoning-delta"; id: string; itemId: string; text: string }
  | { type: "reasoning-end"; id: string; itemId: string }
  | { type: "tool-start"; id: string; toolCallId: string; toolName: string; input?: unknown }
  | {
      type: "tool-end";
      id: string;
      toolCallId: string;
      toolName: string;
      status: ThreadItemStatus;
      output?: string;
      error?: string;
    };

export interface PartTransformer {
  /** Folds one event into zero or more display parts. */
  push(event: RoderSdkEvent): AgentPart[];
  /** Closes any parts still open (an interrupted or unterminated stream). */
  flush(): AgentPart[];
}

/**
 * Re-derives display-ordered parts from `RoderSdkEvent`s.
 *
 * The one structural fix is for agent-message text: the server reuses a single
 * `agentMessage` item id (`{turnId}-agent-{phase}`) for text emitted both
 * before AND after a tool call, with no new `item/started` in between, so a
 * consumer that keys text by item id merges it into one part and renders the
 * whole tool group beneath it. On a tool boundary this transformer closes the
 * open text part and bumps a per-item counter, so post-tool text opens a fresh
 * `${itemId}__seg${N}` part. Reasoning needs no such split — the server already
 * allocates a distinct reasoning item id after each tool — so reasoning passes
 * through on its own ids.
 */
export function createPartTransformer(): PartTransformer {
  const openText = new Set<string>();
  const textSegments = new Map<string, number>();
  const openReasoning = new Set<string>();

  const textSegmentId = (itemId: string): string => {
    const n = textSegments.get(itemId) ?? 0;
    return n === 0 ? itemId : `${itemId}__seg${n}`;
  };

  const splitAtToolBoundary = (out: AgentPart[]): void => {
    for (const itemId of openText) {
      out.push({ type: "text-end", id: textSegmentId(itemId), itemId });
      textSegments.set(itemId, (textSegments.get(itemId) ?? 0) + 1);
    }
    openText.clear();
    for (const itemId of openReasoning) {
      out.push({ type: "reasoning-end", id: itemId, itemId });
    }
    openReasoning.clear();
  };

  const push = (event: RoderSdkEvent): AgentPart[] => {
    const out: AgentPart[] = [];
    switch (event.type) {
      case "item.delta": {
        const { itemId, delta } = event;
        if (delta.type === "agentMessageText") {
          if (!openText.has(itemId)) {
            openText.add(itemId);
            out.push({ type: "text-start", id: textSegmentId(itemId), itemId });
          }
          out.push({ type: "text-delta", id: textSegmentId(itemId), itemId, text: delta.delta });
        } else if (delta.type === "reasoningText" || delta.type === "reasoningSummaryText") {
          if (!openReasoning.has(itemId)) {
            openReasoning.add(itemId);
            out.push({ type: "reasoning-start", id: itemId, itemId });
          }
          out.push({ type: "reasoning-delta", id: itemId, itemId, text: delta.delta });
        }
        // reasoningSummaryPartAdded carries no text and needs no part.
        break;
      }
      case "item.started": {
        if (event.item.type === "toolExecution") {
          splitAtToolBoundary(out);
          const { id, toolCallId, toolName, input } = event.item;
          out.push({ type: "tool-start", id, toolCallId, toolName, input });
        }
        break;
      }
      case "item.completed": {
        const item = event.item;
        if (item.type === "toolExecution") {
          out.push({
            type: "tool-end",
            id: item.id,
            toolCallId: item.toolCallId,
            toolName: item.toolName,
            status: item.status,
            output: item.output,
            error: item.error,
          });
        } else if (item.type === "agentMessage" && openText.has(item.id)) {
          out.push({ type: "text-end", id: textSegmentId(item.id), itemId: item.id });
          openText.delete(item.id);
        } else if (item.type === "reasoning" && openReasoning.has(item.id)) {
          out.push({ type: "reasoning-end", id: item.id, itemId: item.id });
          openReasoning.delete(item.id);
        }
        break;
      }
      default:
        break;
    }
    return out;
  };

  const flush = (): AgentPart[] => {
    const out: AgentPart[] = [];
    for (const itemId of openText) {
      out.push({ type: "text-end", id: textSegmentId(itemId), itemId });
    }
    openText.clear();
    for (const itemId of openReasoning) {
      out.push({ type: "reasoning-end", id: itemId, itemId });
    }
    openReasoning.clear();
    return out;
  };

  return { push, flush };
}
