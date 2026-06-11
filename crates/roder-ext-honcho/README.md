# roder-ext-honcho

Memory store backend backed by [Honcho](https://docs.honcho.dev). Implements
`MemoryStore` against Honcho's v3 REST API and reuses the shared memory
context provider and memory tools, so it is a drop-in replacement for the
sqlite memory extension.

Retrieval is delegated to Honcho's hosted semantic search; the store never
embeds locally and ignores `MemoryQuery::provider_id` / `model`. Honcho does
not return similarity scores, so result scores are rank-derived.

## Configuration

The api key is read from the environment only and is never written to config
files. `HonchoMemoryConfig::from_env()` resolves:

| Field | Env var | Default |
| --- | --- | --- |
| `api_key` | `HONCHO_API_KEY` | required |
| `base_url` | `HONCHO_BASE_URL` | `https://api.honcho.dev` |
| `workspace_id` | `HONCHO_WORKSPACE_ID` | required |
| `peer_id` | `HONCHO_PEER_ID` | `roder-memory` |
| `session_id` | `HONCHO_SESSION_ID` | unset (one session per scope) |

`HonchoMemoryConfig`'s `Debug` impl redacts the api key, and error bodies are
scrubbed of secret-looking markers before they can reach logs.

Backend selection lives in roder config (`roder-extension-host` wiring):

```toml
[memories]
backend = "honcho"

[memories.honcho]
workspace_id = "my-workspace"
# base_url = "https://honcho.internal"   # self-hosted override
# peer_id = "roder-memory"
# session_id = "pinned-session"
# api_key_env = "MY_HONCHO_KEY"          # env var NAME holding the key
```

`RODER_MEMORY_BACKEND=honcho` overrides `[memories] backend`. Embedding hosts
can instead install a configured `HonchoMemoryExtension` through
`set_distribution_extensions` to choose ids per runtime.

## Scope mapping

One roder runtime maps to one Honcho workspace (`workspace_id`). Memories are
stored as Honcho messages authored by `peer_id`, and every read path (search,
list, and direct id lookup) is constrained to messages authored by that peer —
multiple runtimes can share a workspace under distinct peer ids without
reading each other's records.

| Roder scope | Honcho session |
| --- | --- |
| `Global` | `roder-memory-global` |
| `User(id)` | `roder-memory-user-<id>` |
| `Workspace(id)` | `roder-memory-workspace-<id>` |
| `Project(id)` | `roder-memory-project-<id>` |
| `Thread(id)` | `roder-memory-thread-<id>` |

Scope values are sanitized to Honcho's id alphabet (`[A-Za-z0-9_-]`);
collisions after sanitization are harmless because the exact scope travels in
message metadata (`roder_scope`) and every read path filters on it. When
`session_id` is set, all scopes write into that single session instead and
scope separation relies on the metadata filter alone.

Memory ids are `{honcho_session_id}/{honcho_message_id}`.

## Record encoding

Each `MemoryRecord` is one Honcho message: `content` holds the memory text
and message metadata carries the roder envelope (`roder_memory`,
`roder_scope`, `roder_content_hash`, `roder_metadata`, `roder_deleted`,
`roder_created_at`, `roder_updated_at`).

- Honcho message content is immutable, so updating a memory writes a new
  message and tombstones the old one with a `roder_superseded_by` pointer;
  `get`/`delete` follow the pointer chain (bounded) to the live head, so
  stale ids keep resolving after updates.
- Deletes are soft (metadata tombstone), mirroring the sqlite store.
- `MemoryRecord::usage` is not tracked; recording per-search usage would
  require a metadata write per result.

## Tests

`tests/mock.rs` runs the store lifecycle against a stateful fake Honcho
server. `tests/live.rs` is gated on `RODER_LIVE_HONCHO=1` plus
`HONCHO_API_KEY` and runs the same lifecycle against the real API in a
scratch workspace (deleted afterwards); searches are retried because Honcho
indexes messages asynchronously.
