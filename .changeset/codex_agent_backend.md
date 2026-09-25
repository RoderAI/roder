---
roder-api: minor
roder-app-server: minor
roder-ext-codex-backend: minor
roder-ext-openai-responses: patch
roder-ext-anthropic: patch
roder-tui: minor
roder: minor
---

# Select Codex as a complete agent backend for Roder

Add an extension service for complete agent backends and a Codex app-server implementation. Roder app-server maps its thread, turn, streaming event, and approval APIs to Codex, so existing Roder clients and the TUI can use Codex as the agent runtime.

Refresh GPT-6 and Claude Opus 5.5/Sonnet 5 catalog entries and map Codex token usage into Roder's turn counters.
