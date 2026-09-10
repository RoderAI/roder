---
roder-core: patch
---

# Wake `wait_agent` on mailbox activity queued before it starts waiting

`wait_agent` checked `has_pending_turn_steers`, which only sees activity that
delivery already converted into a turn steer. A message queued while the
recipient's turn was not yet registered stayed in the mailbox undelivered, so
the waiter slept through it and returned only on timeout. It now also consults
the mailbox directly, and the lifecycle test removed for flaking is restored —
it passes 20 runs in a row.
