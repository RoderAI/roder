# Roder Chrome Browser Extension

Roder can drive a user's real, logged-in Chrome session through a Manifest V3
(MV3) browser extension. This lets the model inspect live pages, read console
and network activity, interact with the DOM, and record action traces — inside
the browser the user already trusts, without copying credentials out of it.

This document covers the architecture, install/pairing, enabling, a parity
matrix against Claude-in-Chrome, the permission/security model, a privacy
checklist, and troubleshooting.

> Status note: this integration is new. Some capabilities are fully wired
> end-to-end, others are intentional stubs that fail with a clear message. The
> parity matrix below is explicit about which is which. Nothing here overstates
> what ships today.

## Architecture

There are four layers, connected in a single chain:

```text
  MV3 extension                remote WebSocket          app-server            model
  (roder-web-extention)  <-->  bridge (remote.rs)  <-->  chrome/* methods <-->  chrome_* tools
                                                                            +--  TUI panel / CLI
```

1. **MV3 extension** (`/Users/pz/w/roder-web-extention`, v0.2.0). TypeScript +
   React. A service worker holds the WebSocket connection; content scripts do
   DOM snapshots and actions; the side panel/popup/options pages show state and
   pairing. It speaks the JSON wire envelope below.
2. **Remote WebSocket bridge.** The Roder app-server's remote transport
   (`roder-app-server/src/remote.rs`) accepts the extension as a client over the
   `roder.remote.v1` + `bearer.<token>` subprotocols and registers it with the
   process-global `ChromeBridge` (`roder-api/src/chrome.rs`).
3. **`chrome/*` app-server methods.** Runtime methods (`chrome/status`,
   `chrome/enable`, `chrome/tabs/list`, `chrome/page/snapshot`,
   `chrome/page/action`, `chrome/debug/console`, `chrome/debug/network`,
   `chrome/permissions/*`, …) dispatch commands to the connected extension and
   surface status to clients.
4. **Model `chrome_*` tools + TUI/CLI.** The `roder-ext-chrome` crate registers
   policy-gated, model-facing tools (`chrome_tabs_list`, `chrome_page_snapshot`,
   `chrome_click`, `chrome_console_read`, …). The TUI exposes a control panel
   (`/chrome` slash command, plus a ctrl+p palette entry) and the CLI exposes
   `roder --chrome` / `roder chrome status|enable|disable|reconnect`.

### Computer-use primitive repairs in the audit branches

The paired extension changes are local to branch `pz/computer-use-primitives` in
`/Users/pz/.codex/worktrees/browser-extension-audit/roder-web-extention`; they
must be installed together with the Roder audit branch. They are not a claim
that the existing installed extension has been updated.

Click, type, keypress and wheel input use tab-targeted `chrome.debugger` CDP
commands. Type verifies an editable target and actual focus first. Select is a
semantic DOM operation: it checks the requested value and explicitly reports
`eventsTrusted: false`, since macOS native select popups did not respond to
CDP keys in the evaluation. Missing, covered and ambiguous targets fail.
References live in the content script's isolated world, with a unique document
identity; page attributes cannot forge them.

Actions return a fresh, untrusted page observation. Queued actions preserve the
concrete tab, origin and document from the permission gate, then recheck current
settings and site permission before input. Screenshots target that tab through
CDP, support validated viewport crops and preserve CSS pixel coordinate mapping.
Filled sensitive fields are covered during capture; masks are removed on errors
and cancellation. Masking is heuristic, not an exhaustive secret detector.

A dropped or timed-out Rust dispatch sends
`{"type":"command/cancel","targetId":"<corr>"}` to the same extension client.
The extension aborts running input or removes a pending approval, releases held
input and removes masks. Disconnection cancels its pending/running commands.
Recovery remains best effort if Chrome is unreachable or the extension exits.
The connected Chrome profile remains the user's selected profile; use an
isolated agent profile when isolation is required.

### Wire envelope

The extension and app-server exchange JSON frames:

- Roder -> extension (command): `{ "type": "<command>", "id": "<corr>", ...params }`
- extension -> Roder (result): `{ "type": "command/result", "id": "<corr>", "ok": bool, "result"?: any, "error"?: string }`
- extension -> Roder (event): `{ "type": "hello" | "state" | "tabs/list" | "page/snapshot" | "debug/console" | "debug/network" | "activity" | ... }`

The extension also accepts a JSON-RPC-like shape where `method` maps to the
command `type` and `params` are spread into the frame.

