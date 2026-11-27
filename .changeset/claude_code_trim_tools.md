---
roder-api: minor
roder-core: patch
roder-ext-claude-code: patch
roder: patch
---

# Claude Code sessions get only the Roder tools they need

Claude Code sessions now receive one copy of each core tool, under Claude Code's native name (`Bash`, `Read`, `Edit` and so on). Previously each also came with a second copy under its Roder name (`shell`, `read_file`, `edit`...).

They also no longer receive:

- Roder's browser tools (`chrome_*`, `jev_*`, `browser_use_*`, `webwright.*`). Claude Code has its own browser through Claude-in-Chrome, and the system prompt's browser routing section is dropped with them.
- The `zerolang_*`, `design_*`, `knowledge_*` and `memory_*` tools.
- The Codex-style `exec_command`, `unified_exec`, `write_stdin` and `apply_patch`.
- The `echo` and `media_generate_video` test tools.

Every tool schema is sent with every request. This cuts Roder's tool list in a Claude Code session from 117 tools to 35, and the schema text from about 76K characters to about 22K.

The browser routing guidance moved from `roder-core` to `roder_api::browser_routing` so providers can remove it.
