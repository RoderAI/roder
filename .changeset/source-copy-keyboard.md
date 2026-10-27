---
roder-tui: minor
---

# Copy original responses and code blocks with the keyboard

Add a `/copy` source selection mode with keyboard navigation and exact source
copying independent of terminal wrapping. Preserve whitespace and line endings,
keep selection stable across streaming and resizing, and report clipboard
failures instead of displaying premature success.
