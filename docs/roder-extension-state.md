# Roder Extension State

Roder extensions can persist small, versioned state records through the session store. The host treats each record value as extension-owned JSON and uses the record key to keep process-scoped and thread-scoped state separate.

## Record Shape

```json
{
  "key": {
    "extension_id": "roder-tui",
    "scope": {
      "type": "thread",
      "thread_id": "thread-id"
    },
    "key": "transcript_fold"
  },
  "schema_version": 1,
  "value": {
    "collapsed_messages": [],
    "collapsed_tool_calls": ["call-1"]
  },
  "updated_at": "2026-05-16T00:00:00Z"
}
```

`schema_version` belongs to the extension's `ExtensionStateCodec`. Extensions should migrate old values in the codec before decoding.

## App-Server Methods

`extension_state/set` stores or replaces one record:

```json
{
  "jsonrpc": "2.0",
  "id": "save-fold-state",
  "method": "extension_state/set",
  "params": {
    "record": {
      "key": {
        "extension_id": "roder-tui",
        "scope": { "type": "thread", "thread_id": "thread-id" },
        "key": "transcript_fold"
      },
      "schema_version": 1,
      "value": { "collapsed_tool_calls": ["call-1"] },
      "updated_at": "2026-05-16T00:00:00Z"
    }
  }
}
```

Result:

```json
{ "saved": true }
```

`extension_state/get` loads one record by key:

```json
{
  "jsonrpc": "2.0",
  "id": "load-fold-state",
  "method": "extension_state/get",
  "params": {
    "key": {
      "extension_id": "roder-tui",
      "scope": { "type": "thread", "thread_id": "thread-id" },
      "key": "transcript_fold"
    }
  }
}
```

Result:

```json
{ "record": null }
```

When using the default JSONL session store, thread-scoped records are also included in `sessions/load` snapshots under `extension_state`.

## Verification

```sh
cargo test -p roder-api extension_state_record_round_trips_json
cargo test -p roder-ext-jsonl-session extension_state
cargo test -p roder-app-server --test e2e extension_state_round_trips_through_app_server_session_store
```
