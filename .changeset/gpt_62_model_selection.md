---
roder-api: patch
roder-app-server: patch
roder: patch
---

# Add GPT-6.2 Astra and Sol to model selection

Offer `gpt-6.2-astra` and `gpt-6.2-sol` in the OpenAI and Codex model
pickers with 500k-token default context windows, an 872k maximum matching
the local Codex catalog, and low through ultra reasoning. Keep existing
models and provider model defaults unchanged.
