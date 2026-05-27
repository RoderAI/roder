# Roder Memories

Roder stores local memories through the `roder-ext-memory` extension. The default store is a SQLite database under `~/.roder/memory/memories.sqlite3`, unless `[memories].store_path` or `RODER_MEMORIES_PATH` points elsewhere.

## Scopes

- `global`: user-wide memory shared across projects.
- `project:<id>`: project memory, normally resolved from the active workspace.
- `workspace:<id>` and `thread:<id>` remain valid API scopes for extension authors.

## CLI

```sh
roder memory list --scope project
roder memory query "text" --scope project --include-global
roder memory save "text" --scope global
roder memory read <id>
roder memory update <id> "new text"
roder memory delete <id>
roder memory providers list
roder memory providers set openai --model text-embedding-3-large
roder memory reembed --scope project --provider openai --model text-embedding-3-large
```

## App-Server

Clients should use the app-server instead of opening SQLite directly:

- `memory/list`
- `memory/read`
- `memory/save`
- `memory/update`
- `memory/delete`
- `memory/query`
- `memory/provider/list`
- `memory/provider/set`
- `memory/recall/preview`

Memory events are streamed as `memory/saved`, `memory/updated`, `memory/deleted`, `memory/queried`, `memory/recallReady`, `memory/reembedQueued`, `memory/providerChanged`, and `memory/observationRecorded`.

## Embeddings

The provider-neutral embedding contract lives in `roder-api`. `roder-ext-openai-embeddings` registers provider id `openai` and defaults to `text-embedding-3-large`. Live OpenAI checks are gated behind:

```sh
RODER_LIVE_EMBEDDINGS=1 cargo test -p roder-ext-openai-embeddings live -- --ignored
```

Normal tests use deterministic fake vectors and do not require network or secrets.

## Config

```toml
[memories]
store_path = "~/.roder/memory/memories.sqlite3"
embedding_provider = "openai"
embedding_model = "text-embedding-3-large"
project_enabled = true
global_enabled = false
include_global_with_project = false

[embedding_providers.openai]
enabled = true
api_key_env = "OPENAI_API_KEY"
model = "text-embedding-3-large"

[embedding_providers.local]
enabled = true
command = ["local-embedder", "--json"]
dimensions = 384
```

Custom embedding providers can be installed as Roder extensions that implement `EmbeddingProvider`, or configured as command-backed providers once a command embedding extension is installed.

## Migration

When `memories.jsonl` exists beside the SQLite database, `roder-ext-memory` imports it once and writes `.memories-jsonl-imported` to avoid duplicate active rows. Imported records keep their scope, metadata, and timestamps, then receive deterministic embeddings.

## Privacy

Project memories and global memories are local by default. Selecting OpenAI embeddings sends memory text to the OpenAI embeddings API. Use a local command provider or another embedding extension for fully local embedding.

