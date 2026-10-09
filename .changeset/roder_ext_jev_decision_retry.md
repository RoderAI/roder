---
roder-ext-jev: minor
---

# Jev retries an unusable decision reply and names the stop cause

Jev asks the decision again, up to twice, when the service's reply cannot be used. If it stays unusable the run ends `error` with the new `JevStopCause::DecisionUnusable`, whose stop reason gives the reply count and the first reason. New public `JevBilled::unusable` marks a billed reply the loop may ask again about. No fallback is routed after it.
