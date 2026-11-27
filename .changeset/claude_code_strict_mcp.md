---
roder-ext-claude-code: patch
roder: patch
---

# Claude Code sessions load only Roder's MCP tools

Claude Code sessions started by Roder now pass `--strict-mcp-config`, so the CLI no longer loads the user's own MCP servers and claude.ai connectors. Roder already denied those tools, but their schemas went into every request. With a few connectors set up, a one-line prompt cost about 160K input tokens; it now costs under 1K. Claude-in-Chrome still works.
