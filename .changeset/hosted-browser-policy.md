---
roder-app-server: minor
roder-protocol: minor
roder-dist-hosted: patch
roder-sdk-typescript: minor
roder: patch
roder-tui: patch
---

# Hosted browser authentication and request policy seams

Allow hosted deployments to resolve external bearer credentials into dynamic
tenant contexts, authenticate browser WebSockets through the
`roder.remote.v1` subprotocol, and apply a deployment request policy that can
rewrite or deny JSON-RPC calls before dispatch. Hosted health probes remain
unauthenticated for deployment schedulers.

Open hosted sockets now revalidate their bearer before every request and on a
bounded timer while idle, so external credential expiry and service-account
revocation stop request dispatch and notification delivery without waiting for
the client to reconnect or send another message. The gateway periodically
evicts idle tenant runtimes and stops that lifecycle loop during shutdown.

Externally resolved tenant ids now map to collision-resistant data directories;
existing lowercase slug tenant directories retain their original paths.

`turn/start` can now refresh a thread's volatile MCP bearer token without
persisting the credential in thread metadata.
