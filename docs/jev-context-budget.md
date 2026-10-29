# Jev browser context budgets

Hosted TypeSafe decisions must fit both documented default limits:

- Shared state plus all questions: 64,000 tokens.
- Shared state plus each individual question: 32,000 tokens.

Roder checks the final request after the prompt profile, safety questions and
image placement have been assembled. The check leaves 4,096 tokens of request
headroom and 2,048 tokens of per-question headroom. It does not enable the
provider's long-context header.

TypeSafe does not publish a tokenizer or token-count endpoint in its API.
Preflight therefore uses a conservative estimate: UTF-8 byte counts for text and
JSON structure, plus an image reserve of two tokens per 14-by-14 pixel patch.
Every occurrence of an image reference is reserved, including references repeated
across questions; base64 upload bytes are not counted as prompt text. This is an
estimate, not an exact tokenizer. The provider remains authoritative. A context
rejection can cause one smaller repack; requests are never retried unchanged in
that path, and missing usage remains unknown.

When the complete observation will not fit, the client provides bounded pages
of text/DOM evidence and controls. `READ_MORE_TEXT`, `READ_PREVIOUS_TEXT`,
`READ_MORE_TARGETS` and `READ_PREVIOUS_TARGETS` let Jev request another page.
These operations read the saved observation and never send input to the site.
Real action IDs remain unchanged when a page is reindexed. Missing content is
explicitly marked as unobserved; it is not evidence of absence or completion.

The goal and irreversible-action safety questions are never truncated. If they
cannot fit even with a minimal observation, the call fails locally without
sending a request or executing a browser action. Context reads count toward the
existing provider-call budget, have a separate bounded read loop, and retain
all reported usage if a later read fails. Decision usage metadata includes the
estimated budgets and context reads for review.

These limits apply to the hosted Jev client. The OpenAI Decisions adapter keeps
its own provider contract and request budgets.

Sources: [TypeSafe model context limits](https://docs.typesafe.ai/models),
[request errors](https://docs.typesafe.ai/api/errors), and
[image reference expansion](https://docs.typesafe.ai/images), checked October 28, 2026.