All browser-origin payloads (DOM text, controls, console lines, network
metadata) carry `untrusted: true`. The model layer treats them as **data, never
instructions**.

### What the model reads after a `chrome_*` call

The extension answers in JSON; the model does not read the JSON. A snapshot, a
navigation and every page action (`chrome_click`, `chrome_type`,
`chrome_keypress`, `chrome_scroll`, `chrome_select`) are read into one page
model and rendered as lines (`roder-ext-chrome`, `extension_result.rs`,
`observed.rs`, `observed_render.rs`). Other results (tab lists, console,
network, eval) stay labelled JSON, cut at 24,000 characters.

```text
<the untrusted-content label>
Outcome: 1 control changed; page text changed.        (page actions only)
Action result: {"ref":"c2"}                            (what the action itself answered)
Page: https://app.example/cart
Title: Cart
Viewport 1280x800 px, scrolled to 0.
Controls (ref, role, "label"; act by ref, or for a select by the selector shown):
c1 button "Add"
c2 checkbox "Agree" [checked] [changed]
c3 link "Checkout" [new]
Forms:  ...
Frames: ...
Text:
  <the page text, on one line>
```

- **The outcome sentence** is what the action did to the page, from comparing
  the page it shows with the last page seen on the same tab (the tab the
  extension says it answered for, else the `tabId` the call named, else the
  active tab; the last 16 tabs are kept). A call that named no tab is about the
  active tab and is compared with the last page read without a tab named, unless
  the extension says that was another tab's. A call that names a tab is compared
  only with a page of that tab, never with a page of no known tab, which may have
  been another's; it says so (`No earlier observation ...`) rather than claim a
  navigation. A page the extension itself reads, a snapshot, carries no tab, so
  the tab is only known from the call.
  `No visible change.`; `URL <a> -> <b>.` (a change of the hash counts, and
  `Title now "..."` follows when the title moved too); otherwise the parts
  that apply, joined: `N controls changed` (added, removed or showing something
  else: label, value, checked, expanded, disabled; where a control sits or its
  selector does not count), `page text changed`, `title now "..."`, `scrolled
  to y=N`. The first action on a tab has nothing to compare with and says so
  (`No earlier observation of this tab to compare with.`). Controls added since
  the last page are marked `[new]`, changed ones `[changed]`. A navigation or a
  snapshot has no outcome line: it is a page, not the effect of an action.
- **A snapshot of some sections** (`include`) says which it did not read: the
  extension answers a section it was not asked for as empty, which is not the
  page having none, so the result says `Controls: not requested.` or `Text: not
  requested.` instead of `Controls: none found.` and never reports a section it
  did not read as empty. Such a snapshot does not replace the fuller page kept
  for the tab: its unread sections are filled in from the previous page when that
  was the same address, so the next action is not told that every control is new
  or the text changed. A section only one of the two pages read is not compared,
  and the sentence says so (`No visible change in what was compared; controls not
  read both times.`).
- **Controls before text, cut text first.** Controls come before the page
  text. A result stays within 18,000 characters and 150 lines, a margin inside
  the runtime's own cut of a tool result (20,000 characters, 200 lines), which
  keeps only the two ends of what it cuts and so could take the outcome line
  away. When the result would pass either, the text is cut first
  (`… text cut at N chars (the page text is M chars)`, `at least M` when the
  extension itself had cut its 12,000-character text), then forms and frames
  (each keeps its header and says `… N more forms not listed`), and the
  controls last (`… N more controls not listed`). The outcome line is always
  the first line after the label. A page of many short controls is cut by
  lines, not characters: about 140 control lines fit. While any control
  is left out the text is withheld whole and says so. Boxes are neither asked
  for (a `chrome_page_snapshot` without `include` asks for text, controls,
  forms and iframes) nor shown; act by ref. A `<select>` line carries its
  selector, since `chrome_select` on the extension is pointed at it by
  selector.
- **Page words** (labels, values, titles, addresses, text) are put on one line,
  stripped of control and direction-overriding characters and cut, and the
  text is indented, so none of it can start a line of the result's own. A
  password field is listed as a secret field with no value.
