# browser-use browser provider

Roder can drive a browser through the open-source
[browser-use](https://github.com/browser-use/browser-use) local MCP server. The
`roder-ext-browser-use` extension launches the server with `uvx`, talks to it
over stdio MCP (the transport lives in `roder-ext-mcp`), and exposes its tools
to the model as `browser_use_*` tools gated by Roder's approval policy.

The provider is off by default.

## Browser tools in Roder

Roder has four ways to use a browser. They can be enabled together; tool names
and descriptions say which provider each tool belongs to.

| Provider | Tools | Which browser | Best for | Needs |
| --- | --- | --- | --- | --- |
| Chrome extension ([docs](roder-chrome-browser-extension.md)) | `chrome_*` | The user's own Chrome, with their tabs and sign-ins, through the paired Roder MV3 extension | Work that needs the user's real session; console and network debugging | The unpacked extension, paired over the remote app-server |
| Direct CDP (same `chrome_*` tools) | `chrome_*` | Roder Desktop's integrated browser over CDP (`RODER_DESKTOP_CDP_PORT`, default 9334), used when no extension is paired | Navigate, snapshot, click, type and eval without the extension | Roder Desktop's integrated browser |
| Jev ([docs](jev-browser.md)) | `jev_browse` | Chrome over CDP (Roder starts one, or `JEV_CDP_URL`) | Handing one bounded goal to a fast goal-directed agent in a single call | `JEV_API_KEY` |
| browser-use (this page) | `browser_use_*` | A separate browser that browser-use launches with its own profile | Step-by-step control of a fresh, isolated browser; browser-use's own agent as a last resort | `uv` (`uvx`); an OpenAI key for its two LLM-backed tools |

Rules of thumb: use `chrome_*` when the task needs the user's signed-in
Chrome, `jev_browse` for one bounded goal, and `browser_use_*` for
step-by-step work in a browser that is not the user's. Roder also ships the
Webwright workflow ([docs](roder-webwright-browser-agent.md)), which writes
and runs Playwright scripts rather than exposing live browser tools.

## Enable

For one session:

```sh
roder --browser-use
roder exec --browser-use "open example.com and tell me the heading"
```

Always, in `~/.roder/config.toml`:

```toml
[browser_use]
enabled = true
```

Nothing starts when Roder starts. The first `browser_use_*` call launches

```sh
uvx --from 'browser-use[cli]==0.13.10' browser-use --mcp
```

and the first browser call opens the browser. The first launch downloads
browser-use and its dependencies from PyPI into uv's cache, which can take a
minute; later launches take a few seconds. Each thread starts its own server and browser. Calls in the same thread reuse
that session; other threads cannot switch or close its tabs.

With `--browser-use`, Roder checks for `uvx` at startup and prints a warning
with install instructions when it is missing.

## Settings

```toml
[browser_use]
enabled = true
# Run the browser without a window. Default false: the window is visible so
# you can watch what the agent does.
headless = false
# The browser-use release to run. Roder's tool table matches 0.13.10; Roder
# refuses to use a server that lacks any of those tools.
package = "browser-use[cli]==0.13.10"
# Full path to uvx when it is not on PATH.
uvx = "/opt/homebrew/bin/uvx"
```

Roder sets `BROWSER_USE_HEADLESS` from `headless`, sets
`ANONYMIZED_TELEMETRY=false` so browser-use does not send usage telemetry, and
never sets `BROWSER_USE_DISABLE_SECURITY` (it is removed even if present).
`BROWSER_USE_LOGGING_LEVEL` is passed through from Roder's environment, so
`BROWSER_USE_LOGGING_LEVEL=DEBUG roder --browser-use` turns on browser-use's
debug logging.

## LLM keys

Two tools run an LLM inside the browser-use server: `browser_use_extract_content`
and `browser_use_agent`. The pinned runtime needs `OPENAI_API_KEY`.
Roder passes the OpenAI and Anthropic keys it already resolved for its own
providers (environment variables, or `[providers.openai]` /
`[providers.anthropic]` `api_key` in config) to the server process, and to
nothing else:

- The server's environment is not inherited from Roder. It gets an allowlist
  of what `uvx`, Python and a browser window need (`PATH`, `HOME`, locale,
  `XDG_*`, `UV_*`, TLS and proxy variables, display variables) plus the
  browser-use settings above and these two keys. Other Roder secrets, such as
  `JEV_API_KEY` or `GITHUB_TOKEN`, do not reach it.
- Keys are never command arguments, and never appear in `Debug` output.
- The key values are scrubbed from tool results, errors and the server's
  stderr before Roder shows them to the model, the transcript or the user.

Without a key, the direct-control tools work normally, and the two LLM-backed
tools fail at once with a message saying which keys to set; the server is not
contacted. A Codex or other OAuth sign-in is not an API key and is not passed.
Which model browser-use picks when both keys are present is up to
browser-use; `browser_use_agent` takes an optional `model` argument.

## Tools

| Tool | browser-use tool | Class | Notes |
| --- | --- | --- | --- |
| `browser_use_navigate` | `browser_navigate` | navigate | `url`, optional `new_tab` |
| `browser_use_go_back` | `browser_go_back` | navigate | |
| `browser_use_scroll` | `browser_scroll` | navigate | `direction`: `up` or `down` |
| `browser_use_switch_tab` | `browser_switch_tab` | navigate | 4-character `tab_id` |
| `browser_use_get_state` | `browser_get_state` | read | URL, title, tabs and indexed interactive elements; optional screenshot |
| `browser_use_get_html` | `browser_get_html` | read | Whole page or one CSS `selector` |
| `browser_use_screenshot` | `browser_screenshot` | read | The image is attached for providers that accept images in tool output |
| `browser_use_list_tabs` | `browser_list_tabs` | read | |
| `browser_use_list_sessions` | `browser_list_sessions` | read | |
| `browser_use_extract_content` | `browser_extract_content` | read, LLM | `query`; needs a key |
| `browser_use_close_tab` | `browser_close_tab` | manage | |
| `browser_use_close_session` | `browser_close_session` | manage | |
| `browser_use_close_all` | `browser_close_all` | manage | The next call opens a new browser |
| `browser_use_click` | `browser_click` | act | Element `index` from `get_state`, or `coordinate_x`/`coordinate_y` |
| `browser_use_type` | `browser_type` | act | `index`, `text` (clears the field first) |
| `browser_use_agent` | `retry_with_browser_use_agent` | agent, LLM | `task`, optional `allowed_domains`, `max_steps`, `model`, `use_vision`; needs a key |

Every description starts with `[browser-use]`. Input schemas are the pinned
release's own. Results over 24,000 characters are truncated with a hint to
narrow the request.

Roder narrows the agent's step schema to 1–100 (default 50), and rejects invalid
limits before launching an agent. Non-object tool arguments are rejected.

## Browser scope and storage

Every thread's MCP server has a private temporary browser profile, downloads
directory and extraction files. Upstream's shared default profile and personal
browser-use configuration are not reused. The directories are removed after
server shutdown; cancellation restarts with a fresh profile.
On Unix the temporary directory is private (0700) and its configuration is
0600. The pinned server requires its OpenAI key in that configuration to
initialize content extraction; it is removed with the owned profile and
redacted from reports. An Anthropic key alone does not enable this pinned
server's LLM-backed tools.

Set `RODER_BROWSER_USE_ALLOWED_DOMAINS` to a comma-separated list of exact
hostnames to impose an operator ceiling, for example `example.com,docs.example.com`.
Direct navigation outside that ceiling is rejected before dispatch. The
autonomous agent's `allowed_domains` argument can only select a subset of the
ceiling; omitting it keeps the configured restriction. Wildcard patterns, paths
and port-specific entries are not accepted in this operator setting. Upstream's
profile watchdog also enforces it inside the browser. This is navigation scope,
not a firewall for every subresource or redirect request.

For the Desktop fallback, `RODER_DESKTOP_ALLOWED_ORIGINS` accepts exact http(s)
origins including ports. It checks direct navigation before loading and checks
current origin before subsequent reads/input/eval. An off-origin redirect is
reported after loading and stops subsequent input. Both settings are optional.

## Policy

| Class | Default mode | Accept-all | Plan mode | Bypass |
| --- | --- | --- | --- | --- |
| read, navigate, manage | allowed | allowed | allowed | allowed |
| act (click, type) | asks | allowed | denied | allowed |
| agent | asks | asks | denied | allowed |

The approval prompt for a click names the element; for typing it gives the
number of characters, not the text. The prompt for `browser_use_agent` shows
the task and the domains it may visit. Autonomous agent runs are asked about
even in accept-all mode because they can navigate, click, type and submit
forms on their own.

## Untrusted content

Results of the page-reading tools and actions that return page observations start
with a note that the content is
untrusted and must not be followed as instructions, and their result data
carries `"untrusted": true`, as the `chrome_*` tools do. Navigation, clicks, typing, and agent calls return a fresh `browser_get_state`
observation with a screenshot after the operation. The action and observation
are serialized within the thread. Agent reports remain claims: verify the actual
page state against the task. Screenshots use `detail: "original"`, and Responses
replay preserves the accompanying text and original call id.

## Lifecycle

The server runs in its own process group together with everything it starts,
including the browser.

- Roder stops the group when the provider is dropped at shutdown: it asks
  browser-use to close its browser (`browser_close_all`) when shutting down
  gracefully, then sends SIGTERM and finally SIGKILL to the group.
- A small shell guard in the same group watches Roder's pid and stops the
  group within about a second if Roder exits without cleaning up (a crash or
  `kill -9`).
- If the server crashes, the next `browser_use_*` call starts a new one.
- A thread's browser lives until Roder exits or that thread calls
  `browser_use_close_all`. There is no automatic idle eviction yet.
- Cancelling an in-flight browser tool invalidates that thread's server and
  stops its process tree. Its next call starts a fresh browser. Other threads
  retain their sessions.

On Windows the process-group and guard steps are not available; only the
direct child is killed when Roder drops it.

## Troubleshooting

- **"`uvx` was not found on PATH"**: install uv (`brew install uv`, or
  `curl -LsSf https://astral.sh/uv/install.sh | sh`), or set
  `[browser_use] uvx` to its full path.
- **"the MCP server failed to start"**: the error includes the last lines the
  server wrote to stderr. Run
  `uvx --from 'browser-use[cli]==0.13.10' browser-use --mcp` in a terminal to
  see the full output. browser-use needs Python 3.11 or newer (uv installs one
  if needed) and Chrome or Chromium.
- **"does not offer ..."**: the configured `package` is a browser-use release
  whose tools differ from the ones Roder was built against. Remove `package`
  or set it back to `browser-use[cli]==0.13.10`.
- **LLM tools fail**: set `OPENAI_API_KEY` and restart
  Roder.
- **No window appears**: check `headless` is `false`; on Linux a display must
  be available to Roder (`DISPLAY` or `WAYLAND_DISPLAY`).

## Tests

- `cargo test -p roder-ext-browser-use`: unit tests for config, command
  construction, environment and redaction, policy classes and result
  rendering, plus integration tests against a fake stdio MCP server
  (`tests/fake_server.rs`) that serves the pinned tool list. No uvx or network.
- `cargo test -p roder-ext-browser-use --test live -- --ignored --nocapture`
  starts the real server through uvx, checks its tool list and schemas against
  the pinned table, opens https://example.com in a headless browser and reads
  the page state.
- The same ignored test binary includes a local fixture proving cookie
  isolation between threads, persistence within a thread, observed click
  outcomes, screenshot attachment and domain rejection before transmission.
