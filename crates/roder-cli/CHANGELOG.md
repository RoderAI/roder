## 0.3.9 (2026-10-10)

### Fixes

#### Harden tool process groups and the tmux environment handoff

Follow-up to the TUI suspend fix.

- On unix, a `shell` command that times out or is cancelled, and an
  `exec_command` session that times out, now has its whole process group
  killed. Tool children run in their own session, so killing only the shell left
  everything it had started running. A command that finishes by itself leaves
  its background processes alone, unless they keep its output pipes open, in
  which case the command is still waiting and the timeout applies.
- Inside tmux, the launching environment is now handed to the respawned roder
  through a private `0600` file that a small `sh` wrapper sources and deletes,
  instead of `respawn-pane -e` arguments that any local user could read from the
  process list. Non-UTF-8 values are preserved, and a failed respawn no longer
  stops the TUI from starting.
- Removes an unused import from the OpenRouter catalog module.

## 0.3.8 (2026-10-10)

### Fixes

#### Refresh the OpenRouter model catalog and stop assuming server-side compaction

OpenRouter now ships 25 catalogued routes (Kimi K3, Claude 5.5 family, GPT-6.x,
Gemini 3.7/3.8 Flash, Grok 4.6/4.7, DeepSeek V4, Qwen 3.8, GLM 5.3, MiMo 2.6,
Mistral Large 4, Muse Spark) with the context windows, image support, and
reasoning efforts reported by the live `/models` listing. OpenRouter accepts
the OpenAI `context_management` field but does not compact server-side, so the
OpenRouter entries (including Grok 4.6, which previously claimed it) now compact
client-side at 90% of the window instead of overflowing. Routes that are only
discovered at runtime now offer reasoning efforts when OpenRouter reports that
they support reasoning.

#### Keep the launching environment when roder respawns itself inside tmux

Inside tmux, roder restarts its pane once to enable extended key reporting.
`tmux respawn-pane` starts the new process with the tmux server's environment,
not the environment roder was launched with, so anything exported only in the
launching shell (provider API keys such as `OPENROUTER_API_KEY`, direnv or mise
values) was silently dropped and the relaunched TUI reported its credentials as
missing. The respawn now passes the full environment through explicitly.

## 0.3.7 (2026-10-10)

### Features

#### Browser tools report honestly and stall less

Roder's browser features now tell the agent what actually happened and stop it from going in circles.

- **`browser_use`**: the action report comes first and is never cut off, page state is one line per element with an exact omitted count and an `offset`, dead clicks and stale indexes are errors, clicks on a native select are refused with a route that works, and a lost browser says so.
- **`chrome_*`**: results lead with the outcome and stay within a line budget; `chrome_select` works on Roder Desktop; native computer use returns what happened (dialogs, new tabs, HTTP errors), never loses the screenshot, and sends Cmd chords on a Mac page.
- **`jev_browse`**: an unusable decision reply is asked again and then falls back to the frontier model, a delete on a twin row is not repeated at low confidence, runs that go round in circles stop with a named cause, and handoffs are outcomes rather than failed tool calls.
- **Providers that cannot receive tool-result images** no longer get screenshot tools or an "attached" screenshot that never arrives.
- **Webwright** failed runs explain themselves and no longer outlive a timeout.
- A short hint tells the model which browser tool family to use when several are available.

## 0.3.6 (2026-10-09)

### Fixes

#### Run Linux release archives on Ubuntu 22.04 desktops

Build versioned and rolling Linux x86_64 and aarch64 archives in a pinned
Ubuntu 22.04 container on Ubuntu 24.04 hosts so they do
not require the newer GLIBC 2.38/2.39 symbols from an Ubuntu 24.04 build host.
Verify each Linux binary's GLIBC requirement, ACP startup and opt-in Cua tool
registration before packaging.

## 0.3.5 (2026-10-09)

### Added

Run native desktop applications and control browsers through Cua Driver on the
thread's configured computer. Enable `[cua] enabled = true` to expose 19 tools
for screenshots, application/window discovery, pointer and keyboard input,
Unicode value replacement, window management and browser interaction.

The Linux backend reuses the thread's Blaxel runner. The qualified XFCE/X11
workflow launches apps, navigates folders, drags files, creates and reopens
LibreOffice documents, and drives Chrome through native and semantic browser
controls. A loopback, read-only live viewer lets you watch the desktop while
Roder works. The explicit `local-macos` backend uses the signed CuaDriver app;
macOS desktop control is qualified on Apple Silicon.

Browser tools bind to an exact native window and tab, return semantic refs and
real screenshots, navigate pages, click, type Unicode, and end the attachment.
Existing-profile attachment is disabled by default and requires both explicit
Roder configuration and an independent Cua daemon grant. The qualified Chrome
example uses a synthetic login created inside its disposable Linux sandbox.

Roder retains normal approvals, Plan-mode observation, thread/runner isolation
and image replay. Fresh observations are required after actions; stale refs
are rejected and uncertain input is never replayed.

### Notes

Provision a graphical session with the pinned Cua Driver 0.34.0 before enabling
the extension. Durable Linux deployments should pin their provisioned image.
Wayland, personal browser accounts and macOS browser attachment remain
unqualified. The X11 Chrome test includes native UI recovery after a setup
refusal; it does not establish unattended one-call browser setup.

[Setup and tool reference](../../docs/cua-computer-use.md) ·
[Reproducible Linux fixture and live viewer](../../examples/cua-linux/README.md) ·
[Validation evidence and limitations](../../docs/cua-validation.md).

## 0.3.4 (2026-10-09)

### Fixes

#### Use OpenAI Decisions as an alternative browser decision backend

Set JEV_DECISION_PROVIDER=openai to select browser actions with the Decisions
API using an OpenAI API key. Keep the shared Chrome session, text helper,
action validation, approval policy, and optional irreversible-action gate.

Expose screenshot-based selection among host-supplied native computer action
batches, with a blocked outcome and normal host-owned permissions. Add strict
live browser and computer evals and register the backend's environment settings
in distribution metadata.

Resolve single-target choices locally because the live Decisions API requires
at least two choices. Validate authenticated browser tasks and screenshot-based
native computer execution against the live endpoint.

Add a reproducible paired live Decisions/Jev browser benchmark with alternating
provider order, repeated outcome grading, latency, and provider-reported usage.

Use Decisions-specific browser questions, action-effect context and fresh viewport
images, with a text-only option and screenshot suppression around recognized
secrets. Preserve per-question refusals, stopping only when a required selection
is refused and keeping safety checks closed. Add controlled optimization variants,
wire-level diagnostics and held-out browser fixtures.

Improve Jev's text evidence with native form ownership, disabled controls and
concise contextual history. Add staged-workflow guidance and a reproducible
Jev prompt hill-climb with compound development fixtures, sealed holdout tasks,
side-effect grading, negative browser traces and candidate-order stress probes.

