---
roder-ext-jev: major
---

# Jev ends `needs_login` at sign-in walls and scrubs secrets in every encoding

Breaking: a run that reaches a sign-in it cannot make now ends with the new status `needs_login` (`JevStatus::NeedsLogin`) instead of `needs_input` or `blocked`, and `JevRunResult` gains `wall` (`JevWall` with `kind` `password`, `one_time_code` or `sign_in` as `JevWallKind`, the page's `origin`, and the labels of the `fields` it asked for). It applies when the goal gives no value for a password or one-time-code field (or a field in the same form as one), when a `blocked` run ends on a page that shows a sign-in or single sign-on wall, and to an HTTP 401 with nothing to act on, which was `access_denied`. `needs_login` is a handoff (not a failed tool call), never falls back, and its next step asks the user to sign in in the tab or give the code. A field for a new password, or any field of a form that holds one (a sign-up), is still `needs_input`.

A typed password or code is now scrubbed from page reads, addresses, evidence and the result as typed, percent-encoded (a GET form's `password=p%40ss+word%26`, `encodeURIComponent`), JSON-escaped and with its whitespace collapsed. Scrubbing twice changes nothing, and the host of an address on the origin the secret was typed on keeps its name.
