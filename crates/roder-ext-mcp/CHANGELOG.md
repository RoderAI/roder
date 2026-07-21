## 0.1.3 (2026-07-21)

### Features

#### Fail closed when scoped MCP authentication is required

MCP servers can now require a thread-scoped bearer token for tool execution
while continuing to use their configured process credential for startup tool
discovery. Calls without a thread credential are rejected locally before any
HTTP request, preventing shared hosted services from falling back to a
process-wide identity.

## 0.1.2 (2026-06-26)

### Features

#### Per-thread MCP bearer token

Let a remote client scope a thread's MCP tool calls to a specific identity (for
Vex: a per-user, per-organization capability token). The client forwards the
token via a new `mcpAuthToken` field on `thread/start`; the app-server records
it in an in-memory `roder_api::mcp_auth` registry keyed by thread id, and the
MCP tool extension reads it during execution to authenticate that thread's tool
calls (falling back to the process default when absent). Tokens are short-lived
and re-supplied on each `thread/start`.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