- **Builds that do not observe.** A build with `action-observation` answers
  an action with `{action, observation, tabId}`. The earlier builds answer with
  only what they did (`{ok, ref}`; on master a click is a timer that fires 150
  ms later). For those, Roder waits 150 ms and sends one `page/snapshot` (the
  same `tabId`; no boxes) and renders that as the observation. The wait starts
  when the answer arrives, so the snapshot reaches the page just after that
  click and a page that reacts later (a request, a submit that navigates) can
  be read mid-change. It asks once:
  if that snapshot fails (a revoked site permission, say) the result says the
  action ran but the page could not be read, with the reason, and points at
  `chrome_page_snapshot`. Which build answered is in the result's data, never in
  the text: `observation.build` is `action-observation` or `legacy`,
  `observation.source` is `action_result`, `snapshot_fallback` or
  `unavailable` (with `observation.error`), and `outcome` carries the sentence
  and its parts. The fallback snapshot is kept under `observed`, the action's
  own answer under `content`.

On Roder Desktop's browser the same sentence leads the result of `chrome_click`,
`chrome_type`, `chrome_keypress`, `chrome_scroll` and `chrome_select` (the page
is read once, briefly, before the input, and compared with the page after it
by the same function), so one page changing one way reads the same whichever
browser showed it. A dialog the action answered, or a tab it opened, is added
to the sentence. `crates/roder-ext-chrome/tests/computer_use.rs` runs one
scripted page through the Desktop browser and through a scripted extension and
requires the same sentence for a dead button, added text, a toggled checkbox
and a changed hash.

## Direct CDP tools (no extension)

`roder_ext_chrome::direct` is Roder's own CDP toolset for one tab, used where
no extension is involved:

- **Roder Desktop's integrated browser.** When no extension is connected, the
  `chrome_*` tools fall back to the integrated browser's DevTools port
  (`RODER_DESKTOP_CDP_PORT`, default 9334) through the same client; the
  screenshot tool goes through the toolset's `screenshot`. `chrome_select` goes
  through the toolset's `select` (see below). `chrome_page_text` and
  `chrome_highlight` have no Desktop route: with the Desktop browser reachable
  they answer "not supported on the Roder Desktop browser" (and point at
  `chrome_page_snapshot` / the action tools) instead of claiming that no
  browser is connected. The other extension-only tools (`chrome_tab_activate`,
  `chrome_tab_close`, `chrome_tabs_group`, the debugger, console, network and
  recording tools) still answer "not connected" on Desktop.
- **Jev's tab.** `roder-ext-jev` binds the whole set to its session's tab as
  the `jev_tab_*` tools, and runs its automatic fallback on it (see
  [`docs/jev-browser.md`](jev-browser.md), "When Jev cannot progress").

A `DirectSession` attaches to a `DirectTab`: a target of a browser's DevTools
endpoint (an http(s) address whose `/json/version` names the browser
websocket, or that websocket), attached as a flat session, or a page
websocket opened directly. It enables the Page domain and answers
JavaScript dialogs as Jev does (alerts and `beforeunload` accepted, confirms
and prompts dismissed, each reported), and turns on focus emulation so a
background tab keeps rendering. Its tools:

| Tool | What it does |
| --- | --- |
| `look` | address, title, HTTP status, the elements to act on (refs `e1`, `e2`, … with boxes in viewport px; a ref names the same element for as long as it is in the document), the page text (cut at 3,000 characters, 1,200 after an action, and so that a look is at most 140 lines; a cut ends with `… text cut at N chars (the page text is M chars)`); untrusted |
| `screenshot` | a JPEG of the viewport, one image pixel per CSS px; filled secret fields blacked out, withheld while a typed secret shows |
| `click` | real mouse events at a ref (hit-tested: a covered ref is not pressed, and the result names what covers it) or at x/y; right, middle, double |
| `hover` | the pointer moved onto a ref or x/y |
| `drag` | press, move in steps, release, between refs or points |
| `type` | text inserted into a ref (clicked first, content replaced) or the focused field; `submit` presses Enter; a secret field's text is reported as `[secret]` |
| `key` | any key or chord (`Escape`, `Shift+Tab`, `Control+a`, a character), repeatable; Enter and Space press what has focus, so they are refused for a control something covers, and the result names the cover like a refused click does (page text: scrubbed, on one line, without its own quote marks, cut at 60 characters) |
| `scroll` | the mouse wheel at a ref, x/y or the middle of the page |
| `select` | a native select's option by value, else by visible text (see below) |
| `navigate` | an http(s) URL in the same tab, or back, forward, reload |
| `wait` | up to 10 s |

