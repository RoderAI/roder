---
roder-api: minor
roder-ext-openai-responses: patch
roder-ext-xai: patch
roder: patch
---

# Add Grok 4.5 to xAI and SuperGrok providers

Expose `grok-4.5` (500k context, default high reasoning, low/medium/high) as the
default model for both the `xai` API-key provider and SuperGrok OAuth. Keep
legacy Grok 4.3 / 4.20 and SuperGrok Build/Composer entries selectable.
