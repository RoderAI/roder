---
roder-api: minor
roder-core: minor
roder-protocol: minor
roder-app-server: minor
roder-ext-openai-responses: patch
roder-ext-cursor: patch
---

# Resume threads from topic events and scheduled follow-ups

Add agent tools and JSON-RPC methods for thread-scoped topic subscriptions,
one-shot reminders, recurring timer ticks, publication, inspection, and cancellation.
Deliver follow-ups into idle threads with retained conversation context, coalesce
pending work while busy, and stop abandoned watchers with expiration and delivery
limits. User interruption and thread archive discard pending subscriptions.

Subscriptions live in the harness process. Durable independent scheduled runs
continue to use the existing app-server automation scheduler.

Expose next tick and last admitted delivery timestamps for inspecting live watchers.

Fix Codex follow-up requests by sending volatile developer context with the supported
developer input role. Clarify nullable timer fields so one-shot reminders are not
accidentally registered as recurring timers.

Preserve exact integer types in Cursor MCP arguments so timer delays, delivery
limits, and TTL values pass runtime integer validation.

Tell final-delivery turns that their subscription is already exhausted, avoiding
redundant cancellation calls after one-shot reminders.
