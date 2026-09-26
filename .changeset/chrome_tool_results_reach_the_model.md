---
roder-ext-chrome: minor
---

# Return browser content to the model, not just "ok"

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
