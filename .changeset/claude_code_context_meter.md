---
roder-api: minor
roder-ext-claude-code: patch
roder-tui: patch
roder-evals: patch
roder-sdk-typescript: patch
roder: patch
---

# Claude Code context meter shows the context, not the turn's total spend

Claude Code runs its own agent loop, and its turn result sums the usage of
every API request in the turn. The TUI context meter read that sum, so a turn
with one tool call showed about twice the real context (209% of the window on
Claude Code Haiku).

- `TokenUsage` has a new optional `context_tokens` field: the prompt plus
  output of the step's latest model request. `TokenUsage::context_window_tokens()`
  returns it, or `total_tokens` when a provider does not set it.
- The Claude Code provider sets `context_tokens` on the turn's usage from the
  last request it saw. Turn and thread token totals still count every request.
- The TUI context meter and context breakdown use the latest request.
- Claude Haiku 5.5 (`claude-haiku-5-5`, direct and through Claude Code) and the
  Claude Code `haiku` alias now have a 1M token context window, up from 200K.
  Claude Haiku 4.5 stays at 200K.
