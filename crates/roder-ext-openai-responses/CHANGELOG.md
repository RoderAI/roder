## 0.2.1 (2026-10-01)

### Features

#### Native OpenAI Responses computer use

Register the native `computer` tool for an explicitly bound CDP browser, execute
ordered action batches with retained per-thread sessions and cancellation
cleanup, and return original-detail `computer_screenshot` observations through
`computer_call_output`. Preserve native call identities through transcript
replay, WebSocket continuation, and ACP tool updates. Include an independent
real-browser protocol eval and a live OpenAI eval runner.

Accept the native API's nullable mouse modifiers. Initialize TLS and large
worker stacks in the live runner, preserve call execution order in its trace,
and independently verify the final browser UI and masked screenshot.

### Fixes

#### Preserve browser observations and execute Desktop input through CDP

Keep screenshot text and original resolution in Responses tool replay, including
when older images are removed to fit the request budget. Attach Chrome extension
screenshots to the model input. Use stable document-scoped refs, real CDP input,
validated typing targets, isolated script state, and explicit action failures in the Desktop fallback.
Isolate browser-use servers by thread, serialize actions with fresh page state,
and stop the owned process tree when an in-flight tool is cancelled. Cover
browser observations and permission rejection through the ACP transport.
Give each browser-use server a private profile and file directories, enforce
optional operator navigation ceilings, and bound agent steps. Preserve Desktop
tab identity across enumeration changes. Release held input and screenshot
masks on cancellation or failure, including sessions retained by their owner.

Cancel a pending Chrome bridge command on dispatch timeout or dropped futures,
and label the paired extension action observations as untrusted page data.

Support optional caller-defined Jev completion conditions checked against fresh
UI state. Reject premature model DONE and permit bounded fallback recovery;
report unverified model completion explicitly when no conditions were supplied.

#### Validate native browser registration and recover failed input cleanup

Declare the native computer tool in Chrome's extension manifest, reserve its
name from external function tools, and reconcile cancelled cached sessions.
Retry failed input releases before reusing a session and attempt all key, mouse,
and screenshot-mask cleanup independently. Scrub overlapping secrets and every
nested element observation. Reject malformed Desktop origin and tab settings,
check origin scope before resolving targets, and disable arbitrary eval under
an origin ceiling. Honor the configured eval provider endpoint, validate native
compaction, preserve generic image-reopen guidance, and accept documented empty
Jev completion conditions and sparse serialized controls.

## 0.2.0 (2026-09-26)

### Breaking Changes

#### Release the Responses loop and Codex patch parity improvements as Roder 0.2, including dependent crates built against the new shared API. Isolate config-dependent tests from process environment and saved authentication. Update shell-include coverage to the current Plan process policy.

Breaking change: apply_patch accepts only the canonical patch argument and Codex patch syntax. Crate consumers must rebuild against the new shared API versions.

### Fixes

#### Align apply_patch parsing, ordered line matching, custom-tool grammar, model routing, and streamed input with Codex. Use one canonical patch argument and Codex patch syntax for local and remote tools. Fix split UTF-8 SSE decoding, terminal completion, client tool-search continuation contracts and exhaustion, replay, pooled HTTP transport, and Retry-After handling. Retry transient sampling failures across runtime profiles, stop on terminal completion, reject silent EOF, preempt sampling on steering, and emit terminal failures once.

Persist completed messages, reasoning, search exchanges and tool effects during sampling. Execute opted-in reads eagerly through normal policy and authorization checks. Add task-scoped WebSocket pooling with verified delta continuation, interruption invalidation and HTTP fallback. Add provider-native, task-aware compaction, complete opaque-window replay, request byte/image guards, failure identifiers, streamed proposed patch progress, actual filesystem diffs and uncertainty-aware partial outcomes and rollback.

Require canonical completion from mock tool responses and keep sampling steering separate from turn cancellation, including immediate-interrupt races. Keep manual compaction and new input atomic for its target task. Prevent concurrent eager reads from deadlocking streamed item persistence. Default new evaluation runs to GPT-6-luna.

## 0.1.11 (2026-09-26)

### Fixes

#### Publish today's content under fresh crate versions

`roder-api` 0.1.21, `roder-core` 0.1.19, `roder-app-server` 0.1.17,
`roder-evals` 0.1.2 and `roder-ext-openai-responses` 0.1.10 were published to
crates.io on 2026-09-23 from a branch that predates the Codex agent backend,
the Jev browser tool, the Chrome bridge work and the Grok 4.7 catalog entries.
crates.io versions are immutable, so those numbers cannot carry the current
code and the registry copies do not match the git tags of the same version.

