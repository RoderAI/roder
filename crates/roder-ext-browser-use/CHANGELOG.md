## 0.1.3 (2026-10-10)

### Features

#### browser_use reports honestly, shows a compact page state, and refuses native selects

- Action results put the action report first, labelled as a claim by the browser agent, before the observed page state, so a size cut never drops it. `browser_use_agent` returns only its own labelled report, with no follow-up state from a different browser.
- Stale element indexes and other known dead-end replies now count as tool errors instead of successes.
- A call that fails after it reached the server (a transport error, a timeout, the server dying, a failed observation) says that the browser and its logins are gone and that the next call starts a fresh browser, and the first successful result from the replacement browser says so. A JSON-RPC error reply to the action itself, a stale element index and a native-select refusal keep the browser and say nothing of the kind.
- `browser_use_get_state` and the state returned after navigation and actions render as one compact line per element within a 150-line / 18,000-character budget. The view ends with an exact omitted count and the next offset. `browser_use_get_state` has a new wrapper-side `offset` parameter to read further. Any state shape the wrapper does not recognise falls back to the raw state.
- `browser_use_click` and `browser_use_type` refuse an index that the last page state showed as a native `<select>`, because the pinned browser-use ignores the click yet reports success, and typing clears the select without a reliable pick. The refusal is an error that names the working routes, and nothing is sent to the browser.

### Fixes

#### browser_use keeps the browser when a healthy server answers with a JSON-RPC error

A JSON-RPC error reply from a healthy browser-use server (for example an argument that fails the tool's schema) no longer shuts the server down and drops the owned browser profile, and its error no longer claims the browser was lost. Transport errors, timeouts, a dead server, cancellation and a failed page read after an action still drop the browser and say so. The fake-server integration tests are split into focused modules.

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
