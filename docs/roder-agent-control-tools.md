# Roder Agent Control Tools

Roder has two subagent surfaces:

- `task`: a one-shot child run that returns a bounded final result to the parent.
- Agent-control tools: live subagent teammates backed by normal Roder threads.

The live control tools are model-facing tools available to the orchestrating agent:

| Tool | Purpose |
| --- | --- |
| `spawn_agent` | Create a long-lived teammate and send its initial task. |
| `send_message` | Send a direct message to an existing teammate. |
| `followup_task` | Assign follow-up work to an existing teammate. |
| `wait_agent` | Wait for a teammate, or any teammate owned by the caller, to complete. |
| `list_agents` | List teammates owned by the caller thread. |
| `close_agent` | Close a teammate when it is no longer needed. |

`spawn_agent` creates or reuses the caller thread's team, starts a teammate session, and sends the initial message through the same runtime path as `team/member/message`. The child has its own thread id, turn id, model selection, policy mode, tool registry, event stream, and app-server-visible team state.

Example `spawn_agent` arguments:

```json
{
  "task_name": "reviewer",
  "message": "Review the current patch and report risks.",
  "model": "mock"
}
```

The tool result includes stable ids for follow-up control:

```json
{
  "team_id": "team-id",
  "member_id": "member-1",
  "thread_id": "child-thread",
  "task_name": "reviewer",
  "turn_id": "child-turn",
  "status": "running"
}
```

Targets for `send_message`, `followup_task`, `wait_agent`, and `close_agent` can be a task name, member id, child thread id, or `team/member`. Unqualified targets resolve only within teams owned by the calling thread; ambiguous names require `team/member`.

When a provider returns multiple agent-control tool calls in one batch, Roder executes that batch sequentially in model order. This keeps lifecycle-sensitive flows like `send_message` -> `wait_agent` -> `close_agent` deterministic even when the active model otherwise supports parallel tool calls.

The app-server team methods (`team/start`, `team/member/message`, `team/member/interrupt`, and related methods) remain the external client control plane. These tools are the model-facing caller control plane over the same team runtime.
