---
roder-ext-jev: minor
---

# jev_browse handoff statuses are outcomes, not tool failures

A first `needs_input`, `needs_confirmation` or `access_denied` result from `jev_browse` is no longer a failed tool call (`is_error` false, `data.outcome_class` "handoff"). An identical repeat on the same page is an error again (`repeated_handoff`). Other statuses and the result text are unchanged.
