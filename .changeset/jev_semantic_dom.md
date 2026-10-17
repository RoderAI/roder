---
roder-ext-jev: patch
---

# Expose semantic DOM evidence in Jev browser results

Browser results include a bounded semantic DOM tree with page hierarchy, roles,
control states and observed node references. Secret fields and typed secrets stay
redacted. `JEV_PROMPT_PROFILE=dom` opts into the experimental DOM-grounded Jev
prompt; use `JEV_IMAGES=1` to evaluate it with screenshots. The default prompt
remains Workflow. The paired eval supports image input for every Jev profile.

Candidate rectangles are scaled into the fitted screenshot's pixels when the
viewport image is resized, so visual references stay aligned.

For populated editable fields, `JEV_PROMPT_PROFILE=dom_targets` keeps screenshots
in conditional target questions while selecting operations and evaluating
irreversible actions from DOM evidence. Other pages retain shared visual evidence.

DOM evidence preserves styled toggle states and controls nested in headings, and
redacts JSON-escaped secret echoes. Offscreen DOM updates do not invalidate an
otherwise unchanged page-level decision.
