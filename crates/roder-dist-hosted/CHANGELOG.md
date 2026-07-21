## 0.1.2 (2026-07-21)

### Fixes

#### Hosted browser authentication and request policy seams

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

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
