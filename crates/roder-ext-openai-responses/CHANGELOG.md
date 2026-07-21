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
