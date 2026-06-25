---
roder: patch
roder-tui: patch
---

# Fix needing a second Ctrl+C to exit a live Claude Code session

Interrupt any in-flight turn during TUI teardown and exit the process cleanly
once the terminal is restored, so a single Ctrl+C fully exits even when an
in-process provider (e.g. Claude Code) has spawned a CLI subprocess whose
runtime tasks would otherwise block shutdown.