Keep screenshot suppression across calls after short secret input and continue with text evidence if optional capture fails. Scrub secret echoes from form and disabled-control metadata.

## 0.3.3 (2026-10-08)

### Fixes

#### Report context size after routed compaction

Complete context assembly after the selected provider compacts the first
inference request, so clients receive the size of the compacted prompt.

#### Preserve native compaction across OAuth and routed turns

Codex OAuth sessions require provider-owned compaction. Defer automatic
compaction until routing selects the provider, and keep local fallback available
for custom Responses providers that do not implement OpenAI native compaction.

#### Keep OpenAI Responses compaction provider-owned

Use OpenAI native compaction for automatic, manual, and context-limit recovery,
including models absent from the local catalog. API-key and Codex subscription
requests both use the streamed `compaction_trigger` strategy on Responses.
Preserve the full input window and encrypted boundary with retained messages;
clear parent-turn output constraints during compaction. Propagate native failures
instead of replacing conversation state with Roder text summaries.

## 0.3.2 (2026-10-07)

### Fixes

#### Align autonomous goals with Codex and Full Access

Ship corrected goal lifecycle, budgets, usage accounting, continuation context,
and fork snapshots. Goals honor the selected permission mode; Full Access
automatically permits ordinary approval requests while preserving explicit
denials and restricted child permissions.

## 0.3.1 (2026-10-03)

### Fixes

#### Add tenant-scoped runtime ownership leases with database-clock expiry and monotonic generations. Expired and superseded owners cannot renew or release a replacement owner. Fence session, checkpoint, event, and artifact writes within owner-locked transactions; unbound handles cannot write once ownership is enabled for a tenant. Add an optional monotonic runtime execution lease: reject turn/tool admission after loss, recheck after approval waits, and close hosted sockets instead of delivering stale-owner notifications. Add bounded host-backed renewal supervision that revokes authority and drains local work on loss or uncertainty. Already dispatched external actions still require host-level reconciliation after a crash.

Add one-hop authenticated owner forwarding and discard revoked cached runtimes so reconnects can resolve ownership again. Hosts must supply a trusted owner endpoint and shared replica authentication/policy.

Add idle owner sealing that waits for admitted turns, tool futures, and cleanup without cancellation, followed by bounded durable release. Hosts must stop new inbound work and only treat confirmed release as a handoff receipt.

Add replica pool drain admission and release polling. Preserve active-work recovery messages, reject new work, keep readiness separate from liveness, and retain failed release receipts. Count forwarded sockets so an empty local runtime pool cannot falsely report a fully drained replica.

Expose an optional, bounded same-port HTTP lifecycle handler for host-authenticated rollout commands. Update the hosted distribution to leave it disabled by default.

Drain forwarded sockets through an authenticated owner registration without consuming repeated user request-rate tokens. Preserve active browser results, release only idle owners, cancel registrations on rollback, and reconnect read-only subscriptions without retiring their owner.

Update dependent distributions and the TUI for the breaking hosted options, session-store configuration, and MySQL configuration module APIs.

## 0.3.0 (2026-10-03)

### Breaking Changes

#### Bind hosted external tools to one authenticated connection

Hosted external tool threads require an executor binding before a turn starts.
Only that connection receives execution requests and may resolve them with the
current lease and turn. Disconnect, unbind, and takeover terminate pending
requests. Metadata readback supports recovery without replaying effects.
The TypeScript SDK adds a generic executor helper with cancellation and duplicate
request suppression for hosts that own their existing notification loop.

Custom TypeScript transports expose a synchronous closedSignal so executor callbacks abort immediately on transport loss. The unused local startupTimeoutMs option is removed.

Release the reverse dependency closure together so registry builds share the new protocol and core types.

## 0.2.3 (2026-10-01)

### Fixes

#### Validate native browser registration and recover failed input cleanup

Declare the native computer tool in Chrome's extension manifest, reserve its
name from external function tools, and reconcile cancelled cached sessions.
Retry failed input releases before reusing a session and attempt all key, mouse,
and screenshot-mask cleanup independently. Scrub overlapping secrets and every
nested element observation. Reject malformed Desktop origin and tab settings,
check origin scope before resolving targets, and disable arbitrary eval under
an origin ceiling. Honor the configured eval provider endpoint, validate native
compaction, preserve generic image-reopen guidance, and accept documented empty
Jev completion conditions and sparse serialized controls.

#### Browser computer use and native OpenAI actions

Ship the browser observation, input, cancellation, and completion fixes in the
Roder CLI, along with the native OpenAI Responses computer tool for explicitly
bound Chrome sessions.

## 0.2.2 (2026-09-29)

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

### Fixes

#### When Jev cannot progress, Roder's full browser tools go on in the same tab

Jev acts only on the controls its snapshot offers, with a small action
vocabulary. When a run ends because of that (the model answered BLOCKED,
three steps changed nothing, its targets stayed covered, the page offered
nothing Jev can act on, or its budget ran out), Roder now falls back to its
own direct CDP tools on the same tab, with its cookies and page state, as
`JEV_FALLBACK` says:

- `auto` (default): `jev_browse` runs a bounded loop driven by the session's
  model (or `JEV_FALLBACK_MODEL`, at `JEV_FALLBACK_REASONING`, default low)
  with look, screenshot, click at a ref or x/y, hover, drag, type, any key,
  scroll, select, navigate and wait, and returns one result: the call's end
  state, Jev's own status as `jev_status`, and `drivers` with each driver's
  steps, model calls, time and tokens. `JEV_FALLBACK_MAX_STEPS`,
  `JEV_FALLBACK_MAX_SECONDS` and `JEV_FALLBACK_MAX_TOKENS` bound it.
- `handover`: the result tells the caller the full tools work on this same
  tab, names them (`jev_tab_*`) and the tab, and says where Jev stopped.
  `auto` hands over too when no model can drive it or it also fails.
- `off`: Jev's result as before.