`select` takes a ref and an option. An enabled option whose value is the given
text wins; otherwise the option whose visible text is (ignoring case and
spacing), then one whose value is (ignoring case), then the only option whose
text contains it. More than one distinct candidate at a step is ambiguous and
nothing is chosen; a disabled option, a disabled select and something that is
not a native `<select>` are refused by name. A miss lists the options (text,
and value where it differs; disabled ones marked; at most 20, with the rest
counted, each cut and scrubbed, and marked as untrusted page text) and changes
nothing. The choice is made in the page script (`input` and `change` events,
not trusted ones, which the data says: `events_trusted: false`); once the page
has settled the select is read again, and a page that put the old value back
(at once or a moment later) is reported as "changed it back ... did not stick"
with `is_error` set, not as a success.

On Desktop, `chrome_select {tabId?, ref?, selector?, value}` (required:
`value`, and exactly one of `ref` and `selector`; anything else is an error
before any tab is touched) maps onto it: a selector is resolved to a ref
through the same resolver as `chrome_click` (one visible match, or an error),
and `value` becomes the option. With the paired extension the wire command is
unchanged, `{type: "page/select", tabId?, selector, value}` and the extension
still needs the option's exact value; a `ref` is refused before it is sent,
since the extension reads a selector only.

Each `chrome_*` input on the Desktop browser leads with the same `Outcome:`
sentence the extension's results do (see above), computed from a brief look
before the input and the look after it. The untrusted-content label is the first
line and the outcome the second, as on the extension path, since the sentence can
carry the page's title and address.

After every action it waits for the page (a load it started, then 250 ms
without a DOM change, at most 2 s), follows a tab the action opened, and reads
the page again briefly, so the result shows what changed. The owner's
`DirectGuard` is asked before a click on a control or an Enter (a reason stops
it undispatched), and after every action about the page's origin and whether
it refused automated access (either stops the run). There is no script
evaluation tool in the set. `devtools` holds what every DevTools client in
Roder shares: the browser websocket lookup, message decoding (lone surrogates
read as U+FFFD) and the dialog rule; Jev's own connection uses it.

## Install the unpacked extension

```bash
pnpm --dir /Users/pz/w/roder-web-extention install
pnpm --dir /Users/pz/w/roder-web-extention build
```

This produces `dist/`. Then in Chrome:

1. Open `chrome://extensions`.
2. Toggle **Developer mode** on (top-right).
3. Click **Load unpacked** and select
   `/Users/pz/w/roder-web-extention/dist`.

Chrome 116+ is required (MV3 side panel + debugger APIs).

## Pairing

The extension starts **disconnected**. To pair it with Roder:

1. Start the remote app-server, or open the remote/Chrome panel:

   ```bash
   roder app-server --remote --listen ws://127.0.0.1:0
   # or in the TUI:  /remote start   (or /chrome to open the Chrome panel)
   ```

2. Copy the printed **WebSocket URL** and **bearer token**.
3. Open the extension **options page** and paste the URL + token, then connect.

The extension authenticates with the WebSocket subprotocols:

```text
roder.remote.v1, bearer.<token>
```

Prefer `ws://127.0.0.1` or a trusted private network / Tailscale endpoint. Do
not expose Roder remote mode to the public internet.

## Availability for a session

The `chrome_*` tools are available to the agent by default. Pairing connects a
real Chrome extension client; without a paired extension the tools return a
clear "No Chrome extension is connected" error instead of prompting for a
separate enable step.

- In the TUI: run `/chrome` to open the control panel (also reachable via the
  ctrl+p command palette as "Chrome browser plugin") for pairing status and
  mode controls.
- From the CLI: `roder chrome status`; `roder chrome enable|disable` remain
  available for manual override.
- Check state: `roder chrome status` (or the `/chrome` panel) shows whether an
  extension is connected, the active tab, and the current mode.

If a `chrome_*` tool reports "Chrome tools are not enabled", they were disabled
manually with `roder chrome disable` or the equivalent app-server call. If it
reports "No Chrome extension is connected", pairing is incomplete.

## Parity matrix vs Claude-in-Chrome

Priority: **P0** = core, must work; **P1** = important; **P2** = nice-to-have.
"Implemented" = wired end-to-end; "Stub" = present but returns a clear
not-supported error; "Not yet" = no surface yet.

