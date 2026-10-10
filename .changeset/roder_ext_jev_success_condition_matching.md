---
roder-ext-jev: minor
---

# jev_browse success_condition matches text more reliably and adds text_absent

`jev_browse` `success_condition` gains `text_absent`, compares `text_contains` and `text_absent` ignoring case, and all three ignoring whitespace runs, no-break spaces and zero-width characters (`url_contains` keeps the case of the path, query and fragment, as before), leaves form-field values out of the page text being matched, and reports the unmet predicates (condition met / not met).