It never runs after `needs_input`, `needs_confirmation`, `access_denied` or a
page that did not load, and it inherits Jev's rules: the allowed origins, the
irreversible-action gate (stricter, with no model to clear a shortlisted
control, and stopping any press into another site's frame), approvals and
policy modes, cookie banners refused but never accepted (untouched with
refusal off), a covered control never pressed (by a click or by Enter or
Space while it has focus), secrets never read and reported as `[secret]`,
screenshots with filled secret fields blacked out and withheld while a typed
secret shows, and page content marked untrusted.

The `jev_tab_*` tools are registered with `jev_browse` and drive only the
thread's Jev tab, under the session's lock. `roder-ext-chrome` makes its
direct CDP toolset public as `roder_ext_chrome::direct` (a `DirectSession` on
a tab by endpoint and target id, a `DirectGuard` for the owner's rules,
`direct_tools` to bind it as model-facing tools, and the shared `devtools`
helpers Jev's own connection now uses); Roder Desktop's integrated browser
fallback goes through the same client and gains a screenshot.

Jev also uncovers a covered target before giving up: Escape, the covering
layer's own close control, or a press outside it, then the action goes ahead
in the same step. Escape and Enter are sent without a native key code, which
on macOS made Escape open Chrome's "About Chrome" page from a shown tab.

Breaking: `JevExtension` is a struct (`JevExtension::new()`,
`with_inference_engines`); `JevToolContributor::new(engines)`;
`JevActOutcome` and `JevActionRecord` gain `uncovered`; `JevRunResult` gains
`stop_cause` (the new `JevStopCause`).

#### Jev results show the page, and Jev reports access blocks and reads widget frames

The model only ever reads a tool result's text, and `jev_browse` used to give
it one line ("done at <url> (3 actions)"). The text is now a bounded digest
of the call, at most 8,000 characters and 120 lines:

- A header with the status, the session call and tab, today's date, time and
  time zone, the page's address, title and HTTP status (marked
  page-supplied), the outcome and what to do next. After `done` the caller is
  told to check the page against the goal, how to continue with `url ""`, and
  not to sign in, reserve, pay or send personal details unless the user asked.
- The session's tabs, totals and earlier calls.
- Between marker lines that say it is untrusted: why Jev stopped, each step
  with the section its control sat in and its effect, the text of frames Jev
  read, the headings, the page text, and the options Jev can act on, grouped
  by the card or section they sit in.

The result's data gains `controls`, `page` (`http_status`, `headings`,
`frames`) and each step's `effect` and `context`. `JevActionRecord` gains a
`context` field; `JevControl`, `JevPageFacts` and `JevFrameText` are new
public types, and `JevBrowser` gains a default `describe` method.
`JEV_SESSION_LOG=<dir>` appends one JSON line per call for grading.

Jev also looks before it decides:

- A site that refuses automated access (HTTP 401, 403 or 429 with a refusal
  page, a challenge address, or an "Access Denied" page with nothing to act
  on) ends the call with the new status `access_denied` before any decision,
  with the evidence. Jev only reports it and never tries to get around it.
- A first observation that shows nothing yet is read again for up to about
  2.6 s, and a BLOCKED about a blank page is checked once, at no decision's
  cost.
- The text of up to two large, visible frames of another origin (a booking
  widget) is read into the page text and the result; their controls are not
  offered. `JEV_FRAME_TEXT=0` turns this off.

#### Jev keeps one browser session per thread

`jev_browse` calls on one Roder thread now share a session: the first call
opens a tab, and later calls go on in that tab from where the last one
stopped instead of opening a new tab each time.

- `url` is optional: `""` continues on the tab's page without reloading; a url
  loads in the same tab (after its new document commits) and is not reloaded
  when the tab is already there. A new `tab` argument takes `current`
  (default), `new`, `reset` or `close`. At most three tabs stay open; tabs an
  action opens are kept; tabs are no longer closed at the end of a call,
  background ones included.
- The per-call `allowed_origins` argument is removed. A call may go to any
  origin unless the operator sets `JEV_ALLOWED_ORIGINS`. `null`, `""` and `[]`
  mean "not given" for every optional argument.
- A session keeps its last eight calls, running totals, typed secrets (still
  scrubbed from later page reads) and resolved models; each result's text
  names the tab and how the call came to be on it, the tabs open, the totals,
  earlier calls and how to continue, and `data.session` holds the same. The
  tool description states today's date and time zone.
- A closed or moved tab is reported and recovered; overlapping calls on one
  thread wait for each other (or return `busy`); idle sessions close after
  `JEV_SESSION_IDLE_SECS` (default 1200), and a per-process ledger lets a
  later process close the tabs of one that died.
- The approval names the thread's tab and where Jev works in it; `tab:
  "close"` is allowed in every policy mode.
- `JevRunResult` gains a crate-private field, so it can no longer be built
  outside the crate.

## 0.2.1 (2026-09-28)

### Fixes

#### Harden Jev's browsing: settling, reach, twin context and retries

Jev now owns its in-page scripts, prompts and fixtures, and diverges from
upstream Jev Ultrafast on purpose (the crate README lists every divergence).
Still one decision round trip per step.

- Reads wait for the page to go quiet (200 ms without mutation or input, no
  visible loading indicator, capped at 2.5 s) instead of two animation frames.
- Controls above and below the viewport that a scroll can reach are offered as
  `offscreen`; lists keep their pagers under the action caps; `act.js` scrolls
  to its target, hit-tests five points and reports a covered target as the new
  `Covered` error, which counts towards the stall rule.
- Controls that read alike carry a `context` naming their card, row, section or
  table column.
- Controls cut off by a collapsed or `overflow: clip` ancestor are no longer
  offered.
- DONE straight after a covered attempt ends blocked, and an unsure repeat of
  the click that just took effect ends done without clicking again.
- The decision and text-helper transports reuse one client, retry transient
  failures with backoff and `retry-after`, resend a possibly billed request at
  most once, and never retry past the run's deadline.
- A run that stops early says why: new statuses `budget_exceeded`,
  `timed_out`, `needs_input`, `unavailable` and `error` replace the `ready`
  such runs used to report, and the tool result adds a `next_step` hint for
  every status but `done`.
- `timeout_seconds` now covers the whole task, including starting Chrome and
  loading the page, and is cut to the host's remaining tool deadline; a setup
  phase that runs out ends `timed_out` naming the phase.
- A start page that fails to load is retried on transient `net::ERR_…` errors,
  then ends `blocked` ("could not load (…)") without a decision. Jev's tab is
  closed on every failure after it was created, including Chrome's error
  pages, which used to leak.
- `JEV_CDP_URL` attaches to a browser by http(s) address or ws(s) websocket;
  the tool result shows a non-loopback endpoint only as scheme and host.
- The start URL must be an http(s) URL with a host.
- Snapshot names skip script and style text, `aria-labelledby` resolves in a
  shadow root first, and hydrating `aria-disabled` to `"false"` no longer
  makes a decision stale.
- JavaScript dialogs no longer stall a run for 30 s: alerts and
  `beforeunload` are accepted, confirms and prompts dismissed, each is
  reported before the next observation's page text and on the step before
  it, and a run left `blocked` by a dismissed confirm says so.
- A click or fill moves the pointer first and presses only once the
  unchanged target stays under it (for up to 1 s), so hover handlers run and
  a layout shift cannot redirect the press; about 30 ms per click.
- After a fill into a field that offers suggestions, or a click on a control
  that announced a popup, the settle waits up to 1.2 s for the options or the
  popup.
- Fills select by script instead of the select-all accelerator, follow focus
  to an editor the click opened, verify the text stayed, type once more into
  an editor that took focus late, and set native date and time inputs to
  their ISO value; about 150 ms per fill. A fill or select the page refuses
  is recorded on the step, not an error.
- Styled checkboxes and radios are offered through their label, and anchors
  without `href` that handle clicks as buttons.
- The decision request's state and the text helper's field context carry
  today's date, and a native date field's ISO format.
- A foreground task's tab is shown when it is created rather than after the
  first observation.
- Breaking: `JevBrowser::act` returns the new `JevActOutcome`; observations
  may carry `dialogs` (new `JevDialog`); `JevActionRecord` gains `refused`
  and `dialogs`; `JevBrowser::activate` and `JevEngine::activate` are
  removed.
- Breaking: `JevActionRecord` gains `covered` and `target_confidence`;
  `JevDecision` gains `target_confidence`, `model` and `call_confidence()`;
  `JevDecisionRecord` gains `target_confidence` and `model`.
- Breaking: `JevStatus` is `#[non_exhaustive]`, serializes in snake case, and
  gains `BudgetExceeded`, `TimedOut`, `NeedsInput`, `Unavailable` and `Error`;
  runs that ended on a budget, timeout, missing value or provider failure no
  longer report `Ready` (or `Blocked` for the action budget). New public
  `JevStop` error carries a status out of a decision client, transport or
  text resolver.
