# Agent backends

Roder's default backend runs Roder's own inference and tool loop. A complete
agent backend owns its own conversation, inference, tools, and policy. Backend
extensions register an `AgentBackend` service in the extension registry. Roder
app-server maps its existing thread, turn, event, and approval API to the
selected backend. Clients, including the existing Roder TUI, continue to use
the Roder protocol.

## Codex

Start a Codex-backed Roder session:

```sh
roder --backend codex
```

Select a Codex model or resume a Codex thread by its id:

```sh
roder --backend codex --model gpt-6-sol
roder --backend codex resume <codex-thread-id>
```

Roder spawns `codex app-server` from `PATH` in the current directory and speaks
the app-server JSONL protocol over its private stdio stream. Codex must be
installed and signed in. The existing Roder UI handles streamed messages,
tool activity, command output, file edits, token usage, and approval dialogs. Roder's policy mode maps to
Codex's approval and sandbox policy: Default uses on-request/workspace-write,
AcceptAll uses never/workspace-write, Plan uses on-request/read-only, and
Bypass uses never/danger-full-access. Codex's own configuration still supplies
other settings.

The same backend works through Roder's app-server:

```sh
roder app-server --backend codex
```

An app-server client continues to call Roder's `thread/start`, `turn/start`,
`turn/steer`, and `turn/interrupt` methods and receives Roder notifications.
Roder's model picker reads Codex's `model/list`; saved threads come from Codex's
`thread/list` and `thread/resume`.

The `roder-ext-codex-backend` crate translates Codex's thread, turn, item, and
approval messages to Roder's backend protocol. Other agent runtimes can
implement the same interface without joining Roder's inference provider or tool
registry. Codex client actions beyond command approvals, file-change approvals,
permission grants, and option-based user questions currently receive an explicit unsupported-method
response, so the turn fails instead of waiting indefinitely for an answer.
