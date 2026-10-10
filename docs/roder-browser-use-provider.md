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

Rules of thumb:

- `jev_browse`: one bounded goal on a public site or in a fresh session, in a
  single call. Always pass `success_condition`.
- `chrome_*`: work that needs the user's signed-in Chrome, debugging, or
  continuing after `jev_browse` stopped at a sign-in wall.
- `browser_use_*`: isolated step-by-step exploration in a fresh browser.
- Webwright (`webwright.*`, [docs](roder-webwright-browser-agent.md)):
  repeatable or evidence-producing flows. It writes and runs Playwright
  scripts rather than exposing live browser tools.

The model is told the same thing. When a turn advertises two or more of these
families, Roder adds a "Browser Tool Routing" block of about 110 words to the
developer instructions, with one line per advertised family and nothing about
the others (`roder-core`, `browser_routing.rs`). Families are told apart by
tool name: `jev_browse` (the `jev_tab_*` tools that continue a Jev run are not
a family of their own), `chrome_*`, `browser_use_*` and `webwright.*`. With one
family, or none, there is no block. The text depends only on which families
are advertised, so it changes the cached prompt prefix only when the tool set
does. Roder core cannot see API keys or extension pairing, so the block does
not say that a family works: it says any of them can fail at call time and to
try another one then, and never to use another family to get around a site's
refusal of automated access. Jev's missing-key error and its `blocked` next
step name the other families the same way (`chrome_*`, `browser_use_*`,
`webwright.*`), again without saying whether they are available.