- Breaking: `BU_CDP_URL` and `BU_CDP_WS` are no longer read (they never
  worked); set `JEV_CDP_URL` instead.
- Elements with a click listener (read with DevTools' `getEventListeners`),
  an `onclick` handler, a `tabindex` or a pointer cursor are offered as
  buttons, or as options in a floating list, unless they sit inside a control,
  cover more than a third of the viewport or only wrap other controls; an
  unnamed one is named by its picture's file. MiniWoB++ went from 200 to 357
  of 645 episodes over all the changes below, 36 tasks became expressible,
  and a 28,600-element page's snapshot went from 58 ms to about 130 ms.
- Fields are never named by their own content; an unlabelled one is named by
  the short text beside it (a label with no `for`, a header cell in its row).
  Text fields carry `input_type` (withheld from the decision model for native
  date inputs), their values are page text (never a password's), a select
  offers its current option, and text a scroll box hides is not page text.
- The text helper is told the field's `context` and kind and the other
  fields' values (`other_fields`).
- Boxes that scroll on their own are offered as `SCROLL_REGION_DOWN` and
  `SCROLL_REGION_UP` targets with a `scrolled` position; `PRESS_ENTER` in the
  field Jev typed into and a `PRESS_ESCAPE` control are offered while useful
  (the current model has not chosen either).
- Twin contexts read short rows whole and name unnamed grid cells by row and
  column.
- A multi-line value typed into a single-line input is no longer reported as
  refused.
- Controls in open shadow roots and same-origin iframes are observed, named,
  read as page text and hit-tested where they are drawn (through
  `shadowRoot.elementFromPoint` and frame coordinate mapping); closed shadow
  roots and cross-origin frames are still out of reach.
- A tab opened by an action (`target="_blank"`, `window.open`) is followed:
  Jev adopts it, reads it from then on and records `opened_tab` on the step;
  every tab Jev owns is closed with the page.
- Each step records `effect`, what it visibly did, next to `page_changed`
  (not sent to the decision model yet).
- `JEV_ALLOWED_ORIGINS`, narrowed by a call's new `allowed_origins`, ends a
  run `blocked` on a page outside it and is named in the approval request;
  `JEV_MAX_ACTIONS` sets the action budget and `JEV_MAX_SECONDS` caps
  `timeout_seconds`. The result carries summed decision and text `usage`
  (`"unknown"` where a provider did not report a count).
- Chrome is found through its profile's `DevToolsActivePort` after the port
  probe and launched with `--remote-debugging-port=0`; a Chrome that exits
  fails at once, its stderr goes to `jev-chrome/chrome-stderr.log`, the
  750 ms startup pause is gone, and a profile Roder creates starts with the
  password manager off.
- Breaking: `JevRunResult` gains `usage` (new `JevUsage`, `JevCallUsage`,
  `JevTokenCount`); `JevActionRecord` gains `opened_tab` and `effect`;
  `JevEngineConfig` gains `with_max_actions`, `with_max_duration` and
  `with_scope` (new `JevOriginScope`); observations may carry `opened_tab`.
  Code that builds these structs with literals must add the fields.
- Breaking: a Chrome Roder starts no longer listens on 9222 (or
  `JEV_CDP_PORT`) but on a port it picks, advertised in the profile's
  `DevToolsActivePort`.
- Launches on Jev's Chrome profile are serialised (a process-wide lock and a
  lock file beside the profile) and never delete `DevToolsActivePort`: a
  launch that hands off to a Chrome already running on the profile uses that
  Chrome, and parallel tasks start one Chrome.
- An http(s) `JEV_CDP_URL` with a path or query is looked up at
  `<path>/json/version?<query>`; DevTools messages holding half an emoji no
  longer fail to decode.
- A fill whose press navigated is recorded as a refused step; a tab an action
  opened becomes current only once its setup succeeds; closing a tab Chrome
  already dropped succeeds; a deadline during the new tab's attach closes it;
  500, 504 and 524 are resent at most once; a huge `JEV_MAX_ACTIONS` no
  longer overflows.
- Billed calls whose answer could not be used (a decision that failed
  validation, a text helper reply without a value) count in `usage` and
  `model_calls` (new public `JevBilled`).
- Effects no longer pair controls by node id across a navigation or tab.
- The snapshot reaches controls in an app shell's scrolling pane,
  `display: contents` links and text, a transparent native select, text
  directly in a shadow root and slotted text in place; an icon element drawn
  in a shadow root inside a button no longer becomes a phantom control that
  covers the button; `tabindex` alone no longer makes a slider, tab panel or
  code block a button; editable boxes and ARIA text boxes are not named by
  their content; masked secrets (`-webkit-text-security`, password or
  one-time-code autocomplete) are secret fields like passwords; a select's options
  count once against the action caps; strings are cut on whole characters.
- A suggestion list still showing the last query's options is waited past;
  a field that formats typed text (a phone mask) or normalises a datetime is
  no longer refused; page scrolls move the page, not the scroller under a
  fixed point.
- The tool no longer tells callers to put credentials in the goal.
- Adds scripted agent-loop tests, a real-Chrome fixture harness and a
  40-task end-to-end eval corpus with a keyless tier and an opt-in live tier.
  `JEV_REQUIRE_CHROME=1` (or `CI`) makes the Chrome-backed tests fail rather
  than skip when no Chrome is found.
- Opt-in irreversible-action gate, off by default (`JEV_CONFIRM_IRREVERSIBLE=1`
  or `JevEngineConfig::with_irreversible_gate`): the same decision request
  also asks a Noul about each Enter and each click whose label names a
  commitment (at most 8), and a run whose chosen action may not be undone
  (P above 0.5, or no valid answer) ends with the new status
  `needs_confirmation`, nothing dispatched, unless the call sets the new
  `authorize_irreversible` argument and the decision's call confidence is at
  least 0.90. A call that sets `authorize_irreversible` needs approval even in
  accept-all mode. Thresholds and question shape are fastbrowse's and
  unvalidated against the hosted model. With the gate off the decision
  request is unchanged.
- Cookie-banner refusal, on by default (`JEV_REFUSE_COOKIE_BANNERS=0` or
  `JevEngineConfig::with_cookie_banner_refusal(false)` turns it off): Roder
  now bundles DuckDuckGo's autoconsent 16.40.0 (MPL-2.0, unmodified, with its
  licence; SHA-256 pinned by a test; notice in
  `third-party/duckduckgo-autoconsent/`), injected into every document of
  every tab Jev owns, adopted tabs included, before it loads, in its own
  opt-out mode, with its heuristics held to refusing (its own `tier2`
  default also presses a lone "Accept" or "OK"). Jev's own `consent.js` stays as the fallback for banners
  autoconsent does not know, clicking an unambiguous banner's reject button
  once per document, never an accept or settings button. They never both act
  on one document: Jev waits for an opt-out autoconsent is running, and the
  fallback clears autoconsent's action when it clicks. Either refusal is
  recorded as a `cookie_banner` step with no model call. Injecting it costs
  about 30 to 55 ms more to open a small page and 70 to 135 ms on a large
  one, 0 to 70 ms on the first read, and makes the `roder` binary 445,824
  bytes larger.
  The crate's licence expression is now `MIT AND MPL-2.0`.
