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

/** Options for consuming a run's event stream. */
export interface RoderStreamOptions {
  /**
   * Aborts stream consumption. The notification subscription parks
   * indefinitely when the app-server goes quiet, so without this a wedged
   * connection hangs the iterator forever. An aborted signal rejects the
   * pending read with an AbortError and tears the subscription down. Build
   * inactivity/deadline policy from this with `AbortSignal.timeout(ms)` and
   * `AbortSignal.any([...])`.
   */
  signal?: AbortSignal;
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

  async *stream(options: RoderStreamOptions = {}): AsyncIterable<RoderSdkEvent> {
    const base = this.eagerNotifications ?? this.client.notifications();
    this.eagerNotifications = undefined;
    const source = options.signal ? withSignal(base, options.signal) : base;
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

  async wait(options: RoderStreamOptions = {}): Promise<TurnCompletedEvent | undefined> {
    for await (const event of this.stream(options)) {
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

function abortError(): DOMException {
  return new DOMException("Stream aborted", "AbortError");
}

/**
 * Races each read against the signal. On abort the read rejects and the
 * `finally` closes the source iterator, which unsubscribes the notification
 * queue and drains its parked waiter — so no event is left for a later,
 * orphaned read to swallow. The abort listener is removed when each read
 * settles, so a long stream does not accumulate listeners on the signal.
 */
async function* withSignal<T>(
  source: AsyncIterable<T>,
  signal: AbortSignal,
): AsyncIterable<T> {
  const iterator = source[Symbol.asyncIterator]();
  try {
    for (;;) {
      if (signal.aborted) {
        throw abortError();
      }
      const result = await raceAbort(iterator.next(), signal);
      if (result.done) {
        return;
      }
      yield result.value;
    }
  } finally {
    await iterator.return?.();
  }
}

function raceAbort<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const onAbort = () => reject(abortError());
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolve(value);
      },
      (error) => {
        signal.removeEventListener("abort", onAbort);
        reject(error);
      },
    );
  });
}
