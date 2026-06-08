# Roder Extension API

Roder's public extension API is the `roder-api` crate. Extensions register through `RoderExtension`, declare an `ExtensionManifest`, and install services through `ExtensionRegistryBuilder`.

App clients inspect the installed surface with `extensions/list`. The response includes manifests and `capability_statuses`, keyed by extension id, so clients can show requested, granted, and denied capabilities without reading runtime internals.

## Service Categories

Extensions advertise installed services through `ProvidedService` and install
the matching runtime objects through `ExtensionRegistryBuilder`.

- `InferenceEngine` owns transport to a model provider and streams real model
  responses.
- `InferenceRouter` inspects bounded runtime context and recommends a
  provider/model/reasoning selection before an inference request. Routers do not
  proxy provider traffic and should return `abstain` when they cannot make a
  safe local decision.
- `ToolProvider` contributes model-visible tools.
- Other provider categories cover thread stores, notifications, runners,
  context providers, retrieval, speech, TUI surfaces, version control, and
  extension-owned state.

The canonical routing trait is `roder_api::InferenceRouter`. Its context
includes default selection, runtime profile, phase, bounded transcript/tool
summaries, candidate model descriptors, prior failure/escalation counters, and
local signals. Core validates router selections against registered providers,
auth availability, model descriptors, image support, tool-call support, context
window, and reasoning support before applying them.

## Capability Boundaries

Extensions must declare sensitive access in `required_capabilities`. Registry construction records each request and rejects incompatible manifests before runtime starts. Capability ids should be stable and action-oriented, for example:

- `fs.read.workspace`
- `fs.write.workspace`
- `process.spawn.shell`
- `network.web`
- `network.api.openai.com`
- `secret.read.OPENAI_API_KEY`
- `desktop.notification`

Runtime tools receive scoped handles through `ToolExecutionContext`. File and process tools must fail when the matching handle is absent instead of reconstructing local authority from global paths or process state.

## Dynamic Payloads

`serde_json::Value` is allowed only inside a typed envelope that says who owns the payload and how clients should treat it. Current approved envelopes are:

- `AgentInferenceRequest.metadata`: caller-owned request metadata for inference providers.
- `InferenceEvent::ProviderMetadata`: provider-owned response metadata.
- `ToolResult.data`: tool-owned structured result data.
- `JsonRpcError.data`: protocol-owned structured error details.
- `ExtensionStateRecord.value`: extension-owned opaque state payload.

Do not add provider-specific fields to canonical request, event, or protocol structs unless the behavior is general across providers. Product-specific features should use provider metadata, tool result data, or a new typed extension record with an explicit owner and schema version.

`RuntimeHints.hosted_web_search` is a canonical Roder hint, not an OpenAI-only field: it asks a capable inference provider to use its own hosted search path when available. Providers that cannot support the hint should ignore it and leave web search to installed tools.

## State

Extension state is persisted as `ExtensionStateRecord` values and surfaced through `ThreadSnapshot.extension_states`. The host stores extension id, key, scope, schema version, and payload, but decoding and migration stay with the extension's `ExtensionStateCodec`.
