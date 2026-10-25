---
roder-api: patch
roder-core: patch
roder-ext-subagents: patch
roder-usage-analytics: patch
roder-app-server: patch
roder-ext-mysql-session: patch
roder: patch
---

# Preserve token usage across subagent rounds, failures, and interruptions

Configured task agents now accumulate the final usage snapshot from each
inference round instead of reporting only the last round. Runtime turns count
cumulative stream snapshots once and retain observed input, output, cache-read,
cache-write, and service-tier information when a provider fails, a stream ends
with an error, a deadline stops the turn, or the user interrupts execution.

Interrupted turns carry their known usage through persisted history, app-server
notifications, and usage analytics. Thread usage metadata is written before
terminal notifications, and repeated terminal handling cannot charge it twice.
Active-turn native compaction is included once in the same terminal accounting.
Usage that a provider never reports cannot be reconstructed.
