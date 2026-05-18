# Roder Plugin Marketplaces

Roder can track plugin marketplaces for Claude, Cursor, Codex, and custom local catalogs. The marketplace store lives at `~/.roder/marketplaces.json` by default. Tests and scripted runs can override it with `RODER_MARKETPLACES_PATH`.

## Defaults

The baked-in marketplace descriptors are:

| ID | Kind | Source |
| --- | --- | --- |
| `claude-plugins-official` | Claude | `anthropics/claude-plugins-official` |
| `cursor-plugins` | Cursor | `cursor/plugins` |
| `codex-plugins` | Codex | `openai/plugins` |

Install one default marketplace:

```sh
roder marketplace install-default cursor
```

Install all built-in default marketplaces:

```sh
roder setup marketplaces --defaults all
```

## Custom Marketplaces

Add a local marketplace catalog:

```sh
roder marketplace add local-cursor --kind cursor --path /path/to/marketplace --name "Local Cursor"
roder marketplace refresh local-cursor
```

Add remote catalogs:

```sh
roder marketplace add team-claude --kind auto --github owner/plugins --name "Team Claude"
roder marketplace add team-git --kind cursor --git https://github.com/owner/plugins.git --ref main
roder marketplace add team-json --kind cursor --http-json https://example.test/marketplace.json
```

Claude and Cursor catalogs read `.claude-plugin/marketplace.json` or `.cursor-plugin/marketplace.json`. Codex catalogs scan plugin directories for `.codex-plugin/plugin.json`.

Remote source checkouts and downloaded JSON catalogs are cached under `~/.roder/marketplaces/cache` unless `RODER_MARKETPLACE_SOURCE_CACHE_DIR` is set. Tests can point GitHub shorthand at local fixtures with `RODER_MARKETPLACE_GITHUB_FIXTURE_DIR`; live marketplace checks remain opt-in with `RODER_MARKETPLACE_LIVE=1`.

Marketplace ids and plugin ids must be lowercase slugs. Custom marketplace ids
must be unique, GitHub sources must use `owner/repo`, and remote/catalog path
fields may not contain parent-directory traversal.

## Search And Install

Search de-duplicates entries that multiple providers expose by repository,
homepage plus normalized name, or provider-local plugin slug. Weak name-only
matches stay as separate rows and appear as related candidates. Each row carries
a recommended variant key while still preserving exact provider variants for
selected-variant or all-variants installs.

```sh
roder marketplace search repo
roder plugin preview local-cursor repo-tools
roder plugin install local-cursor repo-tools
roder plugin install local-cursor repo-tools --all-variants
roder plugin disable local-cursor:repo-tools
roder plugin list
```

The Ctrl+P command palette includes a `Marketplaces` source with default
marketplace metadata, one/all default install rows, a custom add row, refresh,
search, plugin preview, selected install, all-variants install, installed list,
disable, and uninstall actions. The provider picker also includes a plugin
browser dialog with a list/details split, source/component/risk preview, and a
two-press confirmation for all-variants install. The chat composer slash-command
registry exposes `/marketplace` and `/plugin` suggestions with argument hints
for the same flows.

Installed plugin variants enter workflow import scans as source-attributed
plugin items. Passive skills and commands can be enabled through the existing
workflow import path; MCP servers, hooks, apps, LSP servers, npm packages,
scripts, and binaries remain approval-gated.

## Troubleshooting

- Invalid marketplace JSON: run `roder marketplace refresh <id>` and fix the reported catalog path or JSON parse error.
- Unsupported source kinds: use `--path`, `--github`, `--git`, or `--http-json`; plugin-level unsupported sources stay visible as high-risk preview metadata.
- Private repositories: refresh uses the local `git` credential setup. Authenticate with Git outside Roder first, then rerun refresh.
- Unpinned sources: pass `--ref <branch|tag|sha>` when adding GitHub or git marketplaces if repeatable refreshes matter.
- Hash mismatches or stale installed plugins: refresh the marketplace, preview the plugin, then reinstall or uninstall the affected variant key.

## App-Server Methods

| Method | Purpose |
| --- | --- |
| `marketplaces/list` | List configured marketplaces, including baked-in defaults. |
| `marketplaces/install_default` | Install `anthropic`, `cursor`, `codex`, `all`, or `none` default marketplace descriptors. |
| `marketplaces/add` | Add a custom local, GitHub, git URL, or HTTP JSON marketplace descriptor. |
| `marketplaces/remove` | Remove a custom marketplace or disable a baked-in default descriptor. |
| `marketplaces/refresh` | Read and normalize one marketplace catalog. |
| `marketplaces/search` | Search normalized plugin entries and return de-duplicated results. |
| `marketplaces/plugin` | Return one marketplace plugin variant. |
| `plugins/preview_install` | Return normalized install metadata and risk hints. |
| `plugins/install` | Record an installed marketplace plugin variant in the local cache. |
| `plugins/install_all_variants` | Install every variant in the selected plugin's de-duplicated group. |
| `plugins/list_installed` | List installed plugin variants. |
| `plugins/disable` | Mark an installed plugin variant disabled without removing its record. |
| `plugins/uninstall` | Remove an installed plugin variant record. |
