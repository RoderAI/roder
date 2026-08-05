---
roder-ext-openai-responses: patch
---

### Fixes

#### Include developer/ultra policy in OpenAI and xAI Responses `instructions`

Join stable `system` + `developer` into the Responses top-level `instructions`
field for OpenAI, Codex, xAI, and SuperGrok so ultra-mode multi-agent policy,
plan mode, goals, and other developer-slot addenda actually reach the model.
Keep per-turn `developer_context` as a leading input message outside the
stable prefix.
