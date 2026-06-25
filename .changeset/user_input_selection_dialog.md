---
roder-tui: minor
roder-extension-host: patch
---

# Render an interactive selection dialog for `request_user_input`

The TUI previously only logged a system line when the model called the
interactive `request_user_input` survey tool, so the question and its options
were never shown and the blocked turn appeared to hang. The TUI now opens a
modal selection dialog listing each option with its description, lets you
navigate with the arrow keys (or `Ctrl+J`/`Ctrl+K`), jump with number keys
`1`-`9`, confirm with `Enter`, and skip with `Esc`. Confirming sends
`thread/resolve_user_input` with the chosen option label keyed by question id;
multi-question surveys are answered one at a time and accumulated before
resolving. The turn timer pauses while the dialog is open and resumes once it is
resolved, and a survey with no answerable options resolves immediately so the
turn never hangs.

The local mock inference provider now also drives `request_user_input` when a
user message contains `FAKE_REQUEST_USER_INPUT`, so the selection dialog can be
exercised end to end without a live provider.
