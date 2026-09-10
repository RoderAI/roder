---
roder-tui: patch
---

Repair three stale TUI tests. The slash-command catalog gained `/ultra` without
its expected list being updated, and two transcript-action tests asserted a
policy mode that blocks process spawning — no such mode exists any more, and
both branches they covered are already tested elsewhere.
