---
roder-ext-jev: minor
---

# jev_browse success_condition matches text more reliably and adds text_absent

`jev_browse` `success_condition` gains `text_absent`, compares `url_contains`, `text_contains` and `text_absent` ignoring case, whitespace runs, no-break spaces and zero-width characters, leaves form-field values out of the page text being matched, and reports the unmet predicates (condition met / not met).
