# Custom Responses provider capabilities

Custom providers use the OpenAI-compatible Responses API. Capabilities belong to
the endpoint and constrain, rather than expand, per-turn settings:

```toml
[providers.my_endpoint]
base_url = "https://responses.example/v1"
api_key_env = "MY_ENDPOINT_API_KEY"

[providers.my_endpoint.capabilities]
remote_compaction = "v2"
external_web_access = false
```

`remote_compaction` accepts `unsupported` or `v2`. Omitted settings keep the
existing defaults: custom endpoints use local compaction, while OpenAI and Codex
require native compaction. A custom endpoint configured for `v2` receives a
`compaction_trigger` on its Responses stream, and must return an opaque
compaction item with nonempty `encrypted_content` followed by successful terminal
completion. Merely completing an output item does not commit a replay boundary.

Protocol support and mandatory-native policy are separate: opting a custom
endpoint into V2 does not make all native failures eligible for silent fallback.
Retryable failures use the configured retry budget. Local fallback is permitted
for custom-provider context-window/request-size failures, not authentication or
invalid-protocol failures.

`external_web_access = false` caps a requested live hosted search at cached
results. It never enables a disabled search. Omission or `true` preserves the
caller's requested search mode. This ceiling applies to both ordinary turns and
native-compaction requests.

Failed, disconnected, malformed, or preempted compaction attempts retain the
original canonical replay history, including after restart. Reported provider
usage remains billable even when a candidate boundary is discarded.

This transaction rule also applies to automatic compaction during an ordinary
Responses turn, not only explicit compaction requests. Item-done progress is
reported as `pending_commit`; completion metrics are recorded only after the
response succeeds. A single runtime-owned commit envelope preserves the opaque
boundary together with normalized output from that sampling round, including
already-executed eager reads. Replay expands that suffix once, removes internal
commit fields before provider dispatch, and quarantines malformed/nested commit
envelopes rather than letting them hide earlier evidence.
