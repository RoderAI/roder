---
roder-ext-claude-code: patch
---

# Fix only the first claude-code tool call rendering in the TUI

The claude-code provider derived each in-process tool-call id solely from the
tool name (`claude-code-<Tool>`), so every later invocation of the same tool
(e.g. repeated `Bash`/`Read` calls) reused one id. The TUI and runtime key
tool-call rows by id, which collapsed all subsequent calls into the first
row, making it look like only the first tool call ever ran. Tool-call ids are
now made unique per invocation via a process-global counter
(`claude-code-<Tool>-<seq>`), so each call renders as its own row.
