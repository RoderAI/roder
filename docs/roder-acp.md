# Integrate Roder through ACP

Launch `roder acp` and exchange newline-delimited JSON-RPC over stdin/stdout.
Diagnostics go to stderr. Use `initialize` to discover the running binary's
capabilities; published older binaries may lack the features described here.

Roder supports ACP v1 text and image prompts, resource links, streamed output,
tool activity, cancellation and permission requests. The CLI uses a durable
thread store. Load, resume and close use the existing
[session recovery contract](app-server/api.md#acp-session-recovery).

## Session controls

New, loaded and resumed sessions return `modes` and `configOptions`. The `model`
select option lists registered providers' models with values such as
`codex/gpt-6-sol`. Select exactly one advertised value:

```json
{"jsonrpc":"2.0","id":3,"method":"session/set_config_option","params":{"sessionId":"thread-123","configId":"model","value":"codex/gpt-6-sol"}}
```

The result contains the updated `configOptions`; Roder also sends a
`config_option_update`. Model choices are stored on the native thread. Provider
authentication is still required, and image support depends on the selected
provider and model.

```json
{"jsonrpc":"2.0","id":4,"method":"session/set_mode","params":{"sessionId":"thread-123","modeId":"plan"}}
```

Mode IDs are `default`, `accept_all`, `plan` and `bypass`. They follow Roder's
[permission policies](app-server/api.md#threadset_mode). `accept_all` approves
built-in coding tools; injected MCP calls still ask for permission. `plan`
rejects all injected MCP calls because their side effects are unknown. `bypass`
executes them without an ACP permission request. Stdio server startup itself is
authorized by the client's supplied MCP configuration and occurs during setup.

The result is `{}`; Roder also sends a `current_mode_update` notification. Model and
mode changes require an idle session. Modes persist independently for each
thread; changing one session does not change the runtime default or another
session's permissions. Team restrictions retain precedence.

## Inject MCP tools

Supply stdio or streamable HTTP servers in `mcpServers` on new/load/resume:

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "session/new",
  "params": {
    "cwd": "/absolute/project",
    "mcpServers": [
      {"name":"local","command":"example-mcp","args":[],"env":[]},
      {"type":"http","name":"remote","url":"http://127.0.0.1:9000/mcp","headers":[]}
    ]
  }
}
```

Server names must be unique nonempty ASCII letters, digits, `_` or `-`. MCP
tools become `mcp__<server>__<tool>` native external tools. Tool names may also
contain `.`. The adapter discovers tool schemas before returning the session;
connection or discovery failures fail the request and clean up opened clients.
Tool call failures become tool error results in the conversation.

Stdio servers run in the session's working directory. They inherit a small
environment for executable lookup, home, locale and temporary files; explicit
ACP `env` entries override those values. Explicit values are redacted from stdio
tool text and diagnostics. HTTP headers are forwarded to the server.

Credentials and launch configuration stay in the host's configuration. Only
tool specifications are stored on the thread. Restore must resupply servers
that expose exactly the stored tool names, descriptions and schemas; omission or
changed bindings return `-32602`. This prevents a restored model from calling
tools with stale definitions. The host owns the identity and authorization of
the supplied servers. Session close and orderly stdin EOF shut down clients;
HTTP servers receive session DELETE when they supplied a session ID.

SSE transport, MCP resources and MCP prompts are not exposed by this adapter.

## Images and saved sessions

Image blocks accept base64 PNG, JPEG, WebP and GIF with their matching
`mimeType`. Invalid base64 and unsupported MIME types return `-32602`. Images
enter the native transcript as data URLs. Replayed history preserves image URLs
as user resource links. A text-only provider reports the native image-input
error and cannot interpret the image.

```json
{"jsonrpc":"2.0","id":5,"method":"session/list","params":{"cwd":"/absolute/project"}}
```

Listing reads the native thread store, including sessions from earlier
processes. It returns `sessions` with IDs, cwd, optional titles and update times,
plus an optional `nextCursor`. Pass that cursor unchanged for the next page.
The native page size is 50. Cwd filtering occurs within each page, so an empty
filtered page can still have a next cursor. Omit cwd to list all workspaces.

## Paseo host instructions

`session/new` recognizes `_meta._paseo.systemPrompt` as a string and persists it
as native thread developer instructions beneath Roder's harness system prompt.
Workspace `AGENTS.md` instructions continue to apply. This metadata does not
replace Roder's harness or tool policies.

Load/resume preserves the stored instructions. Omit the field or resupply the
same string; a different string requires a new session and returns `-32602`.
Other Paseo metadata, including `persist`, `providerOptions` and `toolPolicy`, is
not interpreted. The CLI continues to use its configured durable store.

Session deletion, additional workspace roots, audio and embedded resource
content remain unsupported. Clients should expose only features advertised by
the running binary and the session's returned controls.