- Password and one-time-code fields can be typed into: they are offered as
  fills with `input_type` `password` or `one-time-code` and `filled`, never
  read (not their value, page text, guard, effects or the text helper's
  other fields), the fill check for them runs in the page and returns only a
  boolean, and what Jev types there is recorded as `[secret]` everywhere and
  scrubbed from what the page later shows. The text helper types a password
  or code only when the goal holds it word for word; otherwise the run ends
  `needs_input` naming the field. A `needs_input` from the text helper or a
  missing text model now names the field. The tool description and docs say
  plainly that a secret in the goal goes to the hosted decision service, the
  text model and the transcript, and that a `JevTextValueResolver` avoids
  that. MiniWoB++'s `enter-password`, `login-user` and `login-user-popup`
  are labelled supported (not yet run).
- The text helper writes with GPT-6 Sol (`gpt-6-sol`) at low reasoning
  effort whenever Roder holds a ChatGPT/Codex sign-in, ahead of the calling
  turn's model and the fallback providers, over the Responses API: the
  request is built by `roder-ext-openai-responses`, the reply held to a
  strict JSON schema for `{"text": string | null}` and then to the same
  checks as before, the token taken (and refreshed) from `roder-codex-auth`,
  and a 401 sent once more only when the stored token changed meanwhile.
  `JEV_TEXT_MODEL` alone now picks a catalog model through the provider that
  serves it (`deepseek-chat` forces DeepSeek), and the new
  `JEV_TEXT_MODEL_REASONING` (`none`, `low`, `medium`, `high`) sets the
  effort on either path; an explicit model or effort Roder cannot serve
  fails the call instead of being substituted. A default GPT-6 Sol choice
  whose stored sign-in cannot produce a usable token (refresh refused,
  expired, or its token refused with 401) falls back for the rest of the run
  to the turn's model or the provider list, and says so. The tool result's
  `text_model` names the model that actually wrote values, gains `effort`
  and, for such a stand-in, a `note`; its source may be `codex`. Provider
  rejections (any 4xx but 401) now carry their trimmed body.
- The live eval tier with `JEV_EVAL_TEXT=model` gives secret fields the
  task's own values, as a supervisor's resolver would, and records each text
  call's latency, usage and outcome in the row; `one_time_code_missing`
  accepts `blocked` as well as `needs_input` live (nothing typed or posted
  either way), and task `status` may list several statuses.
- Breaking: `JevStatus` gains `NeedsConfirmation`; `JevDecision` and
  `JevDecisionRecord` gain `irreversible`; `JevDecisionClient` gains the
  provided method `choose_gated` and `JevBrowser` the provided method
  `refuse_cookie_banner`; `JevEngineConfig` gains `with_irreversible_gate`,
  `with_irreversible_authorized` and `with_cookie_banner_refusal(bool)`, and
  banner refusal is on unless that is given `false`.
- The `roder` binary names its TLS crypto backend at start. Both rustls
  backends are compiled in, so a TLS websocket (the Codex provider's, or a
  `wss` `JEV_CDP_URL`) panicked.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.25 (2026-09-26)

### Features

#### Select Codex as a complete agent backend for Roder

Add an extension service for complete agent backends and a Codex app-server implementation. Roder app-server maps its thread, turn, streaming event, and approval APIs to Codex, so existing Roder clients and the TUI can use Codex as the agent runtime.

Refresh GPT-6 and Claude Opus 5.5/Sonnet 5 catalog entries and map Codex token usage into Roder's turn counters.

### Fixes

#### Per-turn OpenAI service tier (Fast mode), and GPT-6 Sol and Luna

A caller can now choose the OpenAI service tier for a turn.
`StartTurnRequest::service_tier_override` (for example `"priority"` for Fast
mode) is carried to every inference round of the turn as
`RuntimeHints::service_tier`. The OpenAI Responses provider sends it as the
top-level `service_tier` request field only on the OpenAI profile; OpenRouter,
xAI, and Fireworks never receive it. `None` keeps the provider default.

The tier OpenAI reports it actually served (`response.service_tier`) is recorded
on `TokenUsage::service_tier`, so a biller can tell a request that ran fast from
one that was downgraded to `"default"` under load. Both new fields are optional
and default to absent when older payloads are deserialized.

`StartTurnRequest` gains a public field, so code that builds it with a struct
literal must add `service_tier_override: None`.

The OpenAI/Codex catalog adds `gpt-6-sol` (efforts `low` through `max`, like
`gpt-6-astra`) and `gpt-6-luna` (efforts `low` through `max`, like
`gpt-5.6-luna`), so per-turn reasoning validation accepts them.

#### Persist browser pairing across Roder restarts

Pairing the browser extension used to last exactly as long as one Roder process:
`/remote` minted a fresh random token on every start and listened on an
OS-assigned port, so the endpoint and token the extension had stored were both
dead after a restart and the user had to walk the `/pair` flow again.

The token and the bound loopback port are now stored in
`<config-dir>/remote-pairing.json` (owner-only, never logged) and reused, and a
Roder that finds that file brings the listener back up on the same endpoint at
startup. Pair once and later runs reconnect with no user action. `/remote
regenerate` — new alias `/remote unpair` — rotates the token and revokes every
paired browser; a remembered port that is already taken falls back to an
ephemeral port, which is then remembered in turn.

