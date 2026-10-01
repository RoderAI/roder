## 0.1.2 (2026-10-01)

### Features

#### Preserve browser observations and execute Desktop input through CDP

Keep screenshot text and original resolution in Responses tool replay, including
when older images are removed to fit the request budget. Attach Chrome extension
screenshots to the model input. Use stable document-scoped refs, real CDP input,
validated typing targets, isolated script state, and explicit action failures in the Desktop fallback.
Isolate browser-use servers by thread, serialize actions with fresh page state,
and stop the owned process tree when an in-flight tool is cancelled. Cover
browser observations and permission rejection through the ACP transport.
Give each browser-use server a private profile and file directories, enforce
optional operator navigation ceilings, and bound agent steps. Preserve Desktop
tab identity across enumeration changes. Release held input and screenshot
masks on cancellation or failure, including sessions retained by their owner.

Cancel a pending Chrome bridge command on dispatch timeout or dropped futures,
and label the paired extension action observations as untrusted page data.

Support optional caller-defined Jev completion conditions checked against fresh
UI state. Reject premature model DONE and permit bounded fallback recovery;
report unverified model completion explicitly when no conditions were supplied.

## 0.1.1 (2026-09-29)

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
