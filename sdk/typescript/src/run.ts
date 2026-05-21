import type { RoderRpcClient } from "./client.js";
import type { EventMode, RoderSdkEvent } from "./events.js";
import { normalizeNotification } from "./events.js";

export interface RoderRunOptions {
  eventMode?: EventMode;
}

export class RoderRun {
  constructor(
    private readonly client: RoderRpcClient,
    readonly threadId: string,
    readonly turnId: string,
    private readonly options: RoderRunOptions = {},
  ) {}

  async *stream(): AsyncIterable<RoderSdkEvent> {
    for await (const notification of this.client.notifications()) {
      const event = normalizeNotification(notification, this.options.eventMode ?? "permissive");
      if (event) {
        yield event;
      }
      if (event?.type === "turn.completed" && notificationMatchesTurn(notification.params, this.turnId)) {
        return;
      }
    }
  }

  rawEvents() {
    return this.client.notifications();
  }

  async wait(): Promise<RoderSdkEvent | undefined> {
    for await (const event of this.stream()) {
      if (event.type === "turn.completed") {
        return event;
      }
    }
    return undefined;
  }

  async cancel(reason = "sdk cancel"): Promise<unknown> {
    return this.client.call("turn/interrupt", {
      threadId: this.threadId,
      turnId: this.turnId,
      reason,
    });
  }

  async result(): Promise<unknown> {
    return this.client.call("thread/read", { threadId: this.threadId });
  }
}

function notificationMatchesTurn(params: unknown, turnId: string): boolean {
  if (!params || typeof params !== "object") {
    return true;
  }
  const value = params as { turnId?: unknown; turn?: { id?: unknown } };
  return value.turnId === undefined && value.turn?.id === undefined
    ? true
    : value.turnId === turnId || value.turn?.id === turnId;
}
