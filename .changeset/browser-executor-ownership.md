---
roder-protocol: major
roder-app-server: major
roder-core: minor
roder-sdk-typescript: minor
roder-sdk-python: minor
---

# Bind hosted external tools to one authenticated connection

Hosted external tool threads require an executor binding before a turn starts.
Only that connection receives execution requests and may resolve them with the
current lease and turn. Disconnect, unbind, and takeover terminate pending
requests. Metadata readback supports recovery without replaying effects.
The TypeScript SDK adds a generic executor helper with cancellation and duplicate
request suppression for hosts that own their existing notification loop.
