## 0.2.1 (2026-09-29)

### Features

#### Add browser-use as an opt-in browser provider over stdio MCP

`roder --browser-use` or `[browser_use] enabled = true` exposes the open-source
browser-use local MCP server as `browser_use_*` tools. Roder launches it with
`uvx --from 'browser-use[cli]==0.13.10' browser-use --mcp` on the first call,
with a visible browser by default (`headless = true` to hide it), an
allowlisted environment, and only the OpenAI/Anthropic keys Roder already
holds, which are scrubbed from every result and error. Clicks and typing ask
for approval in default mode, the autonomous agent tool asks in default and
accept-all mode, and plan mode denies both. Page content is labeled untrusted.

`roder-ext-mcp` gains `McpStdioClient`, a stdio transport that runs the
server in its own process group, answers server pings, keeps a redacted
stderr tail for errors, and stops the whole group (including a browser the
server started) on shutdown, on drop, and, through a pid guard, when Roder
dies.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

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
