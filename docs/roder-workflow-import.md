# Roder Workflow Import

Roder can scan an existing repository for agent workflow conventions and show a reviewable import preview before enabling anything with side effects.

## Supported Sources

- `AGENTS.md`, `CLAUDE.md`, and README agent sections become passive context imports.
- `.agents/skills/*/SKILL.md` and user skill roots become skill imports with source attribution.
- `.mcp.json`, `.cursor/mcp.json`, `mcp.toml`, and `mcp.yaml` become MCP server imports.
- `.agents/commands`, `.claude/commands`, and `.roder/commands` markdown files become slash command imports.
- `.codex/hooks.json` and `.cursor/hooks.json` become hook imports.
- `.codex-plugin/plugin.json` and `.agents/plugins/*/plugin.json` become plugin manifest imports.

## Import States

Imports move through explicit states:

```text
detected -> previewed -> enabled -> stale
detected -> ignored
enabled -> disabled
enabled -> removed
```

Every import stores its source path, source type, hash, detected time, risk, and preview payload. Refresh compares the current source hash with the enabled decision and marks changed imports as stale.

## Safety Rules

Passive guidance, skills, and markdown commands can be enabled without approving side effects. MCP servers, hooks, plugins, and other command-capable entries require explicit approval before enablement. Preview output redacts secret-looking keys such as tokens, passwords, API keys, secrets, and environment maps.

The scanner is offline and uses fixture-style local files. It does not start MCP servers, run hooks, execute plugin code, or contact marketplaces.

## Storage

Import decisions are stored under `~/.roder/workflow-imports.json` by default. Tests and controlled environments can override this with `RODER_WORKFLOW_IMPORTS_PATH`.

No workflow import state is written into the scanned workspace, and the scanner must not create a hidden `.roder` directory in the repository.

## App-Server Methods

- `workflow/scan`: scan a workspace and return detected items plus parse errors.
- `workflow/preview`: return one or all preview records and emit preview events.
- `workflow/enable`: enable an item. Command-capable items require `approveSideEffects: true`.
- `workflow/ignore`: record that an item should be skipped.
- `workflow/refresh`: rescan and return enabled imports whose hashes changed.
- `workflow/remove`: record removal for a previously enabled import.

Workflow events are also streamed to clients:

- `workflow/importsDetected`
- `workflow/importPreviewed`
- `workflow/importEnabled`
- `workflow/importDisabled`
- `workflow/importStale`
- `workflow/importFailed`

## CLI

```sh
roder workflow scan
roder workflow preview ITEM_ID
roder workflow import ITEM_ID
roder workflow import ITEM_ID --approve-side-effects
```

`scan` prints a compact table with item id, source type, approval posture, title, and source path. `preview` prints the redacted JSON preview.
