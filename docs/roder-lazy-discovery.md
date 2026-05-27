# Roder Lazy Discovery

Roder mirrors tools, workflow imports, MCP servers, skills, commands, plugins, subagents, and artifact inspection tools into a file-backed discovery catalog. The base prompt keeps only compact discovery guidance, while `discovery.list`, `discovery.search`, and `discovery.read` let the agent inspect detailed schemas or instructions when needed.

## Storage

Catalog files live under:

```text
~/.roder/discovery/
```

Tests and controlled runs can override this with:

```sh
RODER_DISCOVERY_CATALOG_DIR=/tmp/roder-discovery/catalog
RODER_DISCOVERY_STATE_DIR=/tmp/roder-discovery/state
```

Promoted capability state is thread-scoped and stored under:

```text
~/.roder/threads/discovery-state/discovery/promotions.json
```

The catalog never stores secret values. Auth and redaction state are represented as metadata such as `authState = "required"` and redacted field paths.

## Model Tools

- `discovery.list`: list compact capability summaries.
- `discovery.search`: search by item id, name, title, description, tags, or hints.
- `discovery.read`: read bounded detailed schema or instruction content and promote that item for the thread.

Core coding tools remain statically available in this rollout. Lazy discovery primarily gives large MCP, skill, command, plugin, subagent, and artifact surfaces a low-token discovery path.

## App-Server Methods

- `discovery/refresh`: rebuild the catalog from the installed extension registry and workflow-import scan.
- `discovery/groups`: list catalog groups and compact item metadata.
- `discovery/search`: search catalog items.
- `discovery/read`: read bounded detail content and optionally promote the item.
- `discovery/promote`: mark an item promoted without reading it again.
- `discovery/promoted/list`: list promoted thread items.
- `discovery/promoted/clear`: clear promoted state by thread, item, or both.

Read responses are line-bounded and never return arbitrary filesystem paths. Schema and instruction paths are resolved relative to the catalog root only.

## Contributor Notes

Normal tests use fake catalogs and local workflow-import fixtures. No live MCP server is required. When adding a new catalog source, emit compact summaries first and write detailed schemas, instructions, or manifests as catalog files that `discovery.read` can page through.
