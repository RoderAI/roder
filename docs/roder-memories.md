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
roder memory providers set google --model gemini-embedding-2
roder memory providers set zeroentropy --model zembed-1
roder memory reembed --scope project --provider openai --model text-embedding-3-large
roder memory reembed --scope project --provider google --model gemini-embedding-2
roder memory reembed --scope project --provider zeroentropy --model zembed-1
```

`roder memory reembed` is currently a queue placeholder. New saves, updates,
queries, and recall previews use the selected provider/model immediately.

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

The provider-neutral embedding contract lives in `roder-api`. First-party remote providers are:

- `openai`: provided by `roder-ext-openai-embeddings`, default model `text-embedding-3-large`.
- `google`: provided by `roder-ext-google-embeddings`, default model `gemini-embedding-2`.
- `zeroentropy`: provided by `roder-ext-zeroentropy-embeddings`, default model `zembed-1`.

Google Gemini Embedding 2 uses the Gemini API-key `embedContent` endpoint. Roder resolves the key from `RODER_GOOGLE_EMBEDDINGS_API_KEY`, `GEMINI_API_TOKEN`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `GOOGLE_GENAI_API_KEY`, or `GOOGLE_AI_API_KEY`, in that order. Text retrieval prompts can use Google's documented instruction format, for example `task: search result | query: ...` for queries and `title: ... | text: ...` for documents.

ZeroEntropy `zembed-1` uses `POST /models/embed` on `https://api.zeroentropy.dev/v1` by default. Roder resolves the key from `RODER_ZEROENTROPY_API_KEY` or `ZEROENTROPY_API_KEY`, supports `RODER_ZEROENTROPY_EMBEDDINGS_ENDPOINT` for endpoint override, and sends query/document intent through the provider-neutral embedding contract. Supported dimensions are `2560`, `1280`, `640`, `320`, `160`, `80`, and `40`; the default is `2560`. `encoding_format` can be `base64` or `float`, and `latency` can be `fast`, `slow`, or omitted. Use `https://eu-api.zeroentropy.dev/v1` as the endpoint for EU API keys.
Live checks are gated behind:

```sh
RODER_LIVE_EMBEDDINGS=1 cargo test -p roder-ext-openai-embeddings live -- --ignored
RODER_GOOGLE_EMBEDDINGS_LIVE=1 cargo test -p roder-ext-google-embeddings live -- --ignored
RODER_ZEROENTROPY_EMBEDDINGS_LIVE=1 cargo test -p roder-ext-zeroentropy-embeddings live -- --ignored
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

[embedding_providers.google]
enabled = true
api_key_env = "GEMINI_API_KEY"
model = "gemini-embedding-2"
endpoint = "https://generativelanguage.googleapis.com/v1beta"
dimensions = 3072

[embedding_providers.zeroentropy]
enabled = true
api_key_env = "ZEROENTROPY_API_KEY"
model = "zembed-1"
endpoint = "https://api.zeroentropy.dev/v1"
dimensions = 2560
encoding_format = "base64"
latency = "fast"

[embedding_providers.local]
enabled = true
command = ["local-embedder", "--json"]
dimensions = 384
```

Custom embedding providers can be installed as Roder extensions that implement `EmbeddingProvider`, or configured as command-backed providers once a command embedding extension is installed.

## Migration

When `memories.jsonl` exists beside the SQLite database, `roder-ext-memory` imports it once and writes `.memories-jsonl-imported` to avoid duplicate active rows. Imported records keep their scope, metadata, and timestamps, then receive deterministic embeddings.

## Privacy

Project memories and global memories are local by default. Selecting OpenAI embeddings sends memory text to the OpenAI embeddings API. Selecting Google embeddings sends memory text to the Gemini API. Selecting ZeroEntropy embeddings sends memory text to the ZeroEntropy API unless you deploy a separate self-hosted/local provider. Use a local command provider or another local embedding extension for fully local embedding.
