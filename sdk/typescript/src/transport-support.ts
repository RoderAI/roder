import type {JsonRpcResponse} from "./types.generated.js";
import type {JsonRpcNotification} from "./transports.js";
export type PendingResponse = {
  resolve: (response: JsonRpcResponse) => void;
  reject: (error: Error) => void;
  cleanup: () => void;
};

/**
 * Fans notifications out to every active subscriber. The agent's callback loop
 * and each RoderRun stream subscribe independently; a single shared queue
 * would deliver each notification to only one of them. Notifications pushed
 * while no subscriber exists are buffered and replayed to the next subscriber.
 */
export class NotificationHub {
  private readonly subscribers = new Set<AsyncQueue<JsonRpcNotification>>();
  private backlog: JsonRpcNotification[] = [];
  private closed = false;

  push(notification: JsonRpcNotification): void {
    if (this.closed) {
      return;
    }
    if (this.subscribers.size === 0) {
      this.backlog.push(notification);
      return;
    }
    for (const queue of this.subscribers) {
      queue.push(notification);
    }
  }

  close(): void {
    this.closed = true;
    this.backlog = [];
    for (const queue of this.subscribers) {
      queue.close();
    }
    this.subscribers.clear();
  }

  subscribe(): AsyncIterable<JsonRpcNotification> {
    const queue = new AsyncQueue<JsonRpcNotification>();
    if (this.closed) {
      queue.close();
      return queue;
    }
    for (const notification of this.backlog.splice(0)) {
      queue.push(notification);
    }
    this.subscribers.add(queue);
    const unsubscribe = () => {
      this.subscribers.delete(queue);
      queue.close();
    };
    return {
      [Symbol.asyncIterator]: () => {
        const inner = queue[Symbol.asyncIterator]();
        return {
          next: () => inner.next(),
          return: async (): Promise<IteratorResult<JsonRpcNotification>> => {
            unsubscribe();
            return { value: undefined, done: true };
          },
        };
      },
    };
  }
}

class AsyncQueue<T> implements AsyncIterable<T> {
  private readonly values: T[] = [];
  private readonly waiters: Array<(result: IteratorResult<T>) => void> = [];
  private done = false;

  push(value: T): void {
    const waiter = this.waiters.shift();
    if (waiter) {
      waiter({ value, done: false });
    } else {
      this.values.push(value);
    }
  }

  close(): void {
    this.done = true;
    for (const waiter of this.waiters.splice(0)) {
      waiter({ value: undefined, done: true });
    }
  }

  [Symbol.asyncIterator](): AsyncIterator<T> {
    return {
      next: () => {
        const value = this.values.shift();
        if (value !== undefined) {
          return Promise.resolve({ value, done: false });
        }
        if (this.done) {
          return Promise.resolve({ value: undefined, done: true });
        }
        return new Promise((resolve) => this.waiters.push(resolve));
      },
    };
  }
}

export function throwIfAborted(signal: AbortSignal | undefined): void {
  if (signal?.aborted) {
    throw new DOMException("Request aborted", "AbortError");
  }
}

export function abortable<T>(promise: Promise<T>, signal: AbortSignal | undefined): Promise<T> {
  if (!signal) {
    return promise;
  }
  if (signal.aborted) return Promise.reject(new DOMException("Request aborted", "AbortError"));
  return new Promise<T>((resolve, reject) => {
    const cleanup = () => signal.removeEventListener("abort", aborted);
    const aborted = () => { cleanup(); reject(new DOMException("Request aborted", "AbortError")); };
    signal.addEventListener("abort", aborted, {once:true});
    promise.then(value => { cleanup(); resolve(value); }, error => { cleanup(); reject(error); });
  });
}

export function isNotification(
  message: JsonRpcResponse | JsonRpcNotification,
): message is JsonRpcNotification {
  return "method" in message;
}
