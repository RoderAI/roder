# Chrome offline fixtures

Static JSON fixtures that mirror the Roder Chrome bridge wire shapes so
host-side tests can exercise the `chrome_*` tools, `chrome/*` app-server
methods, and snapshot/console/network parsing **without a real browser or a
connected extension**.

Every payload matches the wire contract defined in
`crates/roder-api/src/chrome.rs` and the extension's
`src/shared/protocol.ts`:

- camelCase field names,
- the command-result envelope `{ "type": "command/result", "id", "ok", "result"?, "error"? }`,
- `untrusted: true` on every browser-origin payload (DOM, console, network),
- redaction already applied (no cookies, tokens, hidden inputs, request/response
  bodies or headers; URLs are origin + pathname only, query/fragment stripped).

## Files

| File | Shape | What it represents |
|------|-------|--------------------|
| `tabs.json` | `tabs/list` result | A fake tab list (active + background tabs, one `chrome://` internal tab that must never be controlled). |
| `snapshot.json` | `page/snapshot` result | A `PageSnapshot`: title/url/text, `controls` with aria roles + selectors + bounding `box`es, `forms` metadata, a cross-origin `iframes` entry, `viewport`, and `untrusted: true`. |
| `console.json` | `debug/console/read` result | Redacted CDP console `entries` (warning/error/info), metadata only, `untrusted: true`. |
| `network.json` | `debug/network/read` result | Redacted CDP network `entries`: method, stripped URL, status, sizes, timing, a failed request — no bodies/headers, `untrusted: true`. |
| `action-trace.json` | `recording/stop` result | A `Recording` with a sequence of `actions` (click/type/navigate). |
| `permission-prompt.json` | approval scenario | A pending `AgentActivity` for a protected `page/eval`, the requested command, the current per-origin permissions, and the deny result when `eval` is not granted. |

## Notes

- Redaction is illustrative: real entries pass through the extension's
  `redaction.ts` before transmission. `req-1004` in `network.json` shows a
  failed request with only `errorText` (truncated, no body).
- `snapshot.json` deliberately contains **no** password/hidden inputs or values;
  those are dropped at capture time and must never appear in a fixture.
- IDs and timestamps are stable constants so assertions can match them exactly.