Also fixes a stack overflow that aborted the TUI on its first turn. `main` used
a bare `#[tokio::main]`, so the agent-loop future ran on a worker thread with
tokio's 2 MiB default stack — the Windows main, the roadmap TUI and the
app-server already spawn themselves on 32 MiB for exactly this reason. All the
Roder runtimes now set that stack size for their worker threads too.

## 0.1.24 (2026-09-12)

### Fixes

#### Add Gemini 3.7 Flash and Grok 4.6 to provider model catalogs

Expose Gemini 3.7 Flash and Grok 4.6 through native, Cursor, xAI, SuperGrok,
and OpenRouter integrations with provider-specific context windows and
reasoning controls. Retire Grok 4.5 and Grok Build from active catalogs.

#### Ship local hooks, new frontier models, and wait_agent mailbox wakeup

The `roder` CLI now includes Codex-compatible local project hooks (session-scoped
`SessionStart`, deny/chaining fixes, tighter diagnostics), Gemini 3.7/3.8 Flash,
Grok 4.6, GPT-6 Astra, and Claude Fable 5.1, plus a `wait_agent` fix so mailbox
activity queued before the waiter starts is observed.

## 0.1.23 (2026-08-06)

### Fixes

#### Fix Shift+Enter newline in the composer

Make Shift+Enter insert a newline instead of submitting. Inside tmux, enable
CSI-u extended keys (respawning the pane once when needed) so the terminal can
distinguish Shift+Enter from Enter. Also accept Ctrl+J and Alt+Enter as newline
fallbacks when the terminal cannot report modifiers.

## 0.1.22 (2026-08-06)

### Fixes

#### Fix DeepSeek thinking mode reasoning in Ctrl+P and tool rollouts

DeepSeek models advertise real thinking efforts again, stream
`reasoning_content`, send the DeepSeek `thinking` toggle, and pass CoT back on
tool-call turns.

## 0.1.21 (2026-08-06)

### Fixes

#### Expand Codex-style `user://` tool paths to `$HOME`

File lookup tools resolve skill-style `user://...` paths against the home
directory and `workspace://...` against the workspace root, so agents can open
canonical skill paths instead of treating the scheme as a literal relative path.

## 0.1.20 (2026-08-05)

### Fixes

#### Add Ultra mode as a first-class multi-agent mode for any model

Make Codex Ultra's proactive multi-agent policy a concrete Roder mode
(`/ultra`, `thread/set_ultra_mode`, `settings/get.ultraMode`,
`ultra/modeChanged`), available for every model — not only Sol/Terra Ultra
reasoning effort. Sol/Terra Ultra effort still maps to max wire effort and
enables proactive multi-agent without requiring the mode flag.

Also: `task` / `agent_swarm` children inherit the parent thread's live
provider+model (so SuperGrok stays on grok-4.5), and lane `max_concurrent`
can be raised per request for large fanouts instead of hard-failing at the
old scout cap of 4.

## 0.1.19 (2026-08-04)

### Fixes

#### Recover from Grok prompt-length overflows and advertise xhigh

Detect xAI/Grok `maximum prompt length` (and related context-overflow) errors as
context-limit failures, then force-compact and retry the live turn in place
(with a second attempt that strips the last bulky item). Compaction summary
inference shrinks its head and falls back to deterministic summaries when the
summary request itself overflows. Grok reasoning catalogs now include `xhigh`.

## 0.1.18 (2026-08-03)

### Features

#### Add `/review`: a read-only review sub-turn with structured findings and pluggable review publishers

`/review` runs a detached, read-only reviewer over the working diff, a base
branch, a commit, or a free-form scope, and returns prioritized findings with
file/line locations. Findings render in a new TUI panel where they can be kept
or dropped, and are exposed over the app-server as `review/start`,
`review/publish`, and `review/publishers/list` plus `review/started`,
`review/completed`, `review/failed`, and `review/published` notifications.

Publishing goes through a new `ReviewPublisher` extension service. The
first-party `roder-ext-github-review` publisher submits findings as GitHub pull
request review comments over the `gh` CLI or the REST API, with diff-hunk
prefiltering and a dry-run mode. Configure it under `[review]` and
`[review.publishers.github]`. Each `[review.publishers.<id>]` block is stored
opaquely and parsed by the publisher's own crate, so adding a platform does not
change the core config types.

Also fixes `roder app-server`, which built its Tokio runtime in current-thread
mode. Providers that bridge a synchronous callback back into async work call
`tokio::task::block_in_place`, which panics outright on a current-thread
runtime, so the `claude-code` provider aborted the server on its first
Roder-executed tool call. The app-server now uses a multi-threaded runtime like
the TUI entry point.

## 0.1.17 (2026-07-23)

### Features

#### Parallel search + extract web tools

Fix Parallel.ai Search against the current V1 API (`advanced_settings` for
max_results/domain filters), add `parallel_extract` for URL markdown extraction,
auto-install Parallel tools when it is the selected web_search provider, and
inject short Parallel web-access instructions into the developer prompt when
those tools are available.

## 0.1.16 (2026-07-23)

### Features

#### Add DeepSeek Platform inference provider

Adds first-class `deepseek` provider support labeled "DeepSeek Platform", using
DeepSeek's OpenAI-compatible Chat Completions API at `https://api.deepseek.com/v1`
with `DEEPSEEK_API_KEY` auth and built-in models `deepseek-chat`,
`deepseek-reasoner`, `deepseek-v4-flash`, and `deepseek-v4-pro`.

## 0.1.15 (2026-07-21)

### Fixes

#### Fix OpenCode DeepSeek multi-step tool rollouts

Refresh the OpenCode Zen model catalog (drop disabled free DeepSeek IDs, add
current free models and paid `deepseek-v4-flash` / `deepseek-v4-pro`), coalesce
parallel tool calls into valid chat-completions histories for longer DeepSeek
rollouts, and surface clearer OpenCode ModelError/CreditsError messages.

## 0.1.14 (2026-07-21)

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

## 0.1.13 (2026-07-21)

### Features

#### Eval loop persistence: continue past tool-failure limit and nudge on empty finalization

Adds two `[reliability]` knobs so eval-style runs keep working instead of
finalizing early:

- `continue_on_failure_limit` (default `false`): when a turn hits
  `max_consecutive_tool_failures`, reset the consecutive-failure counter, nudge
  the model to keep going, and continue the round loop instead of stopping.
  Bounded by `max_tool_failures_per_turn`, `max_model_calls_per_turn`, and the
  per-turn tool-round cap.
- `empty_tool_call_nudges` (default `0`): number of times a non-interactive/eval
  turn is nudged to verify completeness when the model returns a final message
  with no tool calls, before genuinely ending the turn.

Both default to the safe (off) behavior for interactive and plain
non-interactive runs; the `eval` runtime profile enables them by default
(`continue_on_failure_limit = true`, `empty_tool_call_nudges = 1`), and explicit
`[reliability]` config keys still override the profile defaults. Also strengthens
the eval-profile persistence instructions to keep working until the task is fully
solved and verified.

