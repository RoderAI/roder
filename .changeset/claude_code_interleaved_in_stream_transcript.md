---
roder-core: patch
---

# Interleave in-stream reasoning/text with tool calls in the transcript

Providers like `claude-code` run their entire tool loop inside a single
`stream_turn`, streaming many assistant messages, thinking blocks, and tool
calls before the turn completes. The turn loop previously coalesced every
reasoning/text chunk into one trailing `ReasoningSummary` + `AssistantMessage`
that was persisted only after all the in-stream tool calls (which the runtime
tool executor persists in real time). The result was a transcript where every
tool row appeared first and all the per-step thinking/narration collapsed into a
single block at the end, so the activity between tool calls looked missing.

The runtime now keeps a shared per-turn buffer of the reasoning + final-answer
text streamed so far. `RuntimeTurnToolExecutor::execute` flushes that buffer as
discrete `ReasoningSummary` + `AssistantMessage` items immediately before it
persists each in-stream tool call, and the turn-end persistence writes only the
remaining (post-last-tool) content. The persisted order is now
`reasoning -> text -> tool -> reasoning -> text -> tool -> ... -> final answer`.
This is gated to providers that execute tools in-stream (currently
`claude-code`); all other providers are unchanged.
