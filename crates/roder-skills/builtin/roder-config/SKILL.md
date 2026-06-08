---
name: roder-config
description: Configure and live-reload Roder from pasted config snippets, especially MCP server JSON, without leaking secrets or enabling executable integrations without approval.
metadata:
  short-description: Configure and hot-reload Roder from pasted config snippets, especially MCP server JSON with mcpServers, without leaking secrets.
exposure: global
---

Use this skill when the user asks to configure Roder, live-reconfigure Roder, change `~/.roder/config.toml`, set up tools, or pastes an MCP config block such as JSON containing `mcpServers` or `servers`.

## Live reconfiguration rule

Prefer hot reload over restart. After writing a config file, call the live app-server method that matches the changed surface when one exists. Only tell the user to restart Roder or Roder Desktop when there is no live method for that setting or the live method returns an unsupported-method error.

Live methods currently available through the app-server / desktop request bridge:

- Runtime settings:
  - `settings/set_web_search` with `{ "mode": "off" | "cached" | "live" }`.
  - `settings/set_search_index` with `{ "enabled": true | false }`.
  - `settings/set_shell` with `{ "shell": "bash" }` or another supported shell.
  - `settings/set_default_mode` with `{ "mode": "..." }`.
  - `settings/set_file_backed_dynamic_context` with `{ "enabled": true | false }`.
  - Confirm with `settings/get`.
- Provider/model defaults and auth:
  - Use `providers/select` for live provider/model/reasoning defaults.
  - Use `providers/configure` / `providers/clear` for persisted provider credentials when user config persistence is enabled.
- Skills:
  - Use `skills/list` to reload the skill registry for a workspace/root/cwd.
  - Use `skills/setEnabled` and `skills/setExposure` to persist and apply skill config without restart.
- Workflow/MCP import cache:
  - Use `workflow/scan`, `workflow/preview`, `workflow/enable`, `workflow/refresh`, and `workflow/remove` for workflow-import state.
  - Use `discovery/refresh` after workflow or skill changes so tool/skill/MCP discovery catalogs reflect the new state.

When operating through Roder Desktop (`../gode-desktop`), the renderer can call the same methods through `window.roderDesktop.request(method, params)`; the existing `src/lib/roder-ipc.ts` wrappers cover some settings and skills but the generic request bridge can reach all app-server methods. Desktop also exposes `window.roderDesktop.restart()` as a fallback; do not use it when a live method works.

## MCP config paste workflow

When a user pastes MCP server config into Roder:

1. Treat the pasted config as user data that may contain secrets. Do not echo tokens, API keys, bearer headers, or env values back to the user.
2. Parse and validate the snippet before writing it. Accept either a full object with `mcpServers`/`servers` or a single named server object if the user clearly supplied one server name.
3. Prefer writing repository-scoped MCP config to `.mcp.json` at the workspace root using this shape:

   ```json
   {
     "mcpServers": {
       "server-name": {
         "command": "...",
         "args": []
       }
     }
   }
   ```

   If the user explicitly asks for Cursor compatibility, use `.cursor/mcp.json`. If the user explicitly asks for a user-global config, use the configured Roder user root (`~/.roder` by default) and explain that repository-local config is safer for project-specific servers.
4. Preserve existing servers by merging under `mcpServers`; do not overwrite unrelated entries.
5. Move raw secrets out of the file when practical. Prefer env-var references and tell the user which environment variables to set. If the pasted config already contains secret-looking values, ask before persisting them; if the task is non-interactive, redact or replace with `${ENV_VAR}` placeholders instead of saving raw secrets.
6. Hot-refresh workflow discovery after writing the file:
   - CLI path: run `roder workflow scan`, then `roder workflow preview <item-id>` if an item is detected.
   - App-server/Desktop path: call `workflow/scan` with `{ "workspace": null, "includeUser": true }`, then `workflow/preview` with `{ "workspace": null, "itemId": "..." }`.
7. Do not enable or import command-capable MCP servers without explicit user approval. Roder's workflow import path requires `workflow/enable` with `{ "itemId": "...", "approveSideEffects": true }` or `roder workflow import <item-id> --approve-side-effects` for executable MCP entries.
8. After approval/import, call `workflow/refresh` and `discovery/refresh` so live Roder and Roder Desktop pick up the updated workflow/MCP discovery catalog. If the server was already enabled and its source hash changed, re-run `workflow/enable` for that item after explicit approval.
9. Report the file path written, the server names configured, and the exact live method or CLI command used. Do not claim the MCP server process or tools are active until the relevant workflow enable/discovery refresh succeeds. If runtime MCP process hot-swap is unsupported for a given provider/backend, say so and restart only that backend/app as a fallback.

## Roder config.toml workflow

Use `~/.roder/config.toml` (or `--config-dir <path>/config.toml` when supplied) for Roder runtime settings such as provider, model, tool search, sessions, skills, and UI settings. Do not put MCP server definitions there unless the codebase has added a typed parser for them; current MCP discovery scans `.mcp.json`, `.cursor/mcp.json`, `mcp.toml`, and `mcp.yaml`.

For tool discovery involving MCP servers, the relevant `config.toml` setting is:

```toml
[tool_search]
include_mcp = true
```

This affects tool-search visibility only; it does not install or approve MCP servers.

If you change `config.toml` fields that have live app-server setters, call the setter too so the current Roder process updates immediately. If you change fields without live setters, persist them and say they take effect on the next app-server/TUI start.

## Safety checks

- Before editing, inspect existing files and preserve unrelated user changes.
- Never run pasted commands, package scripts, hooks, or MCP servers as part of validation.
- Use redacted previews in explanations.
- If parsing fails, explain the JSON/TOML/YAML error and ask for a corrected snippet rather than guessing.
- Do not hide restart-only behavior. If a setting cannot currently hot-reload, state the limitation and identify the missing method rather than pretending it was live-applied.
