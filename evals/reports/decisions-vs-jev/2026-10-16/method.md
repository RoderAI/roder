# Jev 1.14 image input protocol — 2026-10-16

Same tasks, grading, harness and rotation as the
[October 8 protocol](../2026-10-08/method.md); only the variants differ.

| Variant | Provider and model | Request |
|---|---|---|
| `vision` | OpenAI Decisions, `gpt-6-luna` | Native prompts + action effects + viewport screenshot and candidate rectangles (Roder's opt-in OpenAI backend) |
| `jev@jev-1.13.0` | TypeSafe, pinned `jev-1.13.0` (what `jev-latest` served on October 8) | Baseline Jev request, text only |
| `jev@jev-preview` | TypeSafe, `jev-preview` → `jev-1.14.0` | Baseline Jev request, text only |
| `jev-vision@jev-preview` | TypeSafe, `jev-preview` → `jev-1.14.0` | Baseline Jev request + viewport screenshot asset and candidate rectangles, `TypeSafe-Danger-Zone-Images: true` |

The answering model is recorded per attempt from the provider's response.
`jev-preview` answered as `jev-1.14.0` on every call.

1. Confirmation: the 24 development tasks of October 6 and 8, three fresh
   repetitions, all four variants, 288 attempts.
2. Held-out: the four held-out tasks frozen on October 8 (hashes in that
   protocol), three repetitions, the same four variants, 48 attempts.

Each attempt gets a fresh local site and page. Provider order rotates by task
and repetition. Execution is sequential, with pooled HTTP clients and a
60-second per-task timeout. Values for form filling come from fixture data;
fallback and generative text completion are disabled for every provider. No
attempt is removed or rerun. No prompt was tuned on these runs.

Image path. The screenshot is the one the visual Decisions variant receives:
Chrome's `Page.captureScreenshot` JPEG (quality 70) of the 1120×780 viewport at
device scale 1, captured only for variants that use images and suppressed
around recognized secret fields and previously typed secrets. For Jev it is
fitted to TypeSafe's limits (PNG/JPEG, 280–1568 px per side, ≤16 MiB); a
screenshot already inside the limits, as all of these were, is sent byte for
byte as one `data_base64` asset under a content-derived UUID, referenced once
from `state.viewport_screenshot`. The state also carries the candidates'
on-screen rectangles, mirroring the Decisions variant's evidence. Questions and
criteria are unchanged from the text request.

Only the image variants pay screenshot-capture time (recorded per attempt as
`watch.laps.screenshot`). Wall times are local end-to-end measurements on a
shared development machine and include browser work; they are not isolated API
latency. Wire latency is the client-side time of each provider call recorded
by the eval transport, including network. Input tokens are provider-reported
and are not comparable in dollars across providers; TypeSafe bills image
tokens as input tokens.

The test binary was built once from this branch, copied, and run from the copy
(the shared Cargo target directory can otherwise swap in another worktree's
binary). Raw rows contain the wire requests and responses for both providers,
including the throwaway fixture screenshots; they are gzip-compressed. No API
keys or authorization headers are recorded.
