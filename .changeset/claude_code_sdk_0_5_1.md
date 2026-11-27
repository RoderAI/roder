---
roder-ext-claude-code: minor
---

Upgrade to `claude-code-sdk-rust` 0.5.1 (parity with the Python Claude Agent SDK v0.2.189). Roder's tools now run as async SDK MCP tools instead of blocking a runtime thread. They opt out of the SDK's new input-schema validation so Claude's native argument shapes are still repaired. A `conversation_reset` from the CLI now ends CLI-session reuse and is reported as provider metadata. An interrupted turn now ends the `claude` child immediately rather than after a 5 s stdin-EOF grace period. The vendored SDK copy is removed: the supervised-stream patch is now part of the upstream release.
