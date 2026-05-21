# Roder Secure Codebase Indexing

Roder's code index is a local-only semantic codebase index. It is built from the current workspace, stored under the Roder data directory, and exposed through app-server methods that report generation and proof state with every result.

## Storage And Trust

- Default storage is `$HOME/.roder/code-index/<workspace-key>/code-index.sqlite3`.
- `RODER_CODE_INDEX_HOME` can override the base directory for tests or isolated runs.
- Index builds respect workspace path scope and ignore `.git`, `.roder`, `target`, and `node_modules`.
- The current implementation never uploads index data and never shares source chunks.
- Future shared-index reuse must keep the same trust model: clients can reuse metadata only when local content proofs match the workspace root hash and chunk content hash.

## App-Server Methods

- `index/status`: returns current status, store path, generation id, root hash, staleness, stats, and a message when unavailable or stale.
- `index/rebuild`: rebuilds the local index for the workspace and publishes `index/statusChanged`.
- `index/search`: searches the current local index and returns a `CodeIndexSearchResponse` with generation and proof-filter metadata.
- `index/readChunk`: reads a paginated source chunk. It requires `includeSource=true` and clamps page size to a bounded limit.
- `index/proofs/list`: lists local possession proofs for indexed chunks.

`index/search` includes the index generation in every response. If the workspace changed since the index was built, the status and response generation report `stale` so clients can explain that results came from an older local generation.

## Source Reads

Search responses do not include raw source by default. Source is only returned by `index/readChunk`, which:

- Requires an explicit `includeSource=true` request.
- Resolves the chunk path under the current workspace before reading.
- Returns a bounded page with `offset`, `limit`, `totalBytes`, and `hasMore`.
- Rejects unknown chunk hashes and paths that no longer match the workspace authority.

## Embeddings

Normal tests use fake local embeddings and SQLite cache reuse. Live embedding providers are not required for this phase. Future live checks must remain opt-in behind `RODER_CODE_INDEX_LIVE_EMBEDDINGS=1`.
