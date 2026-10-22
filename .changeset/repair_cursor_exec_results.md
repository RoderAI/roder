---
roder-api: minor
roder-core: patch
roder-tools: patch
roder-ext-cursor: minor
roder-ext-claude-code: patch
---

# Restore reliable Cursor native tool execution

Preserve structured, policy-approved results across provider-native tool loops.
Cursor reads, searches, listings, writes, deletes, and shell replies now use the
runtime executor's output and failures. Add current client-side MCP dispatch,
default runtime tool discovery and MCP state replies, explicit unsupported-call
errors, official turn-end usage, and reliable stream
failure and heartbeat cleanup.

Persist native checkpoints and KV blobs to preserve thread continuity. Serve KV
GETs, prefetch restored blobs, and import Roder history explicitly for new Cursor
conversations and compaction summaries. Use the same bidirectional transport for
text-only API calls and tool-enabled turns.
