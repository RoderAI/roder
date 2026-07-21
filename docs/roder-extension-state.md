# Roder Extension State

Roder extension state is host-owned and extension-scoped. Extensions describe their own state with an `ExtensionStateCodec`; the host stores opaque `ExtensionStateRecord` values and only understands the extension id, key, scope, schema version, and serialized JSON value.

Scopes are explicit:

- `Global`: process-wide extension data.
- `Workspace`: data tied to a workspace root.
- `Thread`: data tied to a conversation thread.
- `Turn`: data tied to one turn in a thread.

Thread stores persist records alongside the thread snapshot. The JSONL store writes records to `extension_state.jsonl` and returns them through `ThreadSnapshot.extension_states`, which means app-server clients can inspect state metadata through `thread/read` without decoding private extension payloads. The PostgreSQL session store stores the same opaque records in its tenant-scoped extension-state table.

Schema changes are handled by the extension. `ExtensionStateCodec::decode_state` accepts the current schema version directly; when it sees an older version it calls `migrate_state`. The migration must return a record using the codec's current schema version, or decoding fails.

## Turn lifecycle records

The host-owned `roder.lifecycle` extension persists append-only, versioned
turn-lifecycle records. A lifecycle record is scoped to one turn and contains
the thread id, turn id, lifecycle state, cleanup state, cleanup ownership,
optional reason, and timestamp. The app-server projects the latest valid record
for each turn in `thread/read.lifecycle` and streams each runtime lifecycle
transition through `turn/lifecycleUpdated`.

Lifecycle states are `running`, `interrupt_requested`, `interrupted`,
`completed`, `failed`, and `recovery_needed`. Records in `running` or
`interrupt_requested` are treated as needing reconciliation after a restart;
when no matching local turn is live, loading that thread records
`recovery_needed` with reason `runtime_restart`.

Lifecycle history remains append-only. Consumers should use the latest valid
timestamped record per turn rather than assuming the last physical record is
valid. JSONL loading skips malformed extension-state lines; PostgreSQL loading
does the same for malformed extension-state rows. Both stores record a redacted
in-memory corruption marker. `thread/read.lifecycle.corruptRecordCount` reports
the count; malformed raw values are not exposed. A valid lifecycle record with
an unsupported schema version or inconsistent turn scope is also counted as
invalid for the lifecycle snapshot, without preventing valid records for other
turns from being returned.

`ownership` is intentionally a small redacted proof level rather than process
metadata:

- `runtime_task_only`: Roder observed its own async turn task only. It has no
  provider child-process or remote-job reaping acknowledgement.
- `provider_cleanup_pending`: a provider registered cleanup ownership, but
  Roder has not observed its completion acknowledgement yet.
- `provider_cleanup_confirmed`: the provider reported its owned cleanup path
  complete to Roder.

Clients must not infer an OS PID, command line, remote job identifier, or a
broader host-process guarantee from this field.
