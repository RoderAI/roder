---
roder-ext-jev: patch
---

# Answer one-option choices locally and recover from shed Jev requests

A hosted Jev decision no longer sends a target choice with a single option: it is answered locally (that option, probability 1). A batch the service answers with a 5xx twice in a row is asked one question at a time within the retries it had left, and a large step that is still refused is asked again without its off-screen targets, then paged smaller, instead of ending the run `unavailable`. The decision's `usage.context_budget.shed` counts these retries.