| Capability                         | Prio | Roder tool / method                                   | Status        | Notes |
|------------------------------------|------|--------------------------------------------------------|---------------|-------|
| List tabs                          | P0   | `chrome_tabs_list` / `chrome/tabs/list`                | Implemented   | id, title, url, active |
| Open tab                           | P0   | `chrome_tab_open` / `chrome/tabs/open`                 | Implemented   | http(s) only; joins the `Roder` tab group |
| Activate tab                       | P0   | `chrome_tab_activate` / `chrome/tabs/activate`         | Implemented   | |
| Close tab                          | P1   | `chrome_tab_close` / `chrome/tabs/close`               | Implemented   | |
| Tab group                          | P1   | `chrome_tabs_group` / `chrome/tabs/group`              | Implemented   | one reusable orange `Roder` group per window |
| Navigate                           | P0   | `chrome_navigate` / `chrome/tabs/navigate`            | Implemented   | protected: control mode + approval |
| DOM snapshot (aria/forms)          | P0   | `chrome_page_snapshot` / `chrome/page/snapshot`        | Implemented   | the tool renders controls (ref, role, label, value, state) before the text, then forms and iframes, within 18,000 characters and 150 lines; boxes are not asked for or shown (the `chrome/page/snapshot` method is unchanged); `untrusted:true` |
| Page text                          | P1   | `chrome_page_text` / `chrome/page/getText`             | Implemented   | optional `selector`/`ref`/`text` reads one element; also via snapshot `include:["text"]`; extension only (Desktop browser: explicit "not supported", use `chrome_page_snapshot`) |
| Screenshot                         | P0   | `chrome_screenshot`                                    | Implemented   | full visible-tab PNG data URL; **region crop not supported in MV3 SW** |
| Click                              | P0   | `chrome_click` / `chrome/page/action`                  | Implemented   | by selector, visible text, or snapshot ref |
| Type                               | P0   | `chrome_type`                                          | Implemented   | optional submit |
| Keypress                           | P1   | `chrome_keypress`                                      | Implemented   | |
| Scroll                             | P1   | `chrome_scroll`                                        | Implemented   | |
| Select option                      | P2   | `chrome_select` / `chrome/page/action` (`page/select`) | Implemented   | `ref` or `selector`, plus `value`. Extension: selector and exact value. Desktop browser: ref or selector; value, then visible text; a miss lists the options |
| Highlight element                  | P2   | `chrome_highlight` / `chrome/page/action`              | Implemented   | inspection aid; extension only (Desktop browser: explicit "not supported") |
| Debugger attach/detach             | P0   | `chrome_debug_attach` / `chrome/debug/attach`          | Implemented   | required before console/network reads return anything |
| Console read                       | P0   | `chrome_console_read` / `chrome/debug/console`         | Implemented   | CDP, redacted, bounded; needs debugger site perm + attach |
| Network read                       | P0   | `chrome_network_read` / `chrome/debug/network`         | Implemented   | metadata only, no bodies/headers; redacted URLs; needs attach |
| Evaluate JS                        | P1   | `chrome_eval`                                          | Implemented   | protected: control mode + eval site perm |
| Recording (action trace)           | P1   | `chrome_recording_start` / `chrome/recording/start`    | Implemented   | JSON action trace |
| Per-origin permissions             | P0   | `chrome/permissions/list` / `chrome/permissions/update`| Implemented   | inspect/interact/eval/debugger/download/upload/recording/schedule/alwaysAllow |
| File upload                        | P2   | `page/upload`                                          | **Stub**      | MV3 cannot synthesize a file chooser; asks user to attach via page UI |
| GIF / video capture                | P2   | —                                                      | **Not yet**   | only single-frame screenshots today |
| Native messaging                   | P2   | —                                                      | **Not yet**   | pairing is WebSocket-only |
| Scheduling                         | P2   | `schedule` site-permission flag exists                 | **Not yet**   | permission flag reserved; no scheduler wired |

## Permission and security model

Two independent gates must both pass for a privileged action: the **session
mode** and the **per-origin site permission**.

### Pairing persists

Pair once and every later Roder run reconnects on its own. The bearer token and
the loopback port the listener bound are stored in
`<config-dir>/remote-pairing.json` (owner-only, never logged), and a Roder that
finds that file brings the listener back up on the same endpoint at startup —
no `/remote start`, no second trip through `/pair`. The extension keeps its own
copy of the endpoint and token and reconnects through its keepalive, so a
browser restart, an extension reload, or an MV3 service-worker shutdown all heal
themselves.

`/remote regenerate` (alias `/remote unpair`) mints a new token, which
invalidates every paired browser and is the way to revoke access. If the
remembered port is already taken — a second Roder is running — the listener
falls back to an ephemeral port, remembers that one, and the browser needs one
more trip through `/pair`.

### Tab group

