# Roder runtime for OpenClaw

`@roderai/openclaw` is an OpenClaw plugin that launches
[Roder](https://roder.sh) directly over stdio ACP. It registers the `roder`
runtime backend; it does **not** run ACPx or require an ACPx agent-registry entry.
Roder supplies its normal model providers, workspace tools, MCP support, and
persisted conversations.

The package targets OpenClaw **2026.10.9**, whose public
`openclaw/plugin-sdk/acp-runtime-backend` contract it uses. Its optional peer is
pinned to that version rather than claiming compatibility with untested host
versions. Node must satisfy `>=24.22.1 <25 || >=26.12.1`, matching that host.
The optional host integration test exercises the real pinned SDK registry,
packed native entrypoint, CLI discovery, and config validation in temporary
directories. A separately opted-in gateway test verifies actual local
gateway-to-Roder prompt dispatch with the offline mock provider. These tests
do not qualify external channels, real-model credentials, or a production
gateway deployment.

## Install from this checkout

Run installation and authentication as the same OS user that runs the gateway.
Use a Roder build with `roder acp` persistent load/resume support; see the
[ACP preparation guide](../../docs/openclaw-acp.md#1-prepare-roder-on-the-gateway-machine)
for source-build and provider setup.

```sh
cd /absolute/path/to/roder/packages/openclaw
npm test
npm pack
```

`npm pack` reruns the tests and prints a tarball filename, such as
`roderai-openclaw-0.1.0.tgz`. Give the resulting absolute filename to OpenClaw:

```sh
openclaw plugins install /absolute/path/to/roder/packages/openclaw/roderai-openclaw-0.1.0.tgz \
  --force --accept-capabilities --acknowledge-install-policy-warning
```

For local development, OpenClaw also accepts a package directory:

```sh
openclaw plugins install --link /absolute/path/to/roder/packages/openclaw \
  --force --accept-capabilities --acknowledge-install-policy-warning
```

These flags acknowledge the local, non-ClawHub source and grant capability
consent. Review the package and its executable configuration before granting
that trust. They do not override a blocking host install policy. `--force` can
replace an existing installation of this plugin; omit it and review the host's
prompt if you do not intend replacement.

Do not assume this package has already been published to npm. The source and
local tarball paths above work without an npm release. Installation of this
plugin does not install the Roder executable or configure provider credentials.

## Gateway configuration

Find the active config with `openclaw config file`. Merge this example into it,
preserving existing plugins and allowed agents:

```json
{
  "agents": {
    "ownership": "explicit",
    "entries": {
      "main": { "workspace": "/absolute/project" },
      "roder": { "workspace": "/absolute/project" }
    }
  },
  "acp": {
    "enabled": true,
    "dispatch": { "enabled": true },
    "backend": "roder",
    "allowedAgents": ["roder"]
  },
  "plugins": {
    "entries": {
      "roder": {
        "enabled": true,
        "config": {
          "command": "/absolute/path/to/roder",
          "args": ["acp"],
          "cwd": "/absolute/project",
          "permissionMode": "deny-all"
        }
      }
    }
  }
}
```

Optionally set `acp.defaultAgent` to `"roder"` to select it when no agent is
specified for ACP spawning. This does not redirect ordinary main-agent chats
into Roder. If `plugins.allow` or `plugins.deny` restricts loading, allow plugin
id `roder`. The executable must exist on the gateway machine, not merely on the
machine running a chat client.

The `roder` entry in the **OpenClaw agent roster** is intentional and distinct
from the plugin/backend registration. OpenClaw 2026.10.9 creates
`agent:roder:acp:<id>` sessions for `/acp spawn roder`; its chat API checks that
`roder` exists in `agents.entries` before sending to that target. Merely adding
`acp.allowedAgents` can allow spawning but still leave chat sends failing with
`Unknown agent id "roder"`. Keep all existing agent entries, workspaces,
ownership/default-owner settings, model configuration, and credentials when
merging this example. The example's `main` entry represents an ordinary existing
owner, not a requirement to rename your current owner. The explicit multi-agent
roster must pass `openclaw config validate`.

The executable and arguments are separate fields: `command` must not contain
`"roder acp"` as a single command string. The default is command `"roder"` with
arguments `["acp"]`, resolved through the gateway's `PATH`. An absolute executable
path avoids service-manager `PATH` surprises. Processes are spawned with
`shell: false`; shell interpolation and shell aliases are not supported.

Roder inherits the gateway's environment. Authenticate using that gateway user's
Roder executable, for example `/absolute/path/to/roder auth login codex`, and
configure its provider/model in the normal Roder config. Preserve the intended
`HOME`, `RODER_CONFIG_DIR`, and `RODER_DATA_DIR` across restarts. OpenClaw's model
credentials do not automatically authenticate Roder. There is no arbitrary
per-request environment-injection option.

```sh
openclaw config validate
openclaw gateway restart
```

Then, in an OpenClaw conversation:

```text
/acp doctor
/acp spawn roder --mode persistent --cwd /absolute/project
```

Select the returned ACP session in the Control UI, or bind it to a supported
channel conversation using the host's ACP binding controls, before sending the
prompt. An unbound spawn does not redirect subsequent messages in the original
main session.

Send a prompt and a follow-up to that ACP target. Check streamed output, tool events, session status,
and cancellation before enabling unattended use. Doctor diagnostics alone are
not an end-to-end provider/tool-execution test.

## Permissions and trust

The default is **`deny-all`**. ACP permission requests are rejected unless
explicitly allowed by the plugin configuration:

| `permissionMode` | Behavior |
| --- | --- |
| `deny-all` | Deny every ACP permission request. |
| `approve-reads` | Approve only requests explicitly classified as read operations. Unknown or write-like requests fail closed. |
| `approve-all` | Approve ACP permission requests, including destructive operations. Use only in an appropriately isolated, trusted workspace. |

This is a response policy for permissions that Roder **asks the ACP client to
resolve**, not an operating-system sandbox or a replacement for Roder's own
tool policy. Operations that Roder's policy already permits can execute without
an ACP permission request, even with `deny-all`. Do not configure Roder to bypass
approval and expect this plugin to intercept every action. Review the gateway
user's Roder policy, enabled tools, subprocess environment, and filesystem
access before treating a session as restricted.

There is no interactive terminal approval fallback. Unexpected server requests
and permission shapes fail closed.

## Capabilities and limits

- Persistent and oneshot ACP sessions, with OpenClaw session ownership separated
  by agent id and session key.
- Prompt streaming, text/thought output, plans, usage, tool calls/updates, and
  terminal completion/error events.
- Base64 PNG, JPEG, WebP, and GIF image attachments when the selected model/server
  advertises image support. Audio and resource-link attachments are not supported
  by this runtime API; unsupported forms fail rather than silently losing content.
- Cancellation and subprocess cleanup, with bounded shutdown escalation.
- Session status, mode/config controls, model selection through advertised ACP
  controls, session close, and persistent restore when supported by Roder.
- **Steering an active turn and independent thinking-level control are not
  supported.** Use a new prompt after completion/cancellation. An explicit
  unsupported thinking request is rejected; inherited host thinking defaults
  must not be interpreted as having been applied.
- No native OpenClaw tools, channel credentials, or gateway MCP tool bridge are
  automatically injected into Roder. Use Roder's own tools and configured MCP
  servers. This plugin is an ACP backend, not an OpenClaw agent harness.

All sessions require a Roder ACP v1 server advertising both `session/resume`
and `session/close`; older binaries without these methods are rejected.
Persistent mode keeps the Roder conversation for later turns and gateway
restart recovery. Oneshot mode is for a single turn and does not promise a
reusable conversation. Persistent restore requires the same compatible Roder
installation, workspace, gateway owner, and Roder data directory; an arbitrary
external session id is not authority to attach to another owner's conversation.
Importing arbitrary existing native Roder threads is not supported:
`resumeSessionId`, if supplied, must match the plugin's existing owner mapping.
Restore the plugin metadata along with Roder's data; a missing mapping is not
permission to adopt an existing native thread.
Changing the configured execution/MCP binding can invalidate saved sessions
instead of silently reopening them under a different executable.

## Configuration reference

Unknown keys and invalid values are rejected, including embedded NULs, blank
commands, relative working directories, and unsupported MCP transports.

| Field | Default | Notes |
| --- | --- | --- |
| `command` | `"roder"` | Executable name or absolute path, not a shell command. |
| `args` | `["acp"]` | Literal argument array. |
| `cwd` | Gateway working directory | Must be absolute when specified. Session working directories are also validated. |
| `permissionMode` | `"deny-all"` | `deny-all`, `approve-reads`, or `approve-all`. |
| `requestTimeoutMs` | `30000` | RPC/control timeout; integer from 100 to 300000. |
| `cancelTimeoutMs` | `5000` | Cancellation grace period; integer from 100 to 60000. Process shutdown adds its own bounded TERM/KILL cleanup, so this is not a total shutdown deadline. |
| `maxMessageBytes` | `16777216` | Maximum incoming ACP frame; integer from 1024 to 67108864 bytes. |
| `mcpServers` | `[]` | ACP stdio or HTTP server definitions below. No SSE transport. |

MCP definitions use the ACP wire shape, not an arbitrary environment object:

```json
{
  "mcpServers": [
    {
      "name": "local-tools",
      "command": "/absolute/path/to/mcp-server",
      "args": ["--stdio"],
      "env": [{ "name": "SERVICE_TOKEN", "value": "replace-me" }]
    },
    {
      "type": "http",
      "name": "remote-tools",
      "url": "https://mcp.example.test/mcp",
      "headers": [{ "name": "Authorization", "value": "Bearer replace-me" }]
    }
  ]
}
```

Stdio definitions may explicitly set `"type": "stdio"`; it is omitted on the
ACP wire. HTTP URLs must be absolute HTTP(S), without URL credentials or
whitespace. MCP names and environment/header names must not be duplicated
within their corresponding lists. Protect the OpenClaw configuration if it
contains MCP secrets. Servers are passed only when the ACP agent advertises the
matching transport capability.

## State, backup, and recovery

The plugin stores session metadata under **`<OpenClaw service stateDir>/roder-acp`**.
Each owner has a hashed `.json` filename; writes are atomic, directories are
owner-only, and records are owner-only files. Symlinked state directories/files
are rejected. The OpenClaw service chooses the parent directory; do not add a
`stateDir` plugin option.

Back up the OpenClaw session state, the plugin's `roder-acp` directory, and
Roder's configured persistent data/config/credential locations together. The
plugin metadata maps OpenClaw owners to Roder session ids; it does not replace
the underlying Roder transcript/database. Stop the gateway before copying or
restoring these files, preserve ownership and restrictive permissions, and
restart with the same user and compatible executable.

Do not run multiple gateways against the same plugin state directory. Atomic
record writes do not provide multi-process gateway ownership or distributed
locking. Do not hand-edit a record to move another user's Roder conversation
into your session.

## Development and release

```sh
npm test
npm pack --dry-run
```

Optional integration checks are separate from the offline unit suite:

```sh
RODER_TEST_BINARY=/absolute/path/to/roder npm run test:integration
```

That runs the real Roder lifecycle test with its offline mock provider. The
host test skips unless an installed host package is supplied. To qualify the
actual pinned host without installing it in this checkout or touching your
gateway:

```sh
host="$(mktemp -d)"
npm install --prefix "$host" --ignore-scripts --omit=optional \
  --no-audit --no-fund openclaw@2026.10.9
OPENCLAW_TEST_PACKAGE="$host/node_modules/openclaw" \
  RODER_TEST_BINARY=/absolute/path/to/roder npm run test:host
```

Use a supported Node version for these commands. The host test packs and
installs the local plugin into a temporary consumer, resolves its native entry
through the real host package, starts/stops an actual runtime through the real
SDK registry, and runs CLI installation/discovery/config validation with a
temporary HOME and state directory. `RODER_TEST_BINARY` optionally adds a real
ACP doctor probe with isolated mock-provider config. It does not start a gateway,
log in, access a real model provider, or validate end-to-end channel messaging.

For stronger end-to-end evidence, explicitly enable the separate gateway test:

```sh
OPENCLAW_TEST_PACKAGE="$host/node_modules/openclaw" \
  RODER_TEST_BINARY=/absolute/path/to/roder RODER_TEST_GATEWAY=1 \
  npm run test:gateway
```

This starts **only a test-owned, temporary loopback gateway**, on an ephemeral
port with a random local token, using the packed plugin and an isolated mock
Roder installation. Through the real host's public authenticated client it
spawns an ACP session, sends two prompts, checks exact `hello from roder`
assistant replies in durable chat history, and checks ACP status. It asserts
that an owned loopback guard receives **zero native-inference fallback requests**.
The test-only custom model metadata exists solely to satisfy host admission
without contacting a real provider; it is not recommended production model
configuration.

The admin scope needed for internal `/acp` management is requested through the
public client handshake with the test gateway's token, not forged in command
parameters. The test does not use a user's existing gateway or credentials.
Its environment excludes ambient API keys/channel settings; catalog refresh,
updates, heartbeat cadence, browser, and cron are disabled. Cleanup terminates
the test's detached process group and removes its temporary configuration and
state. This test is skipped unless all three opt-ins above are supplied, even
when running `test:integration`; external channel delivery and real providers
remain outside its scope.

There are no runtime npm dependencies and no build step. The native plugin
entrypoint imports the host's public SDK; the separate `./runtime` export is
usable without importing the OpenClaw host. `./plugin-factory` exposes
`createRoderPlugin` for registry/lifecycle unit tests, not a substitute host.
Live provider/gateway qualification is separate from the offline unit suite.

The package participates in the repository's `roder-openclaw` knope release
stream. Publishing requires the npm package's initial creation and a trusted
publisher configured for `.github/workflows/openclaw-release.yml` in
`RoderAI/roder`; until then install a locally packed tarball. Release CI verifies
the `roder-openclaw/v<version>` tag before publishing with provenance.

For the alternative existing ACPx integration, see
[Use Roder with ACPx and OpenClaw](../../docs/openclaw-acp.md). Choose one
`acp.backend`: `roder` for this package, or `acpx` for that guide.
