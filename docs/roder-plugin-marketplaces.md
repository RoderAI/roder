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

## Local Marketplaces

Add a local marketplace catalog:

```sh
roder marketplace add local-cursor --kind cursor --path /path/to/marketplace --name "Local Cursor"
roder marketplace refresh local-cursor
```

Claude and Cursor catalogs read `.claude-plugin/marketplace.json` or `.cursor-plugin/marketplace.json`. Codex catalogs scan plugin directories for `.codex-plugin/plugin.json`.

## Search And Install

Search de-duplicates entries that multiple providers expose by repository, homepage plus normalized name, or provider-local plugin slug.

```sh
roder marketplace search repo
roder plugin preview local-cursor repo-tools
roder plugin install local-cursor repo-tools
roder plugin list
```

The Ctrl+P command palette includes a `Marketplaces` source with default marketplace metadata, install-all-defaults, refresh, and marketplace search actions.

## App-Server Methods

| Method | Purpose |
| --- | --- |
| `marketplaces/list` | List configured marketplaces, including baked-in defaults. |
| `marketplaces/install_default` | Install `anthropic`, `cursor`, `codex`, `all`, or `none` default marketplace descriptors. |
| `marketplaces/add` | Add a custom local marketplace descriptor. |
| `marketplaces/refresh` | Read and normalize one marketplace catalog. |
| `marketplaces/search` | Search normalized plugin entries and return de-duplicated results. |
| `marketplaces/plugin` | Return one marketplace plugin variant. |
| `plugins/preview_install` | Return normalized install metadata and risk hints. |
| `plugins/install` | Record an installed marketplace plugin variant in the local cache. |
| `plugins/list_installed` | List installed plugin variants. |
| `plugins/uninstall` | Remove an installed plugin variant record. |
