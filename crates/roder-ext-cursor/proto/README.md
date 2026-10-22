# Cursor `agent.v1` schema reference

`agent_v1.proto` is a partial, reverse-engineered transcription of Cursor's
`agent.v1` protobuf schema. Roder talks to `agent.v1.AgentService/Run` but does
**not** compile this `.proto` — `src/proto.rs` hand-rolls the encoding/decoding.
This file exists so the recovered field numbers are captured in one referenceable
place (for future maintenance or codegen) instead of living only as inline
comments.

## Source of truth

Field numbers come from the Cursor app bundle's generated protobuf-es schema:

```
/Applications/Cursor.app/Contents/Resources/app/extensions/cursor-agent-host/
  dist/main.js   # Cursor Desktop 3.25.9, checked 2026-10-21
```

cross-checked against a live `agent.v1.AgentService/Run` capture. See
`docs/roder-cursor-agent-runtime-protocol.md` for the runtime/bidi protocol
(channels, exec loop, kv acks) that complements this message schema.

## Refreshing after a Cursor update

1. Locate the worker bundle path above (version moves under `app/extensions`).
2. Search the bundle for the generated message descriptors, e.g.
   `rg -o 'SelectedImage[^"\n]*' main.js`. Current compact descriptors use
   `static $(){return["MessageName|1 field_name type|..."]}`; older bundles
   used `proto3.makeMessageType` registrations.
3. Update `agent_v1.proto` and the matching `src/proto.rs` comments/encoders
   together; add/adjust the encode tests in `src/proto.rs`.

## Scope

Covers the messages Roder encodes (`AgentRunRequest`, `UserMessage`,
`ConversationHistory`, …) plus the full inline-image contract
(`SelectedContext` / `SelectedImage` for the live message,
`ConversationHistoryImageContent` for history, and the
`client_supports_inline_images` gate). It is intentionally not the complete
Cursor schema.

Native thread continuity uses `ConversationStateStructure` plus KV GET/PUT blobs
and `PreFetchedBlob`. The older `ConversationHistory` types remain schema
reference material; Roder imports checkpoint-free history explicitly in the
initial prompt because the hosted harness ignored field-7 injection in live
checks. Runtime execution and direct API inference share the same bidi client.