#### Add bounded lifecycle recovery, cleanup proof, and shutdown diagnostics

Roder now persists redacted per-turn lifecycle records, reconciles interrupted
turns after restart, and reports bounded cleanup ownership rather than treating
an aborted runtime task as proof that provider work was reaped. Local process
tasks drain through graceful signal, forced kill, and reap; remote tasks use the
remote runner cancellation API; and the Claude Code provider uses a vendored SDK
cleanup path with offline real-child regression coverage.

The app-server adds lifecycle notifications, `runtime/drain`, and
`lifecycle/metrics`; the CLI and TUI expose durable recovery state. A shared
`[lifecycle]` configuration controls shutdown budgets, task policy, bounded
process diagnostics, and compatible legacy shutdown fallbacks.

## 0.1.12 (2026-07-10)

### Fixes

#### Match Codex V2 Ultra agent lifecycle semantics

#### Added

- Added Codex V2-style canonical agent trees, full/empty/last-N context forks,
  nested agents, reusable follow-up turns, mailbox-aware waiting, and
  non-destructive interruption.
- Added exact parent model, provider, Ultra reasoning, workspace, policy, tool,
  runner, and live developer-context inheritance for spawned agents.
- Added full `team/started`, `team/member/started`, and terminal result details
  to the app-server protocol.

#### Changed

- `send_message` now queues coordination without starting an idle agent, while
  `followup_task` starts or steers the existing canonical agent thread.
- Child final results and terminal errors are delivered automatically to their
  direct parent, and completed identities remain available for later work.
- Inter-agent delivery now uses typed `MESSAGE`, `NEW_TASK`, and `FINAL_ANSWER`
  envelopes with canonical sender and recipient paths.

#### Fixed

- Fixed spawn-capacity, completion/follow-up, interruption, mailbox batching,
  acknowledgement, restart, and wait races found by comparison with Codex V2
  and Claude Code agent workflows.
- Prevented full-history children from replaying parent orchestration by making
  the newest `NEW_TASK` payload the authoritative child assignment.
- Preserved spawn-time live instructions, developer context, and model
  selection across reusable follow-up turns.
- Prevented interrupted-turn mailbox reservations from stranding queued
  messages or accepting stale delivery acknowledgements.
- Bounded recursive agent paths to five levels below `/root`, rejecting deeper
  spawns before creating team or thread state.

## 0.1.11 (2026-07-09)

### Fixes

#### Add GPT-5.6 Codex models and Ultra mode

Expose GPT-5.6 Sol, Terra, and Luna plus GPT-5.4 in the OpenAI and Codex
catalogs, with the current context windows, defaults, and reasoning-effort
menus. Make Sol the default Codex model.

Keep Ultra as a first-class Roder effort for Sol and Terra while mapping it to
the provider's `max` wire effort. Ultra enables proactive, bounded multi-agent
delegation; lower Sol and Terra efforts remain explicit-request-only.

## 0.1.10 (2026-07-09)

### Fixes

#### Add Grok 4.5 to xAI and SuperGrok providers

Expose `grok-4.5` (500k context, default high reasoning, low/medium/high) as the
default model for both the `xai` API-key provider and SuperGrok OAuth. Keep
legacy Grok 4.3 / 4.20 and SuperGrok Build/Composer entries selectable.

## 0.1.9 (2026-07-07)

### Fixes

#### Make provider browser auth robust on WSL and add Kimi Code API key login

Print auth URLs before opening browsers, fall back to WSL-friendly browser commands, and allow `roder auth login kimi-code --api-key [KEY]`.

## 0.1.8 (2026-06-30)

### Fixes

#### Dependency refresh and runner lifecycle method manifest

Register the `runners/pause`, `runners/resume`, `runners/detach`, and
`runners/rejoin` methods in the app-server method manifest and regenerate the
checked-in JSON schema and the TypeScript/Python generated client types so they
expose the runner lifecycle surface.

Refresh dependencies across the workspace and SDKs after validating each change:

- Rust: semver-compatible lockfile updates plus major bumps of `which`
  (6 -> 8), `tokio-tungstenite` (0.28 -> 0.29), `rcgen` (0.13 -> 0.14),
  `rusqlite` (0.38 -> 0.40), and `sqlx` (0.8 -> 0.9). Fixed a `time` 0.3.52
  deprecation (`format_description::parse` -> `parse_borrowed`). The
  `agent-client-protocol-schema` 1.x major is deferred because it renames the
  ACP type surface and needs a dedicated ACP-compliance migration.
- TypeScript/edit-tools: bump `@types/node` to v26.
- Python SDK: refresh the uv lock (anyio, pytest, pyright, idna).

## 0.1.7 (2026-06-30)

### Features

#### Blaxel sandbox runner with pause, resume, detach, and rejoin

Replace the placeholder Blaxel runner passthrough with a first-party Blaxel
Sandboxes provider that drives the real control-plane (`/sandboxes`) and
per-sandbox (process/filesystem/preview) REST APIs.

The remote-runner contract gains optional, defaulted lifecycle support so a
runner-bound thread can pause its sandbox toward standby, resume it, fully
detach (releasing the local session while keeping the sandbox alive), and
rejoin the same sandbox from persisted thread state — including across a
process restart, with no orphan sandbox creation. New `RunnerCapabilities`
flags (`pausable`, `detachable`) and `RemoteRunnerSession`/`RemoteRunnerProvider`
methods (`pause`, `resume`, `detach`, `rejoin_session`) default to no-op/false so
existing providers are unchanged.

Exposed through new app-server JSON-RPC methods (`runners/pause`,
`runners/resume`, `runners/detach`, `runners/rejoin`) and a `roder runners` CLI.
The Blaxel credential is sourced from the environment (`BLAXEL_API_KEY` /
`BL_API_KEY`, with `BL_WORKSPACE`) and never written to session state.

A selected runner now actually routes coding tools into the sandbox: a
runtime-level destination (TUI runner picker or config `default_destination`)
auto-binds new threads when the provider advertises a default workspace via the
new `RemoteRunnerProvider::default_workspace` (Blaxel opts in; other providers
are unchanged). Verified live end to end against a real Blaxel account: TUI
shell/file tools execute inside an Alpine sandbox, and pause/resume/detach/rejoin
work through the CLI.

## 0.1.6 (2026-06-26)

### Features

#### Agent-swarm mode