Bump them so the released content reaches crates.io under versions that
describe it. `roder-ext-jev` and `roder-ext-codex-backend` are bumped with
them: both are new crates whose first publish must depend on a registry
`roder-api` that actually contains `roder_api::backend`, which the 0.1.21
registry copy does not.

Adds the package-local READMEs both new crates need to publish.

## 0.1.10 (2026-09-26)

### Features

#### Per-turn OpenAI service tier (Fast mode), and GPT-6 Sol and Luna

A caller can now choose the OpenAI service tier for a turn.
`StartTurnRequest::service_tier_override` (for example `"priority"` for Fast
mode) is carried to every inference round of the turn as
`RuntimeHints::service_tier`. The OpenAI Responses provider sends it as the
top-level `service_tier` request field only on the OpenAI profile; OpenRouter,
xAI, and Fireworks never receive it. `None` keeps the provider default.

The tier OpenAI reports it actually served (`response.service_tier`) is recorded
on `TokenUsage::service_tier`, so a biller can tell a request that ran fast from
one that was downgraded to `"default"` under load. Both new fields are optional
and default to absent when older payloads are deserialized.

`StartTurnRequest` gains a public field, so code that builds it with a struct
literal must add `service_tier_override: None`.

The OpenAI/Codex catalog adds `gpt-6-sol` (efforts `low` through `max`, like
`gpt-6-astra`) and `gpt-6-luna` (efforts `low` through `max`, like
`gpt-5.6-luna`), so per-turn reasoning validation accepts them.

### Fixes

#### Select Codex as a complete agent backend for Roder

Add an extension service for complete agent backends and a Codex app-server implementation. Roder app-server maps its thread, turn, streaming event, and approval APIs to Codex, so existing Roder clients and the TUI can use Codex as the agent runtime.

Refresh GPT-6 and Claude Opus 5.5/Sonnet 5 catalog entries and map Codex token usage into Roder's turn counters.

## 0.1.9 (2026-09-12)

### Fixes

#### Add Gemini 3.7 Flash and Grok 4.6 to provider model catalogs

Expose Gemini 3.7 Flash and Grok 4.6 through native, Cursor, xAI, SuperGrok,
and OpenRouter integrations with provider-specific context windows and
reasoning controls. Retire Grok 4.5 and Grok Build from active catalogs.

## 0.1.8 (2026-08-20)

### Fixes

#### Explain streamed response failures

Include the response read failure kind, configured idle timeout, safe provider request identifier, and bounded cause chain when a Responses stream fails.

## 0.1.7 (2026-08-05)

### Fixes

#### Fixes

##### Include developer/ultra policy in OpenAI and xAI Responses `instructions`

Join stable `system` + `developer` into the Responses top-level `instructions`
field for OpenAI, Codex, xAI, and SuperGrok so ultra-mode multi-agent policy,
plan mode, goals, and other developer-slot addenda actually reach the model.
Keep per-turn `developer_context` as a leading input message outside the
stable prefix.

## 0.1.6 (2026-07-21)

### Features

#### Freeform apply_patch on the Responses custom-tool channel

Advertise `apply_patch` on the OpenAI Responses freeform/custom tool channel
(`type:"custom"`) for the gpt-5.5 family, matching the channel the model was
RL-trained to emit patches on. `ToolSpec` gains a `freeform_input_field` marker
(default `None`, so ordinary function tools are unchanged); the Responses
provider serializes marked tools as `type:"custom"`, parses `custom_tool_call`
outputs into the normal tool-dispatch path, and replays their results as
`custom_tool_call_output`. Non-gpt-5.5 models and every other provider keep the
JSON `type:"function"` shape. The `apply_patch` handler accepts both the JSON
`{ "patch": ... }` arguments and the raw freeform body.

#### Path-based `view_image` tool for vision tasks

Adds a native `view_image(path)` tool that mirrors Codex's semantics: it reads
an image file (png/jpeg/gif/webp, validated by magic bytes, capped at 10 MiB),
base64-encodes it, and returns it as an image content block in the tool result
so the model sees the pixels. It reads through the workspace backend, so it
works against both local and remote-runner workspaces.

- `roder-tools`: new `view_image` tool (registered alongside the builtin coding
  tools); a `read_bytes` method on the workspace backend for binary reads; and
  `media_attach` now degrades to actionable guidance (pointing at `view_image`)
  instead of hard-failing when called without raw base64 bytes, so it no longer
  burns the consecutive-tool-failure budget in headless/eval runs.
- `roder-api`: `VIEW_IMAGE_DISPLAY_KEY`, a reserved `display_payload` key that
  carries the image block from tool result to provider.
- `roder-ext-openai-responses`: `function_call_output` now forwards a
  `view_image` result as an `input_image` content block (when the model
  supports images), falling back to the plain string output otherwise.

