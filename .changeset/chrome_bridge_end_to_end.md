---
roder-protocol: minor
roder-app-server: minor
roder-ext-chrome: minor
---

# Make the Chrome browser bridge usable end to end

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
