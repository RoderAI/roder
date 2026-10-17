# Integration review after the source freeze

The 664-attempt campaign finished before these follow-ups changed source. The
377 frozen source/configuration/fixture files still matched the freeze at
completion; `validation-source-after-campaign.log.gz` records that source check.
The executable is identified separately by the SHA-256 digest in `freeze.json`;
that log does not verify its bytes.
The measured source remains `50fdd040910dcfe466b31b9a96947436717643cc`.

- [Custom-client budgets](https://github.com/RoderAI/roder/pull/157#discussion_r4301541312):
  expose `JevDecisionTransport::decide_budgeted()` as the public reservation
  boundary. Built-in clients use that same boundary. Custom clients must reserve
  each component and aggregate token usage, including billed errors. Known
  reservations also populate omitted `usage.calls`. Direct I/O bypassing this
  contract cannot be observed or prevented by the engine. Integration tests
  exercise a custom client with two calls and a denied third call, without a
  reported `usage.calls` field. Decision interfaces move out of the 489-line
  engine module to keep its implementation below 500 lines.
- [Split-request documentation](https://github.com/RoderAI/roder/pull/157#discussion_r4301541318):
  qualify the split with an available fresh image, a populated clipped field and
  the audited profile. Otherwise the initial questions stay combined; an audit
  can still be a later text-only request.
- [Audit request counts](https://github.com/RoderAI/roder/pull/157#discussion_r4301541322):
  assert two requests for a completed activation with a usable conditional click
  target: the second request establishes that the activation already happened.
  Also assert exactly one request when no click target is offered. Skipping the
  audit merely because the original answer is DONE would defeat its purpose.
- [Native image fitting](https://github.com/RoderAI/roder/pull/157#discussion_r4301541316):
  keep native viewport pixels unchanged. The built-in Chrome implementation sets
  device scale factor 1 and captures the matching viewport, so its CSS rectangles
  already match image pixels. This path performs no client-side resize. The
  [native Decisions image contract](https://developers.openai.com/api/reference/resources/decisions/methods/create)
  differs from TypeSafe's 280–1568-pixel fitting policy; applying the latter would
  alter native model input. Custom hosts must provide a valid inline full-viewport
  image aligned with their observed rectangles, now explicit in the embedding
  docs. A malformed image rejected by the provider stops the request without
  dispatching an action. This does not claim automatic normalization of arbitrary
  custom screenshots.

These changes do not retune the frozen native prompt or image routing, or alter
fixture pages. Their source hashes and separate validation are recorded in
`validation.json`; no new paid campaign is substituted for the frozen results.
