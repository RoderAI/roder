# Run Roder through OpenClaw ACP

Use a Roder build that advertises ACP load, resume, and close support. Install the
CLI, configure its provider/model, and authenticate that provider before starting
an unattended OpenClaw session. For Codex, use `roder auth login codex`. Roder owns
the provider credentials; OpenClaw's ACPx plugin launches `roder acp`.

Merge this configuration into OpenClaw's existing settings. Preserve any other
allowed agents and plugin settings. Replace the executable path with the Roder
binary that the gateway should run.

```json
{
  "acp": {
    "enabled": true,
    "backend": "acpx",
    "allowedAgents": ["roder"]
  },
  "plugins": {
    "entries": {
      "acpx": {
        "enabled": true,
        "config": {
          "agents": {
            "roder": {
              "command": "/absolute/path/to/roder acp"
            }
          }
        }
      }
    }
  }
}
```

The explicit agent override works with ACPx releases whose built-in registry does
not yet include Roder. Set `acp.defaultAgent` to `roder` if it should be the default.
After applying the configuration through OpenClaw's normal gateway workflow,
use `/acp doctor`, then `/acp spawn roder` to start a session. See
[OpenClaw's ACP agent guide](https://docs.openclaw.ai/tools/acp-agents) for channel
bindings and gateway configuration.

Persistent sessions retain the same native Roder thread ID. ACPx can reconnect
with `session/load` (history replay) or `session/resume` (no replay) after the
Roder process exits. Roder requires the original cwd and durable thread store.
Closing cancels ongoing work, releases live resources, and preserves the stored
conversation. A failed cleanup returns an error and can be retried.

Roder's current ACP endpoint accepts text/resource-link prompts and bridges tool
permissions. It does not advertise model controls, session list/delete,
additional workspace roots, or MCP-over-ACP. Select models in Roder configuration;
leave OpenClaw's optional MCP-server injection disabled for this agent.
