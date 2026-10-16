---
roder-tui: patch
---

# Keep concurrent subagent streams out of the main transcript

Route background agent reasoning, messages, tools, and lifecycle events to
independent transcripts instead of mixing their tokens and turn counters with
the parent. Press Left with an empty composer to open an agent overview, then
Enter to inspect one agent's read-only live transcript and Left or Escape to
return. Approval and input requests still use the normal dialogs.
