---
roder-protocol: major
roder-app-server: major
roder-core: major
roder-sdk-typescript: major
roder-sdk-python: minor
roder: major
roder-dist-hosted: major
roder-evals: major
roder-ext-claude-code: major
roder-ext-postgres-session: major
roder-ext-runner-sprites: major
roder-extension-host: major
roder-tui: major
---

# Bind hosted external tools to one authenticated connection

Hosted external tool threads require an executor binding before a turn starts.
Only that connection receives execution requests and may resolve them with the
current lease and turn. Disconnect, unbind, and takeover terminate pending
requests. Metadata readback supports recovery without replaying effects.
The TypeScript SDK adds a generic executor helper with cancellation and duplicate
request suppression for hosts that own their existing notification loop.

Custom TypeScript transports expose a synchronous closedSignal so executor callbacks abort immediately on transport loss. The unused local startupTimeoutMs option is removed.

Release the reverse dependency closure together so registry builds share the new protocol and core types.
