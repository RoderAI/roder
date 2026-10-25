# Use Roder with ACPx and OpenClaw

Users can run Roder through ACPx and OpenClaw today by registering a local command
named `roder`. An entry in the upstream agent registry is not required.

The launch command is always the Roder executable followed by `acp`. Configure
that command in the client that will launch it:

<!-- markdownlint-disable MD013 -->

| Client | Where to register `roder` |
| --- | --- |
| Standalone ACPx CLI | `agents.roder` in `~/.acpx/config.json` |
| OpenClaw's ACPx plugin | `plugins.entries.acpx.config.agents.roder` in OpenClaw config, plus `roder` in `acp.allowedAgents` |

<!-- markdownlint-enable MD013 -->

OpenClaw's embedded runtime has its own configuration. Configuring the standalone
ACPx CLI alone does not register the agent in OpenClaw.

## 1. Prepare Roder on the gateway machine

Run these steps as the same OS user that runs the OpenClaw gateway. Roder needs
that user's provider credentials, configuration and persisted threads. If the
gateway is on another machine, install and authenticate Roder there.

Use a build containing [Roder PR #180](https://github.com/RoderAI/roder/pull/180),
which added persistent load/resume/close and ACP authentication. The
`roder/v0.3.20` release predates that change. Until a release includes it, build
from `master` using the repository's Rust/mise prerequisites:

```bash
git clone https://github.com/RoderAI/roder.git
cd roder
mise install
make build
realpath bin/roder
```

Use the absolute path printed by the last command in the client configuration
below. A source build may still report the preceding release version, so check
its ACP capabilities using the readiness probe later in this guide.

For an installed binary, find its path and version with:

```bash
command -v roder
roder --version
```

Configure the provider and model in `~/.roder/config.toml`. For example:

```toml
provider = "codex"
model = "gpt-6-sol"
```

Authenticate before starting an unattended session:

```bash
roder auth login codex
```

When testing a source build, use its absolute executable path for the login
command too. Other providers use their own Roder authentication or API-key setup;
see the [provider table](../README.md#provider-quickstart). Roder owns these credentials.
Select the model in Roder configuration; its ACP endpoint does not expose ACPx's
`--model` control.

If the gateway uses a different `HOME`, `RODER_CONFIG_DIR` or `RODER_DATA_DIR`,
ensure it sees the intended configuration, credentials and persistent data.
Keep those locations stable across gateway restarts.

## 2. Verify standalone ACPx

This is a useful diagnostic before involving OpenClaw. OpenClaw's bundled ACPx
runtime does not require a separate global ACPx installation.

Install the CLI if needed:

```bash
npm install -g acpx
```

Merge this entry into `~/.acpx/config.json`, preserving existing agent entries.
Replace the executable path with the one found above:

```json
{
  "agents": {
    "roder": {
      "command": "/absolute/path/to/roder acp"
    }
  }
}
```

The single command string works with the Unix ACPx configuration used here.
For a path containing spaces, quote the executable inside the JSON string:
`"command": "\"/path with spaces/roder\" acp"`.

Use an existing project directory consistently:

```bash
acpx --cwd /absolute/project config show
acpx --cwd /absolute/project roder exec 'Reply with READY.'
acpx --cwd /absolute/project roder sessions new --name onboarding
acpx --cwd /absolute/project roder -s onboarding 'Remember the marker cedar-47.'
acpx --cwd /absolute/project roder -s onboarding 'What marker did I give you?'
acpx --cwd /absolute/project roder sessions show onboarding
```

Expected result: the one-shot request responds, both persistent prompts succeed,
and the follow-up retains the marker. This smoke test sends real provider
requests. `exec` uses a fresh ACP session; it does not test persistent recovery.

`<cwd>/.acpxrc.json` can override the global `roder` entry. If the wrong binary
launches, inspect the resolved configuration from the same project directory.

## 3. Register Roder in OpenClaw

Find the active OpenClaw configuration:

```bash
openclaw config file
```

It defaults to `~/.openclaw/openclaw.json`; `OPENCLAW_CONFIG_PATH` can select
another file. Merge the following settings into that file. Preserve existing
allowed agents and plugin settings instead of replacing the whole configuration.

```json
{
  "acp": {
    "enabled": true,
    "dispatch": { "enabled": true },
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
          },
          "pluginToolsMcpBridge": false
        }
      }
    }
  }
}
```

If `acp.allowedAgents` already contains other agents, append `roder`. If the
installation restricts plugins through `plugins.allow` or `plugins.deny`, ensure
the `acpx` plugin is allowed. The gateway must be able to execute the configured
absolute path.

Optional: set `acp.defaultAgent` to `roder` if requests without an explicit agent
should use Roder.

Validate, then restart the gateway to load the changed settings:

```bash
openclaw config validate
openclaw gateway restart
```

In an OpenClaw conversation, run:

```text
/acp doctor
/acp spawn roder --mode persistent --cwd /absolute/project
```

Send a short prompt, then a follow-up in the same session. `/acp doctor` checks
the backend; successfully spawning and prompting `roder` verifies the custom
agent configuration. Channel/thread binding is separate from adapter setup; see
[OpenClaw's ACP agent guide](https://docs.openclaw.ai/tools/acp-agents).

## Permissions and supported features

OpenClaw forwards Roder's protected tool requests to ACPx's permission policy.
When interactive approval is unavailable, requests may be denied. The plugin's
`permissionMode` accepts `approve-reads`, `deny-all`, or `approve-all`;
`nonInteractivePermissions` accepts `deny` or `fail`. `approve-all` grants every
permission request forwarded to ACPx. Choose that policy deliberately for trusted
unattended work. These are plugin-wide settings, not per-agent overrides.

Roder supports text/resource-link prompts, streamed output, tool permissions,
cancellation and persistent load/resume/close. Load replays stored conversation
history; resume restores the same context without replay. Close cancels and
drains active work, releases live resources and preserves the conversation.

Recovery requires the original cwd and durable Roder thread data. ACPx/OpenClaw
also keep their own session records; keep both clients' state and Roder's data
when moving or restarting a gateway. An ACPx record ID and a Roder native thread
ID are different identifiers.

Roder does not advertise dynamic model controls, session list/delete, extra
workspace roots or MCP-over-ACP. Keep optional OpenClaw MCP injection disabled
for Roder, including any configured `mcpServers`. If other agents need those
plugin-wide MCP settings, use a separate OpenClaw configuration for Roder.

## Troubleshooting

<!-- markdownlint-disable MD013 -->

| Symptom | What to check |
| --- | --- |
| `ACP agent "roder" is not allowed by policy` | Add `roder` to `acp.allowedAgents` in the active OpenClaw config. |
| ACPx tries to run bare `roder`, opens a TUI, or reports a missing executable | Register the complete absolute command ending in `acp` in the client actually being used. |
| CLI works but OpenClaw fails | Check the plugin's agent override, gateway OS user/HOME, executable path and credentials; the two clients use different config. |
| `Authentication required` | Run provider login as the gateway user using the same Roder binary/configuration. |
| `session/load`, `session/resume` or `session/close` is unsupported | The configured executable predates the lifecycle change. Replace it with a build containing PR #180 and restart the gateway. |
| `session cwd does not match its stored workspace` | Resume with the original project directory; changing cwd is not a new conversation under the old ID. |
| `additionalDirectories` or `mcpServers` is rejected | Remove extra roots and disable plugin MCP injection for this agent. |
| A protected tool is denied | Check Roder's policy and ACPx's permission/non-interactive policy. Successful text output alone does not verify tool permission setup. |
| A saved session is not found | Check gateway user, Roder data directory, and the native session ID. Preserve the old data before creating a replacement conversation. |

<!-- markdownlint-enable MD013 -->

## Internal readiness and support handoff

Before handing this setup to a user, verify the exact binary the gateway will
launch. This probe starts ACP, checks capabilities and exits without running a
model or opening a provider login:

```bash
python3 - /absolute/path/to/roder <<'PY'
import json
import selectors
import subprocess
import sys

process = subprocess.Popen(
    [sys.argv[1], "acp"], stdin=subprocess.PIPE,
    stdout=subprocess.PIPE, text=True,
)
try:
    process.stdin.write(json.dumps({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": 1, "clientCapabilities": {}},
    }) + "\n")
    process.stdin.flush()
    ready = selectors.DefaultSelector()
    ready.register(process.stdout, selectors.EVENT_READ)
    if not ready.select(15):
        raise SystemExit("Timed out waiting for ACP initialize")
    response = json.loads(process.stdout.readline())
    capabilities = response["result"]["agentCapabilities"]
    assert capabilities.get("loadSession"), "loadSession is missing"
    sessions = capabilities.get("sessionCapabilities", {})
    assert "resume" in sessions and "close" in sessions, sessions
    print("READY: persistent ACP load/resume/close")
finally:
    process.terminate()
    process.communicate(timeout=5)
PY
```

Record the Roder executable path/build, OpenClaw/ACPx versions, gateway user and
project cwd with the result of a real-provider prompt/follow-up. Keep credentials
out of the handoff. For a persistence check on a test session, restart the gateway
and confirm a follow-up continues that conversation using the same project and
state directories.

The adapter lifecycle has been tested through OpenClaw's bundled ACPx runtime
with mock inference, including cold history replay and close/resume. A user's
provider login and real-model/tool access still need their own smoke test.
