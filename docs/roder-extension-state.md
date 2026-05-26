# Roder Extension State

Roder extension state is host-owned and extension-scoped. Extensions describe their own state with an `ExtensionStateCodec`; the host stores opaque `ExtensionStateRecord` values and only understands the extension id, key, scope, schema version, and serialized JSON value.

Scopes are explicit:

- `Global`: process-wide extension data.
- `Workspace`: data tied to a workspace root.
- `Thread`: data tied to a conversation thread.
- `Turn`: data tied to one turn in a thread.

Thread stores persist records alongside the thread snapshot. The JSONL store writes records to `extension_state.jsonl` and returns them through `ThreadSnapshot.extension_states`, which means app-server clients can inspect state metadata through `thread/read` without decoding private extension payloads.

Schema changes are handled by the extension. `ExtensionStateCodec::decode_state` accepts the current schema version directly; when it sees an older version it calls `migrate_state`. The migration must return a record using the codec's current schema version, or decoding fails.
