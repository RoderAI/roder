---
roder-ext-jev: minor
---

# Route Jev decisions through OpenRouter

`JEV_DECISION_PROVIDER=openrouter` sends Jev's System One requests to `https://openrouter.ai/api/v1/systemone` with `OPENROUTER_API_KEY` (or the configured `openrouter` provider key), text only. `JEV_DECISION_URL` overrides the URL of either hosted Jev route. Run usage now carries the dollar `cost` a host reports when every call reported one, and live eval rows, tallies and summaries record decision cost and the model that answered.
