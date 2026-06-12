---
description: Summarize recent repository activity as a standup update.
argument-hint: "[days]"
---

Write a standup-style update for the last {{arguments}} day(s) of work in this
repository. Check `git log` for recent commits, group them by theme, and
produce three sections: Done, In Progress, and Blockers (infer blockers from
TODO/FIXME comments touched recently). Keep it under 15 lines.
