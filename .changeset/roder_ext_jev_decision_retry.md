---
roder-ext-jev: minor
---

# Jev retries an unusable decision reply and falls back when it stays unusable

Jev asks the decision again, up to twice, when the service's reply cannot be used. That includes a reply body that cannot be decoded, after the HTTP layer's one resend; it is counted with unknown usage. If the replies stay unusable the run ends `error` with the new `JevStopCause::DecisionUnusable`, whose stop reason gives the reply count and the first reason. With the default `JEV_FALLBACK=auto` the frontier model then takes over with the new `decision_unusable` fallback trigger, unless Jev declined a page dialog during the run; `handover` tells the caller to go on with the browser tools instead, and `off` leaves the error as it was. New public `JevBilled::unusable` marks a billed reply the loop may ask again about. Every other `error` (a refused key, billing or rate limit, a server error, an unreachable provider) still does not fall back.
