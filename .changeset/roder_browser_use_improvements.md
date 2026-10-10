---
roder: minor
---

# Browser tools report honestly and stall less

Roder's browser features now tell the agent what actually happened and stop it from going in circles.

- **`browser_use`**: the action report comes first and is never cut off, page state is one line per element with an exact omitted count and an `offset`, dead clicks and stale indexes are errors, clicks on a native select are refused with a route that works, and a lost browser says so.
- **`chrome_*`**: results lead with the outcome and stay within a line budget; `chrome_select` works on Roder Desktop; native computer use returns what happened (dialogs, new tabs, HTTP errors), never loses the screenshot, and sends Cmd chords on a Mac page.
- **`jev_browse`**: an unusable decision reply is asked again and then falls back to the frontier model, a delete on a twin row is not repeated at low confidence, runs that go round in circles stop with a named cause, and handoffs are outcomes rather than failed tool calls.
- **Providers that cannot receive tool-result images** no longer get screenshot tools or an "attached" screenshot that never arrives.
- **Webwright** failed runs explain themselves and no longer outlive a timeout.
- A short hint tells the model which browser tool family to use when several are available.
