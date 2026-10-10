---
roder-ext-browser-use: minor
---

# browser_use reports honestly, shows a compact page state, and refuses native selects

- Action results put the action report first, labelled as a claim by the browser agent, before the observed page state, so a size cut never drops it. `browser_use_agent` returns only its own labelled report, with no follow-up state from a different browser.
- Stale element indexes and other known dead-end replies now count as tool errors instead of successes.
- A call that fails after it reached the server (a transport error, a timeout, the server dying, a failed observation) says that the browser and its logins are gone and that the next call starts a fresh browser, and the first successful result from the replacement browser says so. A JSON-RPC error reply to the action itself, a stale element index and a native-select refusal keep the browser and say nothing of the kind.
- `browser_use_get_state` and the state returned after navigation and actions render as one compact line per element within a 150-line / 18,000-character budget. The view ends with an exact omitted count and the next offset. `browser_use_get_state` has a new wrapper-side `offset` parameter to read further. Any state shape the wrapper does not recognise falls back to the raw state.
- `browser_use_click` and `browser_use_type` refuse an index that the last page state showed as a native `<select>`, because the pinned browser-use ignores the click yet reports success, and typing clears the select without a reliable pick. The refusal is an error that names the working routes, and nothing is sent to the browser.
