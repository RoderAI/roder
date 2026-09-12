---
roder: patch
---

# Ship local hooks, new frontier models, and wait_agent mailbox wakeup

The `roder` CLI now includes Codex-compatible local project hooks (session-scoped
`SessionStart`, deny/chaining fixes, tighter diagnostics), Gemini 3.7/3.8 Flash,
Grok 4.6, GPT-6 Astra, and Claude Fable 5.1, plus a `wait_agent` fix so mailbox
activity queued before the waiter starts is observed.
