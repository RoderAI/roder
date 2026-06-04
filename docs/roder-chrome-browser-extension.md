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

## Enabling for a session

Pairing connects the extension; enabling turns the `chrome_*` tools on for the
agent. They are **off by default**.

- In the TUI: run `/chrome` to open the control panel (also reachable via the
  ctrl+p command palette as "Chrome browser plugin"). Enable from there.
- From the CLI: `roder --chrome`, or `roder chrome enable`.
- Check state: `roder chrome status` (or the `/chrome` panel) shows whether an
  extension is connected, the active tab, and the current mode.

If a `chrome_*` tool reports "Chrome is not enabled" or "No Chrome extension is
connected", that is the bridge telling you pairing/enabling is incomplete.

## Parity matrix vs Claude-in-Chrome

Priority: **P0** = core, must work; **P1** = important; **P2** = nice-to-have.
"Implemented" = wired end-to-end; "Stub" = present but returns a clear
not-supported error; "Not yet" = no surface yet.

| Capability                         | Prio | Roder tool / method                                   | Status        | Notes |
|------------------------------------|------|--------------------------------------------------------|---------------|-------|
| List tabs                          | P0   | `chrome_tabs_list` / `chrome/tabs/list`                | Implemented   | id, title, url, active |
| Open tab                           | P0   | `chrome_tab_open`                                      | Implemented   | http(s) only |
| Activate tab                       | P0   | `chrome_tab_activate` / `chrome/tabs/activate`         | Implemented   | |
| Close tab                          | P1   | `chrome_tab_close`                                     | Implemented   | |
| Navigate                           | P0   | `chrome_navigate` / `chrome/tabs/navigate`            | Implemented   | protected: control mode + approval |
| DOM snapshot (aria/forms/boxes)    | P0   | `chrome_page_snapshot` / `chrome/page/snapshot`        | Implemented   | aria roles, form metadata, bounding boxes, iframes; `untrusted:true` |
| Page text                          | P1   | (via snapshot `include:["text"]`)                      | Implemented   | |
| Screenshot                         | P0   | `chrome_screenshot`                                    | Implemented   | full visible-tab PNG data URL; **region crop not supported in MV3 SW** |
| Click                              | P0   | `chrome_click` / `chrome/page/action`                  | Implemented   | by selector, visible text, or snapshot ref |
| Type                               | P0   | `chrome_type`                                          | Implemented   | optional submit |
| Keypress                           | P1   | `chrome_keypress`                                      | Implemented   | |
| Scroll                             | P1   | `chrome_scroll`                                        | Implemented   | |
| Select option                      | P2   | `chrome/page/action` (`page/select`)                   | Implemented   | no dedicated model tool yet |
| Highlight element                  | P2   | `chrome/page/action` (`page/highlight`)                | Implemented   | inspection aid |
| Console read                       | P0   | `chrome_console_read` / `chrome/debug/console`         | Implemented   | CDP, redacted, bounded; needs debugger site perm |
| Network read                       | P0   | `chrome_network_read` / `chrome/debug/network`         | Implemented   | metadata only, no bodies/headers; redacted URLs |
| Evaluate JS                        | P1   | `chrome_eval`                                          | Implemented   | protected: control mode + eval site perm |
| Recording (action trace)           | P1   | `chrome_recording_start` / `chrome_recording_stop`     | Implemented   | JSON action trace |
| Per-origin permissions             | P0   | `chrome/permissions/list` / `chrome/permissions/update`| Implemented   | inspect/interact/eval/debugger/download/upload/recording/schedule/alwaysAllow |
| File upload                        | P2   | `page/upload`                                          | **Stub**      | MV3 cannot synthesize a file chooser; asks user to attach via page UI |
| GIF / video capture                | P2   | —                                                      | **Not yet**   | only single-frame screenshots today |
| Native messaging                   | P2   | —                                                      | **Not yet**   | pairing is WebSocket-only |
| Scheduling                         | P2   | `schedule` site-permission flag exists                 | **Not yet**   | permission flag reserved; no scheduler wired |

## Permission and security model

Two independent gates must both pass for a privileged action: the **session
mode** and the **per-origin site permission**.

### Session modes

`chrome/setMode` selects one of:

- **observe** — chat, tab status, connection state only.
- **assist** (default) — inspect actions run when the site permits; privileged
  actions queue for user approval.
- **control** — enabled actions execute within the approved plan and site
  scope; protected actions still require explicit approval.

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