Add a Roder-native `agent_swarm` fanout tool and `/agent-swarm` (alias `/swarm`)
commands (roadmap phase 104). A lead model can launch many homogeneous
subagent tasks from one `prompt_template` (with the `{{item}}` placeholder) over
an `items` array, optionally resuming existing agents via `resume_agent_ids`,
and receives an ordered `<agent_swarm_result>` summary with completed/failed/
aborted counts and resumable agent ids. A bounded scheduler paces launches
(initial burst then one per interval), honors an optional concurrency cap,
preserves input order, and supports cooperative cancellation. Configure via
`[agent_swarm]` or `RODER_AGENT_SWARM_*` env. The `/agent-swarm on|off|status`
command toggles a persistent swarm reminder; `/agent-swarm <prompt>` runs one
swarm task.

### Fixes

#### Server-side agent-swarm mode

Move agent-swarm mode from TUI-only client state to runtime/app-server state so
every client benefits (roadmap 104). Adds the `thread/set_agent_swarm_mode`
app-server method, an `agentSwarmMode` field on `settings/get`, and an
`AgentSwarmModeChanged` event. When swarm mode is active the runtime injects the
canonical swarm reminder into each turn's developer instructions
(`Runtime::set_agent_swarm_mode` + `apply_agent_swarm_mode`), so the model is
nudged toward the `agent_swarm` fanout tool regardless of which client drove the
turn. The TUI now toggles swarm mode through the method and no longer prepends
the reminder client-side. Also fixes two pre-existing method-manifest ordering
issues (`auth/kimi-code/*`, `thread/compact`).

## 0.1.5 (2026-06-26)

### Fixes

#### Fix needing a second Ctrl+C to exit a live Claude Code session

Interrupt any in-flight turn during TUI teardown and exit the process cleanly
once the terminal is restored, so a single Ctrl+C fully exits even when an
in-process provider (e.g. Claude Code) has spawned a CLI subprocess whose
runtime tasks would otherwise block shutdown.

## 0.1.4 (2026-06-24)

### Features

#### Manage external web-search providers from the TUI

Extend the "Web search provider" settings submenu to list the external provider
router options (firecrawl, tavily, perplexity, parallel, synthetic) alongside
the hosted modes, showing each provider's enabled and API-key-configured status
and the active selection. Selecting an external provider persists
`[web_search] mode = "external"`, the chosen `provider`, and enables that
provider's sub-section in user config (applies on restart).

The same hosted modes and external providers are also selectable from the
`Ctrl+P` command palette, and both the menu and palette fall back to reading
providers directly from user config when app-server settings are unavailable.

`settings/get` and `settings/set_web_search` now report the external-router
snapshot via `WebSearchSettings { external_enabled, external_provider, providers }`
and accept an optional `external_provider` selection. New `roder-config`
helpers (`save_web_search_external_provider`, `WebSearchConfig::provider_configured`,
`WebSearchConfig::router_snapshot`, `web_search_router_snapshot`) back the
persistence, status checks, and config-only fallback.

Synthetic web search now auto-configures from the synthetic inference provider:
because both share `SYNTHETIC_API_KEY`, pasting the synthetic provider key
(`providers/configure`) makes the synthetic search provider report as
`key configured`, enables its `[web_search.synthetic]` sub-section, and lets it
resolve the borrowed key at runtime — no separate web-search key entry needed.

## 0.1.3 (2026-06-22)

### Features

#### Add first-party Synthetic inference provider

Adds the `synthetic` provider using Synthetic's OpenAI-compatible Chat
Completions API. The provider ships built-in `syn:` model aliases
(`syn:large:text` default, plus `syn:small:text`, `syn:large:vision`,
`syn:small:vision`), preserves concrete `hf:{owner}/{model}` ids across config,
discovery, and selection, and resolves credentials only from
`SYNTHETIC_API_KEY`/`RODER_SYNTHETIC_API_KEY` or `[providers.synthetic]`. The
provider is visible without credentials so app-server and TUI can show setup
state, and turn-time inference fails locally with setup guidance when the key
is missing. The TUI provider menu points to the Synthetic dashboard for API-key
setup instead of the generic fallback URL.

### Fixes

- Stabilize Roder startup, streaming responses, and provider behavior

## 0.1.2 (2026-06-16)

### Features

#### Fireworks AI inference provider

Add the first-party `fireworks` inference provider with account-scoped model ids, Fireworks-specific API-key configuration, OpenAI-compatible Responses transport, offline model metadata, model discovery, and app-server provider-list coverage.

### Fixes

#### Added first-class `kimi-code` (aliases: `kimi`, `moonshot`) inference provider and `roder-ext-kimi-code` crate.

- Kimi Code subscription OAuth uses the managed API (`api.kimi.com/coding/v1`) with Kimi device headers and `kimi-code-cli` User-Agent; API keys still use Moonshot Open Platform (`api.moonshot.ai/v1`).
- Catalog entry + `kimi-for-coding` model (K2.7 Code).
- Device OAuth against `auth.kimi.com` with `roder auth login kimi-code`, TUI/app-server `auth/kimi-code/*`, and token storage under `~/.roder/auth/kimi-code.json`.
- API key fallback via env/config (`KIMI_CODE_API_KEY`, `RODER_KIMI_CODE_API_KEY`).
- Registered via extension host (always available, like SuperGrok).
- Docs: `docs/roder-kimi-code-provider.md`.
- Live smoke test added (opt-in via `RODER_KIMI_CODE_LIVE=1`).

## 0.1.1 (2026-06-15)

### Features

#### First-party image generation providers (OpenAI GPT Image and Google Gemini Nano Banana)

Provider-neutral image generation through the core media API: an image-capable
`MediaGenerationRequest`/multi-output `MediaGenerationResponse` contract, a new
`ProvidedService::MediaGenerator` extension service, a runtime media generation
service backing the canonical `media_generate_image` tool with a deterministic
offline fallback, new `roder-ext-openai-images` (`gpt-image-2` plus legacy ids)
and `roder-ext-google-images` (Nano Banana 2/Pro/base) provider crates,
`[media.image_generation]` config, `media/image/providers/list` and
`media/image/generate` app-server methods, `roder media` CLI commands, palette
entries, and regenerated schemas/SDK stubs. Live provider smokes stay opt-in
behind `RODER_OPENAI_IMAGE_LIVE` / `RODER_GEMINI_IMAGE_LIVE`.

#### One-command Roder package install (`roder install npm:/git:/path`)

Roder packages bundle process extensions, skills, slash commands, and themes
behind a root `roder.toml` manifest. Install from npm, git (shorthand, SSH,
raw URLs, pinned refs), or local paths; manage with `roder packages
list|resources|enable|disable|approve|filter|sync|init`, `roder remove`,
`roder update`, and ephemeral `-e` loading. Resources surface through the
existing skills/commands/theme registries; the process-extension protocol
gains manifest-declared tool providers served over `tools/call`. New
app-server `packages/*` methods, a `/packages` builtin, and a Packages
palette section round out the surfaces. npm lifecycle scripts stay disabled
unless `--allow-scripts` is passed, and package process extensions never
launch before explicit approval.

### Fixes

- Add a stdio Agent Client Protocol v1 adapter backed by the Roder app-server runtime.

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
