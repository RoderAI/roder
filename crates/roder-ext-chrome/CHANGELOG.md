## 0.2.2 (2026-10-01)

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

#### Native OpenAI Responses computer use

Register the native `computer` tool for an explicitly bound CDP browser, execute
ordered action batches with retained per-thread sessions and cancellation
cleanup, and return original-detail `computer_screenshot` observations through
`computer_call_output`. Preserve native call identities through transcript
replay, WebSocket continuation, and ACP tool updates. Include an independent
real-browser protocol eval and a live OpenAI eval runner.

Accept the native API's nullable mouse modifiers. Initialize TLS and large
worker stacks in the live runner, preserve call execution order in its trace,
and independently verify the final browser UI and masked screenshot.

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

#### Allow the native computer eval to run in a visible Chrome window, navigate from

a demo start page to the form, and keep its final page open for inspection.

## 0.2.1 (2026-09-29)

### Features

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

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

## 0.1.2 (2026-09-26)

### Features

#### Make the Chrome browser bridge usable end to end

Everything the browser extension can do is now reachable from a JSON-RPC client
and from the model, and the host's permission mode actually reaches the browser.

App-server methods: `chrome/tabs/open`, `chrome/tabs/close`, `chrome/tabs/group`,
`chrome/page/getText`, `chrome/debug/attach`, `chrome/debug/detach`,
`chrome/recording/start`, `chrome/recording/stop`.

Model tools: `chrome_tabs_group`, `chrome_page_text`, `chrome_highlight`,
`chrome_select`, `chrome_debug_attach`, `chrome_debug_detach`.

`chrome/enable` and `chrome/setMode` now push the mode to the extension as a
`session/mode` command. Previously the host recorded the mode locally while the
extension stayed in its own mode, so `chrome/setMode { control }` left every
click, keystroke and navigation refused by the browser side. The mode can only
narrow what runs; the extension's user-set capability ceiling and its per-origin
site permissions are unchanged.

`chrome_page_snapshot`'s `include` schema advertised a non-existent `aria`
section and omitted `text` and `controls`, so a model following the schema got a
snapshot with no page text and no interactive elements. It now matches the
extension: `text`, `controls`, `forms`, `iframes`, `boxes`.

Also restores the sorted order of the method manifest, which
`thread/set_ultra_mode` had broken.

#### Return browser content to the model, not just "ok"

Every `chrome_*` tool set its `ToolResult::text` to a fixed
`"chrome <kind> ok"` and put the real payload in `data`. The runtime feeds
`text` to the model and keeps `data` for the UI, so the model never saw a tab
list, a page snapshot, page text, console output, or network metadata — asked to
read an element it would either report a plausible-looking wrong value or
complain that "the chrome tools keep returning just ok". Both were observed
end-to-end against a real browser.

Tool text now carries the result, keeping the untrusted-content note in front of
page-derived payloads. A screenshot is summarized instead of inlining megabytes
of base64, and an oversized result is truncated with guidance to narrow the
request.

`chrome_page_text` also takes an optional `selector`, `ref`, or `text` target.
Whole-page text is a single flattened blob with no element boundaries, so
"what does #out say?" was unanswerable; it now reads just that element.

## 0.1.1 (2026-06-15)

### Fixes

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