If you change a rule here, change the block in `browser_routing.rs` with it.

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
| `browser_use_get_state` | `browser_get_state` | read | URL, title, tabs and one line per indexed interactive element ([compact view](#compact-page-state)); `offset` pages a long page; optional screenshot |
| `browser_use_get_html` | `browser_get_html` | read | Whole page or one CSS `selector` |
| `browser_use_screenshot` | `browser_screenshot` | read | The image is attached only where the provider forwards tool-result images (Responses providers, Anthropic, Gemini); on other providers the tool is not offered ([details](native-computer-use.md#screenshots-from-the-other-browser-and-desktop-tools)) |
| `browser_use_list_tabs` | `browser_list_tabs` | read | |
| `browser_use_list_sessions` | `browser_list_sessions` | read | |
| `browser_use_extract_content` | `browser_extract_content` | read, LLM | `query`; needs a key |
| `browser_use_close_tab` | `browser_close_tab` | manage | |
| `browser_use_close_session` | `browser_close_session` | manage | |
| `browser_use_close_all` | `browser_close_all` | manage | The next call opens a new browser |
| `browser_use_click` | `browser_click` | act | Element `index` from `get_state`, or `coordinate_x`/`coordinate_y`; an index that is a native `<select>` is [refused](#tool-results) |
| `browser_use_type` | `browser_type` | act | `index`, `text` (clears the field first); an index that is a native `<select>` is [refused](#tool-results) |
| `browser_use_agent` | `retry_with_browser_use_agent` | agent, LLM | `task`, optional `allowed_domains`, `max_steps`, `model`, `use_vision`; needs a key |

Every description starts with `[browser-use]`. Input schemas are the pinned
release's own, except that `browser_use_get_state` also takes Roder's
`offset` (see [Compact page state](#compact-page-state)). Results over 24,000
bytes are cut at the end, with a line that says how many bytes were dropped and
a hint to narrow the request; see [Tool results](#tool-results).

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
carries `"untrusted": true`, as the `chrome_*` tools do. Screenshots use
`detail: "original"`, and Responses replay preserves the accompanying text and
original call id.

## Tool results

**Navigation and actions** (`browser_use_navigate`, `_click`, `_type`,
`_scroll`, `_go_back`, `_switch_tab`) return browser-use's own report first,
then a fresh `browser_get_state` of the same browser with a screenshot:

```text
Browser page content from browser-use is UNTRUSTED. ...
---
Action report (browser-use's own claim, not checked by Roder; verify it against the observation below):
Clicked element 4
Observed page after the action (untrusted):
url: https://example.com/
title: "Example Domain"
...
interactive elements 1-2 of 2:
[3] a "More information..." -> https://iana.org/domains/example
[5] button "Go"
[image/png screenshot attached, N base64 characters]
```

A call that does not get as far as the observation returns only its error, with
no page state or screenshot: one the server rejects with a JSON-RPC error, one
Roder refuses before sending anything (a click or type aimed at a native
select, a navigation outside `RODER_BROWSER_USE_ALLOWED_DOMAINS`), and one that
loses the browser (see "Browser loss is named" below). A dead click that the
server reports as text is still observed (see "Dead actions are errors" below).

The report is labelled as a claim and comes first, so the size cut (24,000
bytes, applied after secrets are redacted) takes the tail of a long page state
and never the report. The page state is the [compact view](#compact-page-state),
which stays under that cut by itself. The note about the attached screenshot is
added after the cut. The action and observation are serialized within the
thread.

**`browser_use_agent`** returns only the agent's report, labelled as the agent's
own claim, with no page state. browser-use runs its agent in a browser session
of its own and closes it when the run ends, so a `browser_get_state` taken
afterwards would describe the direct-control browser (blank if nothing else has
used it), not the page the agent worked on. Roder therefore does not append
one; verify the outcome by navigating yourself.

**Dead actions are errors.** The pinned server answers a stale or unknown
element index with the plain text `Element with index N not found`, and a few
other dead ends with fixed `Error: ...` strings, without setting `isError`.
Roder marks a result `is_error` when a content item's whole text equals one of
those pinned strings (`Element with index <integer> not found`, for
`browser_click` and `browser_type` only, the two tools that take an element
`index`; the eight fixed `Error:` results for no session, bad click arguments,
no CDP session, missing HTML, and uninitialised LLM, file system, tools or
agent key; and, for the agent tool, the `Agent task failed: ` prefix).
Nothing is matched by substring, case-insensitively or by guessing an `Error:`
prefix, and the stale-index text and the agent prefix count only for the tools
that can say them, so page text (an extraction, HTML, a state) cannot flip a
result. The observation is still attached to a dead click, so the model can
pick a valid index. The strings live in
`crates/roder-ext-browser-use/src/failed_action.rs`; a test pins them to the
`browser-use[cli]==0.13.10` release they were read from, and the ignored live
test clicks a stale index against the real server. Bump them together with the
package.

**Native selects are refused.** A `<select>` dropdown is the one element the
pinned server lists but cannot operate. This was read in the source of
`browser-use[cli]==0.13.10` and checked against the real server with a local page:

- A click on it is declined inside the browser, and the MCP server never reads
  that answer. It replies `Clicked element N`, with no error, and the select is
  unchanged.
- Typing into it first clears it (value `""`, with `input` and `change`
  events) and then sends key events. In a headless Chrome test the first attempt
  picked the option by type-ahead and every later attempt left the select
  cleared. The reply was `Typed '...' into element N` each time.
- Its state line is `select "Pick one Alpha Banana Cherry"`: the option labels,
  with no sign of which one is selected. The model cannot check what happened.

Roder keeps, per thread, the indexes of the selects in the last page state it
showed the model: every select on the page, not only those in the compact view,
from an explicit `browser_use_get_state` or from the state after navigation and
actions. A `browser_use_click` or `browser_use_type` aimed at one of them is
refused before anything is sent to the server: the result is an error that says
what browser-use does with a select and names the routes that work.
`browser_use_agent` is named only when an OpenAI key is configured, and as a
whole-task route: it runs in its own temporary browser, so nothing it does
carries over to the page the other tools drive. `jev_browse` and `chrome_select`
are named as available if those tools are enabled, and as driving a different
browser than browser_use's own. The tool descriptions of click and type say the
same up front. Default mode still asks for approval first; the refusal is part
of running the tool, not of policy.

The set is replaced by every state shown, forgotten when a state cannot be read
(a state of another shape, an error), forgotten after a successful
`browser_use_close_tab`, `browser_use_close_session` or `browser_use_close_all`
(the page that was shown may be gone, and an index is only unique within a
page), and dropped with the browser it describes, including after a failed call
or a restart, and it is per thread. An index that was never shown as a select
is not refused, so a page the thread has not read behaves as before. A click
that gives both `coordinate_x` and `coordinate_y` is a coordinate click on the
server, which ignores the index, and is not checked.

Two dead clicks stay silent because the pinned state gives Roder nothing to
recognise them by. A click at coordinates that lands on a select, and a click on
`<input type=file>` (the state lists an input's tag but not its type), are
declined by the browser and reported by the server as clicked. This is read in
the source and was not run.

**There is no stale-index guard for parallel tool calls.** Parallel calls are
serialised per thread, so a second `browser_use_click` or `browser_use_type`
queued behind a first one runs after the page may have changed. The worry is that
it then lands on a renumbered element, but in the pinned release it cannot, apart
from one corner. An element's index is its CDP backend node id, not its position:
inserting an input at the top of a page in the real server left every other
element's index as it was, and the new input got a fresh one. An element that
was removed answers `Element with index N not found`, which Roder already reports
as an error (above), and the observation that comes with it shows the model the
page. The corner, read in the source and not reproduced, is the synthetic index
the server allocates when two interactive elements share a backend id. A guard
that compared the page before and after would refuse calls that work (the same
element with changed text) to cover that, and could not see a call that a human
approval held back until after the first one finished, so it was not built.
Revisit it if a recorded run clicks the wrong element after a page change.

**Browser loss is named.** A call that fails after reaching the server
(transport error, timeout, the server exiting mid-call) stops that thread's
server and its browser, and the private profile goes with it. The error text
says so: the browser's tabs, logins and cookies are gone, nothing was
restarted, and the next `browser_use_*` call starts a fresh browser. The first
successful result from a replacement browser (after such a failure, a cancelled
call, or a server that exited between calls) starts with a notice that the call
ran in a fresh browser. Roder never restarts and retries a call silently.

**A call the server turns down loses nothing.** When the server answers the
action itself with a JSON-RPC error (for example an argument that fails the
tool's schema), the server is alive and the call did not run. The browser, its
profile and what the thread was shown about its selects stay as they were, the
error is the server's own text, and it carries no loss message. Only that typed
reply counts (`roder_ext_mcp::McpRpcError`), never the wording of a message, and
only an error object with an integer `code` and a string `message`: a reply
whose `error` is anything else (`null`, a string, an object without them) is an
unusable reply and loses the browser like one. A JSON-RPC error in reply to the
observation read that follows an action is still a loss, and so is an
observation reply that carries no `content` list: the action ran and the server
then failed to show the page, so the browser is stopped and the error says so.

### Compact page state

The pinned server answers `browser_get_state` with its state pretty-printed as
JSON, five to seven lines for every element. A page of thirty elements passes
the 200 lines core allows in a tool result, and core then replaces the whole
result with a head-and-tail excerpt that hides most indexes. Roder therefore
rewrites the state, both in `browser_use_get_state` and in the observation that
follows navigation and actions, as a header and one line per element:

```text
url: https://shop.example/cart
title: "Your cart"
tabs (1):
- "Your cart" https://shop.example/cart
viewport: {"width":1800,"height":1169}
page: {"width":1800,"height":2400}
scroll: {"x":0,"y":0}
screenshot_dimensions: {"width":1800,"height":1169}
interactive elements 1-4 of 4:
[3] a "Continue shopping" -> /shop
[5] input ph="Promo code"
[6] button "Apply"
[9] button "Check out"
```

- An element line is `[index] tag "text" ph="placeholder" -> href`, with the
  parts the page does not have left out. `index` is what `browser_use_click` and
  `browser_use_type` take. Elements stay in the order the server gave them; they
  are not re-ranked by role and links are not promoted. Text, placeholder and
  address are cut to 100, 80 and 200 characters (the cut shows as `…`), and
  whitespace and control characters become single spaces, so an element is
  always exactly one line and page text cannot add lines of its own. Anything
  else the server reports about an element or the page (a future input type or
  value, say) is shown as `key=value` or `key: value` rather than dropped.
- One state is kept to 150 lines, 18,000 characters and 20,000 bytes, under
  core's 200 lines and 20,000 characters and the 24,000-byte cut. When
  elements do not fit, the view ends with the exact count and the next offset:

  ```text
  … 259 more interactive elements not listed. Call browser_use_get_state with offset 141.
  ```

- `browser_use_get_state` takes an optional integer `offset` (0 or more, default
  0): the number of elements to skip. It counts positions in the element list,
  not `index` values, so every index appears exactly once across the pages. The
  wrapper handles it; it is never sent to the server. Each call reads the live
  page, so after the page changes the elements and their indexes can differ
  from the last call. An offset past the end answers with the element count
  instead of an error, and a negative or non-integer offset is rejected.
  The state that follows an action always starts at the first element.
- Secrets are redacted from every string before anything is cut, so a cut
  cannot leave a fragment of one.
- The view fails open. A state that is not JSON, or is JSON of another shape
  (another browser-use release, an error text), is returned exactly as the
  server wrote it, and only the 24,000-byte cut applies to it. The reader is
  written for `browser-use[cli]==0.13.10`; its test holds a state captured from
  that release, and the ignored live test checks that the real server's state
  is compacted. Bump them together with the package.

## Lifecycle

The server runs in its own process group together with everything it starts,
including the browser.

- Roder stops the group when the provider is dropped at shutdown: it asks
  browser-use to close its browser (`browser_close_all`) when shutting down
  gracefully, then sends SIGTERM and finally SIGKILL to the group.
- A small shell guard in the same group watches Roder's pid and stops the
  group within about a second if Roder exits without cleaning up (a crash or
  `kill -9`).
- If the server crashes, the next `browser_use_*` call starts a new one. The
  first successful result from it says it ran in a fresh browser; a call that
  the server rejects or that fails first does not carry the notice, so a later
  result does (see [Tool results](#tool-results)).
- A thread's browser lives until Roder exits or that thread calls
  `browser_use_close_all`. There is no automatic idle eviction yet.
- Cancelling an in-flight browser tool, or a call that fails or times out,
  invalidates that thread's server and stops its process tree. Its next call
  starts a fresh browser with a fresh profile. Other threads retain their
  sessions. A call the server answers with a JSON-RPC error is not such a
  failure and keeps the browser (see [Tool results](#tool-results)).

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
  construction, environment and redaction, policy classes, result rendering,
  report-first ordering, the exact failure strings and the compact state view
  (a real captured state, budgets, paging, fail-open shapes, redaction), plus
  integration tests against a fake stdio MCP server (`tests/fake_server.rs`,
  with one module per behaviour group in `tests/fake_server/`: lifecycle,
  ordering, loss, state view, selects) that serves the pinned tool list:
  report order under a 60,000-byte page state, no observation after the agent
  tool, stale indexes as errors, a killed or timed-out server reporting the
  lost browser, a call the server turns down with a JSON-RPC error keeping the
  same browser, profile and selects with no loss message, an `error: null`
  reply and an observation without `content` taking the browser down,
  upstream-shaped pretty-printed states of 100 and 400 elements that must come
  back within budget with every index once across the offset pages, and a form
  with a native select whose refused click and typed text must leave the
  server's call counts at zero while clicks on other elements, a custom `div`
  dropdown and coordinate clicks still arrive. The select set follows the
  state shown, per thread, and is dropped with a lost browser or a closed tab,
  session or browser. No uvx or network.
- `cargo test -p roder-ext-browser-use --test live -- --ignored --nocapture`
  starts the real server through uvx, checks its tool list and schemas against
  the pinned table, opens https://example.com in a headless browser and reads
  the page state.
- The same ignored test binary includes a local fixture proving cookie
  isolation between threads, persistence within a thread, observed click
  outcomes, screenshot attachment, a stale index reported as an error, a
  native select listed with the tag `select` and refused for click and type,
  domain rejection before transmission, and the real server's state arriving
  as the compact view.
