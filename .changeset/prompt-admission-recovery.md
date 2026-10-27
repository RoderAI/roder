---
roder-tui: patch
roder-core: patch
---

# Retain prompts through admission and recover rejected steering safely

Gate queued input on pending starts, accepted receipts, unsettled settings, and
copy mode. Retain each request's exact prompt and attachments, recover a
definitively absent steer target once without clearing a newer turn, and preserve
unrelated queued inputs during manual retry. Ambiguous responses never trigger
an automatic resend. Scope queued/recoverable inputs to their original thread
and reconcile accepted receipts against persisted lifecycle evidence.
Keep the admission-time running lifecycle authoritative so a delayed task-start
event cannot overwrite a subsequent interrupt request.
