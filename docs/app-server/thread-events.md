# Thread follow-ups, topics, and timer ticks

Thread events let an agent check CI periodically or babysit an operation without
holding a shell command or inference request open. A subscription saves concrete
instructions against the existing thread. A timer tick or a published topic event
starts a normal turn when that thread is idle, retaining its transcript, provider,
model, workspace, thread instructions, and tool restrictions. Per-turn developer
context is not retained as authority for later ticks.

These are **process-local subscriptions**. They run while the owning harness is
alive, including the local TUI harness; they stop at process exit and do not resume
after restart. For durable cron/interval schedules and missed-run recovery across
processes, use [automations](automations.md) with an enabled scheduler. Automations
create independent run threads; thread events continue the existing conversation.

## Agent tools

- `subscribe_event`: save `topic` and `prompt`, optionally `intervalSeconds` for a
  recurring tick, `delaySeconds` for the first tick, or both. A delay without an
  interval is one-shot. Omit both to listen only for published events.
- `list_event_subscriptions`: inspect this thread's subscriptions, remaining
  deliveries, expiration, pending state, and last delivery error.
- `cancel_event_subscription`: cancel this thread's `subscriptionId` and discard
  pending delivery. An already admitted turn continues normally.
- `publish_event`: publish `topic` and `message` to all explicit subscribers in the
  same runtime. The result counts queued subscriptions, not completed turns.

Use subscriptions only when the user requests a follow-up. Stop polling when the
requested terminal condition is reached. For example, create a CI subscription:

```json
{
  "topic": "ci:pr-167",
  "prompt": "Check PR 167 CI. Investigate failing checks within the requested scope. Cancel this subscription once checks are terminal and report the result.",
  "intervalSeconds": 60,
  "maxDeliveries": 30,
  "ttlSeconds": 3600
}
```

Topics are strings scoped to one runtime; there is no external broker or implicit
subscription. Each subscriber must opt in. Publishing does not grant subscribers
new authority or change their policy mode.

## JSON-RPC API

The methods work with local and remote app-server clients. No automation scheduler
enablement is needed. The thread must have started a turn in this runtime before
subscribing so the harness has a real continuation context. Client disconnect does
not destroy a subscription while the owning runtime remains alive.

### `thread/events/subscribe`

Request:

```json
{
  "threadId": "thread-123",
  "topic": "ci:pr-167",
  "prompt": "Check CI and cancel this subscription when checks are terminal.",
  "intervalSeconds": 60,
  "maxDeliveries": 30,
  "ttlSeconds": 3600
}
```

Response:

```json
{
  "subscription": {
    "id": "subscription-123",
    "threadId": "thread-123",
    "topic": "ci:pr-167",
    "prompt": "Check CI and cancel this subscription when checks are terminal.",
    "intervalSeconds": 60,
    "remainingDeliveries": 30,
    "expiresAt": "2026-10-24T12:00:00Z",
    "pending": false,
    "lastError": null
  }
}
```

`delaySeconds` and `intervalSeconds` are optional positive integers from 1 through
86400. The first tick uses the delay, otherwise the interval. `maxDeliveries`
defaults to 100 (range 1..1000; one-shot delays always have one delivery); `ttlSeconds` defaults to 86400 (range 1..86400).
Topics must contain non-whitespace text and at most 128 UTF-8 bytes; prompts must
contain non-whitespace text and at most 8192 bytes. There are at most 32 live
subscriptions per thread and 128 per runtime. Validation failures, unknown/unstarted
threads, quiescing runtimes, and capacity violations return `-32602` with details.

### `thread/events/list`

Request: `{"threadId":"thread-123"}`.
Response: `{"subscriptions":[...]}` with the same subscription shape above.
Expired, cancelled, and exhausted subscriptions are excluded. Unknown threads
return an empty list.

### `thread/events/cancel`

Request: `{"threadId":"thread-123","subscriptionId":"subscription-123"}`.
Response: `{"cancelled":true}`. Unknown IDs or subscriptions belonging to another
thread return `false`; this never cancels another thread's subscription.

### `thread/events/publish`

Request: `{"topic":"ci:pr-167","message":"The CI workflow has completed."}`.
Response: `{"queuedSubscriptions":1}`. The message is bounded to 8192 bytes;
topic validation is the same as subscribe. Invalid input returns `-32602`.

## Delivery and lifecycle

The harness checks timers every second. A busy or draining thread keeps one pending
event per subscription; newer publications replace older pending publications.
Missed timer ticks coalesce rather than building a catch-up backlog. Different
subscriptions stay independent. Timer ticks do not overwrite an unpublished topic
payload already waiting for delivery.

Admission is serialized with normal turns. A tick never interrupts an active turn
or creates concurrent turns in the same thread. Delivery consumes a delivery count
when a new turn is admitted, regardless of whether that turn later succeeds.
Admission errors retain the pending event and populate `lastError` for inspection.
All tool execution and approvals use the normal harness policy and execution owner
checks. Follow-up turns emit the normal thread/turn/item notifications; there is no
separate event subscription transport to open.

User interruption discards that thread's subscriptions and pending follow-ups.
Archiving a thread or closing a subagent removes its subscriptions. Shutdown quiescing prevents new tick
turns. Paused or budget-limited thread goals defer pending events until the goal
is resumed or its budget is changed. Expiration and delivery limits stop abandoned watchers even if the model
forgets to cancel. Runtime shutdown ends all subscriptions; no external scheduler
or OS service is installed.
