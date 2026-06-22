---
roder-api: minor
roder-ext-synthetic: minor
roder-extension-host: minor
roder: minor
roder-app-server: patch
roder-tui: patch
---

# Add first-party Synthetic inference provider

Adds the `synthetic` provider using Synthetic's OpenAI-compatible Chat
Completions API. The provider ships built-in `syn:` model aliases
(`syn:large:text` default, plus `syn:small:text`, `syn:large:vision`,
`syn:small:vision`), preserves concrete `hf:{owner}/{model}` ids across config,
discovery, and selection, and resolves credentials only from
`SYNTHETIC_API_KEY`/`RODER_SYNTHETIC_API_KEY` or `[providers.synthetic]`. The
provider is visible without credentials so app-server and TUI can show setup
state, and turn-time inference fails locally with setup guidance when the key
is missing. The TUI provider menu points to the Synthetic dashboard for API-key
setup instead of the generic fallback URL.
