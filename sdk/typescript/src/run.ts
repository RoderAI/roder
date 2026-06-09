import type { RoderRpcClient } from "./client.js";
import type { EventMode, RoderSdkEvent, TurnCompletedEvent } from "./events.js";
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
      if (event?.type === "turn.completed" && event.turn.id === this.turnId) {
        return;
      }
    }
  }

  rawEvents() {
    return this.client.notifications();
  }

  async wait(): Promise<TurnCompletedEvent | undefined> {
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