Every tab Roder opens or is pointed at is collected into a single Chrome tab
group named **Roder** (orange, one per window), so the user can see at a glance
which tabs the agent is driving and collapse or close all of them at once.
`chrome/tabs/group` with no title reuses that group; pass an explicit title to
make a separate, named group. Tabs the user has already grouped themselves are
left where they are.

### Session modes

`chrome/setMode` selects one of:

- **observe** — chat, tab status, connection state only.
- **assist** (default) — inspect actions run when the site permits; privileged
  actions queue for user approval.
- **control** — enabled actions execute within the approved plan and site
  scope; protected actions still require explicit approval.

The mode is pushed to the extension as a `session/mode` command whenever it
changes, so both sides gate on the same value. It can only narrow what runs: the
extension's own capability toggles (options page) and its per-origin site
permissions remain the user's ceiling and are never raised from the wire.
Pairing grants a usable default — inspection, navigation and input, in `assist`
mode so each privileged action waits for approval — while eval, debugger,
downloads, uploads and recording stay off until the user turns them on.

### Action classes

- **Inspect** (`tabs/list`, `page/snapshot`, `debug/console`, `debug/network`)
  — generally allowed.
- **Interact** (`click`, `type`, `keypress`, `scroll`, `select`) — require
  interact permission; in the extension also require control mode + input
  capability.
- **Protected** (`eval`, navigation, downloads, uploads) — require control mode
  **plus** user approval. Denied means stop, not work around.
- **Prohibited** (solving/bypassing CAPTCHAs, handling raw payment-card or
  credential data) — always refused.

### Per-origin site permissions

The extension stores a permission record per origin with flags: `inspect`,
`interact`, `eval`, `debugger`, `download`, `upload`, `recording`, `schedule`,
`alwaysAllow`. Defaults are inspect-only; everything else is opt-in. A grant on
one origin never transfers to another. Chrome internal pages (`chrome://`) and
extension pages are never controlled, and only `http:`/`https:` URLs are
accepted.

## Privacy checklist

The integration runs inside the user's existing session and must never copy
secrets out of it. By construction:

- [ ] **Cookies** are never read or transmitted.
- [ ] **Auth tokens** (Authorization, x-api-key, x-auth-token, x-csrf-token, …
      headers) are stripped from network metadata.
- [ ] **Hidden and password inputs** (and name/id/autocomplete fields hinting at
      secrets: password, otp, cvc/cvv, ssn, card, pin, …) are never captured in
      snapshots.
- [ ] **Request/response bodies and headers** are never captured; network reads
      are method/URL/status/size/timing metadata only.
- [ ] **URLs are stripped** to origin + pathname, dropping query strings and
      fragments that often carry tokens.
- [ ] **Unrelated tabs** are not snapshotted or persisted; the agent works
      against the active/target tab for the task.
- [ ] Nothing above is persisted to disk by Roder as part of normal operation.

## Troubleshooting

| Symptom | Likely cause | Fix |
|---------|--------------|-----|
| **Extension not detected** | Not paired, or wrong URL/token | Re-copy the URL + token from `roder app-server --remote` (or the `/chrome` panel) into the options page; confirm `roder chrome status` shows a client. |
| **Service worker idle** | MV3 service workers are evicted when idle | Open the side panel/popup or trigger any command to wake it; the next command reconnects. Use `roder chrome reconnect` if needed. |
| **Debugger attach blocked** | Another DevTools/debugger is attached, or no `debugger` site permission | Close DevTools for that tab; grant the `debugger` permission for the origin in the extension. Console/network reads require this. |
| **Permission denied** | Action class not allowed by mode, or site permission missing | Switch to the right mode (`chrome/setMode`), approve the queued action, and grant the per-origin flag. Protected actions need control mode + approval. |
| **No active tab** | Target tab closed, or focus on a `chrome://`/extension page | Activate a normal http(s) tab; pass an explicit `tabId`. Internal pages are never controllable. |
| **Connection drops** | Network blip, server restart, or token rotation | The extension auto-reconnects when `autoConnect` is on; otherwise reconnect from options or run `roder chrome reconnect`. Re-pair if the token changed. |

## Related

- Wire contract and bridge: `crates/roder-api/src/chrome.rs`
- Model tools: `crates/roder-ext-chrome/src/tools.rs`
- Built-in skill: `crates/roder-skills/builtin/chrome/SKILL.md`
- Extension source: `/Users/pz/w/roder-web-extention`
- Offline fixtures for host-side tests: `evals/fixtures/chrome/`
