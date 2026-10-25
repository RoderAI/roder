---
roder-api: minor
roder-core: minor
roder-protocol: minor
roder-app-server: minor
---

# Resume threads from topic events and scheduled follow-ups

Add agent tools and JSON-RPC methods for thread-scoped topic subscriptions,
one-shot reminders, recurring timer ticks, publication, inspection, and cancellation.
Deliver follow-ups into idle threads with retained conversation context, coalesce
pending work while busy, and stop abandoned watchers with expiration and delivery
limits. User interruption and thread archive discard pending subscriptions.

Subscriptions live in the harness process. Durable independent scheduled runs
continue to use the existing app-server automation scheduler.
