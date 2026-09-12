---
roder-core: patch
---

# Fire the `SessionStart` local hook once per session

`SessionStart` hung off `ThreadLoaded`, which is emitted on every read of a
thread from its store, so a single `roder exec` run executed its setup hooks
four times — including once after the turn had already stopped. The dispatch is
now claimed once per thread, whichever event opens the session.
