---
roder: patch
roder-api: minor
roder-ext-openai-images: minor
roder-extension-host: patch
roder-app-server: patch
roder-tui: patch
---

# Support GPT Image 2.5 Flare and Sunburst

Add both GPT Image 2.5 models to the image catalog and make Flare the default
OpenAI image model across generation, editing, provider discovery, and the TUI
image command. Preserve explicit selection of older image models. Cover both
2.5 models and all six quality levels with offline HTTP regression tests.

Document that the API-key provider uses separate API billing, while Codex's
public subscription interface does not yet expose a verifiable 2.5 selector.
