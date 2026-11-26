---
roder-app-server: minor
roder-core: minor
roder-edit-core: patch
roder-ext-jsonl-thread-store: patch
roder-ext-mcp: patch
roder-ext-runner-hosted-common: patch
roder-protocol: minor
roder-sdk-python: minor
roder-sdk-typescript: minor
roder-tools: patch
roder-tui: patch
---

# Keep prompts durable and thread history responsive

Record accepted prompts before interruptible execution and coordinate thread-list writes across processes. Bound hosted WebSocket backlogs, omit credentials from runner errors, and renew expired MCP sessions on the next request. Page terminal history on resume and preserve line endings when applying local and remote patches.