### Fixes

#### Fix provider compaction thrashing and show token/duration summaries

Persist OpenAI/Codex compaction items as soon as the stream emits them so a
later SSE decode failure cannot drop the boundary and re-compact every round.
Surface before/after estimated tokens and elapsed time in the TUI and
app-server item stream.

#### Honor provider compaction boundaries

Drop pre-compaction history after OpenAI server-side compaction items so long sessions no longer re-send and re-compact the full window every request. Treat provider compaction as a local transcript boundary, and map Anthropic local context summaries correctly when the emergency client path runs.

## 0.1.5 (2026-07-09)

### Fixes

#### Add GPT-5.6 Codex models and Ultra mode

Expose GPT-5.6 Sol, Terra, and Luna plus GPT-5.4 in the OpenAI and Codex
catalogs, with the current context windows, defaults, and reasoning-effort
menus. Make Sol the default Codex model.

Keep Ultra as a first-class Roder effort for Sol and Terra while mapping it to
the provider's `max` wire effort. Ultra enables proactive, bounded multi-agent
delegation; lower Sol and Terra efforts remain explicit-request-only.

## 0.1.4 (2026-07-09)

### Fixes

#### Add Grok 4.5 to xAI and SuperGrok providers

Expose `grok-4.5` (500k context, default high reasoning, low/medium/high) as the
default model for both the `xai` API-key provider and SuperGrok OAuth. Keep
legacy Grok 4.3 / 4.20 and SuperGrok Build/Composer entries selectable.

## 0.1.3 (2026-06-22)

### Fixes

- Stabilize Roder startup, streaming responses, and provider behavior

#### Fix grok-composer-2.5-fast image input handling

The `grok-composer-2.5-fast` model does not support image inputs, but Roder's catalog
hardcoded `supports_images: true` for all xAI/SuperGrok models. The `xai_model` macro now
takes a `supports_images` parameter so non-vision models can correctly declare their
capabilities.

The OpenAI Responses provider engine now checks the model's `supports_images` flag before
emitting `input_image` content items in request payloads. This prevents the xAI API error:

  "Image inputs are not supported by this model."

`grok-composer-2.5-fast` is set to `supports_images: false`; all other Grok models keep
their previous `true` value.

## 0.1.2 (2026-06-16)

### Features

#### Fireworks AI inference provider

Add the first-party `fireworks` inference provider with account-scoped model ids, Fireworks-specific API-key configuration, OpenAI-compatible Responses transport, offline model metadata, model discovery, and app-server provider-list coverage.

## 0.1.1 (2026-06-15)

### Fixes

#### Fix xAI/SuperGrok Responses 400 on hosted web search

When using the xAI or SuperGrok provider with hosted web search enabled (cached or live), the Responses mapper was unconditionally emitting `"external_web_access"` on the `web_search` tool object. xAI's backend rejects this key with:

  Argument not supported: external_web_access

Now, for `ResponsesProviderProfile::Xai` (both direct `xai` key and `supergrok` OAuth), we emit a plain `{"type": "web_search"}` tool (the `external_web_access` flag is only sent for OpenAI/OpenRouter profiles that understand it).

The web search tool is still included when the runtime requests hosted web search, so Grok's native search should activate as before.

Updated an Xai profile mapping test to assert the key is omitted.

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.

#### SuperGrok: default to grok-build-0.1, add it to catalog, enable live /models discovery

- Change SuperGrok provider default_model to `grok-build-0.1`.
- Add `grok-build-0.1` (Grok Build) model entry under the `supergrok` provider (rich xAI capabilities: tools, structured, images, configurable reasoning; 256k ctx).
- `SuperGrokEngine::list_models` now plugs into the shared OpenAI-compatible `/models` + `/v1/models` discovery (using the live SuperGrok OAuth access token for Bearer auth). It uses the standard `~/.roder/models-cache.json` (respects RODER_MODELS_* envs for TTL/refresh/path), background refresh on stale, and falls back to the (now updated) static catalog on no-auth or error. This lets Roder surface the latest models and (basic) capabilities from xAI for SuperGrok subscribers without requiring Roder releases.
- Exposed the reusable `discover_models`, `cached_models`, `save_cached_models`, `cache_ttl`, `force_refresh_requested`, and `CachedProviderModels` from `roder-ext-openai-responses` (pub) so other xAI-flavored paths can reuse.
- Updated tests, docs, and examples to reference `grok-build-0.1` for SuperGrok. (Composer 2.5 remains a Cursor-native model.)
- Live validation with real SuperGrok token confirms `/models` returns (among others) `grok-build-0.1` + current Grok variants.
