import type { RoderRpcClient } from "./client.js";
import type { EventMode, RoderSdkEvent, TurnCompletedEvent } from "./events.js";
import { normalizeNotification } from "./events.js";
import type { JsonRpcNotification } from "./transports.js";

export interface RoderRunOptions {
  eventMode?: EventMode;
  /**
   * Subscription opened before turn/start was sent. Without it, a
   * turn/completed delivered in the same I/O chunk as the turn/start response
   * is fanned out before stream() subscribes and the run never terminates.
   */
  notifications?: AsyncIterable<JsonRpcNotification>;
}

export class RoderRun {
  private eagerNotifications: AsyncIterable<JsonRpcNotification> | undefined;

  constructor(
    private readonly client: RoderRpcClient,
    readonly threadId: string,
    readonly turnId: string,
    private readonly options: RoderRunOptions = {},
  ) {
    this.eagerNotifications = options.notifications;
  }

  async *stream(): AsyncIterable<RoderSdkEvent> {
    const source = this.eagerNotifications ?? this.client.notifications();
    this.eagerNotifications = undefined;
    for await (const notification of source) {
      const event = normalizeNotification(notification, this.options.eventMode ?? "permissive");
      if (event) {
        yield event;
      }
      if (event && this.isOwnTurnCompleted(event)) {
        return;
      }
    }
  }

  rawEvents() {
    return this.client.notifications();
  }

  async wait(): Promise<TurnCompletedEvent | undefined> {
    for await (const event of this.stream()) {
      if (this.isOwnTurnCompleted(event)) {
        return event;
      }
    }
    return undefined;
  }

  /**
   * The notification stream carries every thread on the connection (and the
   * hub may replay a backlog from before this turn), so terminal decisions
   * key on both ids — a foreign turn's completion must not end this run.
   */
  private isOwnTurnCompleted(event: RoderSdkEvent): event is TurnCompletedEvent {
    return (
      event.type === "turn.completed" &&
      event.threadId === this.threadId &&
      event.turn.id === this.turnId
    );
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
