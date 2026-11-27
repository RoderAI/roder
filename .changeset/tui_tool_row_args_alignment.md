---
roder-tui: patch
---

# Cleaner tool rows in the timeline

Tool rows no longer show raw JSON while arguments are still streaming in.
Headline values such as `message`, `prompt`, and `goal` show without a `key=`
label, and `apply_patch` calls list their file operations.

The right-hand duration column now shows how long each tool ran (ticking live
while it runs) instead of the gap since the previous row, and is right-aligned
at a fixed width so timestamps and durations line up.
