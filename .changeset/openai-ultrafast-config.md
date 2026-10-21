---
roder-config: patch
roder-ext-openai-responses: minor
roder-extension-host: minor
roder: minor
roder-configure: patch
---

# Configure OpenAI Ultrafast mode

Set `[providers.openai] service_tier = "ultrafast"` to request Ultrafast mode
by default. Explicit per-request service tiers take precedence. The setting
applies to the OpenAI API provider and is also passed through generated remote
app servers.
