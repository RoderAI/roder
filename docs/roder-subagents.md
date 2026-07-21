# Roder Subagents

Roder subagents are installed through `roder-ext-subagents`. When enabled, the extension registers the canonical `task` tool and an in-process dispatcher that runs a child turn with its own agent definition, model override, scoped tools, timeout, and bounded result.

Subagents are disabled by default. With no `[subagents]` config, `tools/list` does not expose `task`.

## Config

Add subagents to `~/.roder/config.toml`:

```toml
[subagents]
enabled = true
default_agent = "explore"
default_timeout_seconds = 180
max_concurrent = 2
max_depth = 1
include_child_transcript = false
expose_per_type = false

[subagents.disk]
user_dir = "~/.roder/agents"
workspace_dir = ".roder/agents"
```

`max_depth = 1` means a child cannot spawn another `task` call. Increase it only when nested delegation is intentional.

## Environment

- `RODER_SUBAGENTS_DEFAULT`: overrides `default_agent`.
- `RODER_SUBAGENTS_MAX_DEPTH`: overrides `max_depth`.
- `RODER_LIVE_SUBAGENTS=1`: enables opt-in live smoke tests where a test supports them.

Provider API keys are still owned by the provider extensions. The subagent extension does not read model API keys.

## Agent Files

Agent definitions load from the user directory first, then the workspace directory. Workspace definitions override user definitions with the same `name`.

```markdown
---
name: explore
description: Read-only repository exploration for locating code and references.
model: mock
tools: [echo]
permission_mode: read_only
max_turns: 8
max_result_chars: 4000
---

You are an exploration subagent. Search the workspace and report findings.
```

Required fields are `name`, `description`, and `tools`. Unknown tools fail during registry construction instead of being skipped.

## Tool Contract

The canonical tool is `task`. Per-agent tools such as `task_explore` are registered only when `expose_per_type = true`.

```json
{
  "description": "Inspect repository",
  "prompt": "Find where provider selection is configured.",
  "subagent_type": "explore",
  "model": "mock",
  "tools": ["echo"],
  "timeout_seconds": 60
}
```

`description` and `prompt` are required. `subagent_type` must exactly match a
configured role or it defaults to `[subagents].default_agent`. A lane only
restricts the selected role's declared tool whitelist; it never creates a role
or grants tools. If a lane leaves the selected role with no compatible tools,
the task fails before a child is started. Use `spawn_agent` for generic
repository work when it is available. The optional `tools` list can only further
restrict the agent definition's whitelist.

The tool result text is the child final message with a short agent label. Structured result data includes the child `thread_id`, `turn_id`, `agent_type`, model, usage, exit reason, and optional bounded transcript.

## App-Server Surface

`agents/list` returns public summaries of installed agents:

```json
{
  "jsonrpc": "2.0",
  "id": "agents",
  "method": "agents/list"
}
```

The response omits private system prompts:

```json
{
  "agents": [
    {
      "agent_type": "explore",
      "description": "Read-only repository exploration for locating code and references.",
      "tools": ["echo"],
      "model": "mock",
      "permission_mode": "read_only",
      "max_turns": 8,
      "max_result_chars": 4000
    }
  ]
}
```

When a `task` call runs, the runtime emits canonical subagent events on the app-server event stream. Each child event carries both the child ids and the parent `thread_id` / `turn_id`:

```json
{
  "kind": "subagent.completed",
  "event": {
    "SubagentCompleted": {
      "thread_id": "child-thread",
      "turn_id": "child-turn",
      "parent_thread_id": "parent-thread",
      "parent_turn_id": "parent-turn",
      "agent_type": "explore",
      "exit_reason": "completed"
    }
  }
}
```

Failure events expose a stable error category, not raw provider payloads, prompts, or secrets.

## Relationship To Skills And Goal Mode

Skills are reusable instruction packages and workflows. A subagent definition can point a child model at a skill-like role, but it is still just an agent definition with a tool whitelist and prompt.

Goal mode owns long-running objective tracking. Subagents are short child turns that return a bounded result to the parent. A future goal-mode planner can call `task`, but subagents do not replace goal state or roadmap planning.

## Verification

Local tests do not require provider credentials:

```sh
cargo test -p roder-ext-subagents
cargo test -p roder-extension-host -p roder subagents
cargo test -p roder-core -p roder-app-server task
cargo test -p roder-app-server agents
```

Live smoke tests must remain opt in through `RODER_LIVE_SUBAGENTS=1`.
