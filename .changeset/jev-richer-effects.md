---
roder-ext-jev: patch
---

# Richer step effects for Jev, and per-snapshot name and visibility caches

A step's `effect` now names an HTTP status of 400 or more that a navigation
landed on, and the fields its form still needs when an action set nothing
(the snapshot marks required-and-empty and `aria-invalid` fields
`blocking`). A click on an option, checkbox, radio or switch that set no value
says so, and the stall rule counts it as a step that changed nothing; an
autocomplete pick that leaves the typed text as it was counts as set.

The observation reads each element's visibility, name and cursor once per
snapshot, and `context.js` each title's and scope's text once per run;
the caches end with the call, so later scripts read the page as it is.
