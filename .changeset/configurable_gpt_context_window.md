---
roder-config: major
roder-configure: patch
roder-core: major
roder-protocol: major
roder: patch
roder-app-server: patch
roder-dist-hosted: patch
roder-evals: patch
roder-ext-anthropic: patch
roder-ext-claude-code: patch
roder-ext-codex-backend: patch
roder-ext-cua: patch
roder-ext-deepseek: patch
roder-ext-hopper: patch
roder-ext-jev: patch
roder-ext-opencode: patch
roder-ext-poolside: patch
roder-ext-postgres-session: patch
roder-ext-runner-sprites: patch
roder-ext-synthetic: patch
roder-ext-webwright: patch
roder-ext-xiaomi-mimo: patch
roder-extension-host: patch
roder-tui: patch
---

# Configure GPT and Codex context windows from TUI Settings

Use 500k-token default working context windows for GPT models that support
them. Add a persisted `model_context_window` preference with a numeric TUI
editor and reset-to-default action. Clamp each model's effective window to
its supported maximum and apply the same budget to context counters and
runtime compaction.

Rust integrators must rebuild against the updated Config, RuntimeConfig,
and settings response structs, which add the optional context-window
preference. Release their compatible reverse dependencies together.
