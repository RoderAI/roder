## 0.1.3 (2026-06-22)

### Features

#### Add first-party Synthetic inference provider

Adds the `synthetic` provider using Synthetic's OpenAI-compatible Chat
Completions API. The provider ships built-in `syn:` model aliases
(`syn:large:text` default, plus `syn:small:text`, `syn:large:vision`,
`syn:small:vision`), preserves concrete `hf:{owner}/{model}` ids across config,
discovery, and selection, and resolves credentials only from
`SYNTHETIC_API_KEY`/`RODER_SYNTHETIC_API_KEY` or `[providers.synthetic]`. The
provider is visible without credentials so app-server and TUI can show setup
state, and turn-time inference fails locally with setup guidance when the key
is missing. The TUI provider menu points to the Synthetic dashboard for API-key
setup instead of the generic fallback URL.

### Fixes

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

### Fixes

- Improve context compaction across phases 2–4: prune old tool outputs before full compaction, add LLM state-snapshot summarization with verify/reject, hysteresis coalescing, `/compact` via `thread/compact`, `context.compaction_skipped` metrics, and a Grok-style loop regression fixture. Phase 1 fixes remain: compaction boundary on load, once-per-turn guard, ProviderMetadata exclusion from token estimates, and suffix retention from the last user message.
- SuperGrok now lists only Grok Build 0.1 (500k context) and Grok Composer 2.5 Fast (200k context), with curated catalog metadata instead of raw xAI /models discovery.

#### Added first-class `kimi-code` (aliases: `kimi`, `moonshot`) inference provider and `roder-ext-kimi-code` crate.

- Kimi Code subscription OAuth uses the managed API (`api.kimi.com/coding/v1`) with Kimi device headers and `kimi-code-cli` User-Agent; API keys still use Moonshot Open Platform (`api.moonshot.ai/v1`).
- Catalog entry + `kimi-for-coding` model (K2.7 Code).
- Device OAuth against `auth.kimi.com` with `roder auth login kimi-code`, TUI/app-server `auth/kimi-code/*`, and token storage under `~/.roder/auth/kimi-code.json`.
- API key fallback via env/config (`KIMI_CODE_API_KEY`, `RODER_KIMI_CODE_API_KEY`).
- Registered via extension host (always available, like SuperGrok).
- Docs: `docs/roder-kimi-code-provider.md`.
- Live smoke test added (opt-in via `RODER_KIMI_CODE_LIVE=1`).

## 0.1.1 (2026-06-15)

### Features

#### First-party image generation providers (OpenAI GPT Image and Google Gemini Nano Banana)

Provider-neutral image generation through the core media API: an image-capable
`MediaGenerationRequest`/multi-output `MediaGenerationResponse` contract, a new
`ProvidedService::MediaGenerator` extension service, a runtime media generation
service backing the canonical `media_generate_image` tool with a deterministic
offline fallback, new `roder-ext-openai-images` (`gpt-image-2` plus legacy ids)
and `roder-ext-google-images` (Nano Banana 2/Pro/base) provider crates,
`[media.image_generation]` config, `media/image/providers/list` and
`media/image/generate` app-server methods, `roder media` CLI commands, palette
entries, and regenerated schemas/SDK stubs. Live provider smokes stay opt-in
behind `RODER_OPENAI_IMAGE_LIVE` / `RODER_GEMINI_IMAGE_LIVE`.

#### One-command Roder package install (`roder install npm:/git:/path`)

Roder packages bundle process extensions, skills, slash commands, and themes
behind a root `roder.toml` manifest. Install from npm, git (shorthand, SSH,
raw URLs, pinned refs), or local paths; manage with `roder packages
list|resources|enable|disable|approve|filter|sync|init`, `roder remove`,
`roder update`, and ephemeral `-e` loading. Resources surface through the
existing skills/commands/theme registries; the process-extension protocol
gains manifest-declared tool providers served over `tools/call`. New
app-server `packages/*` methods, a `/packages` builtin, and a Packages
palette section round out the surfaces. npm lifecycle scripts stay disabled
unless `--allow-scripts` is passed, and package process extensions never
launch before explicit approval.

### Fixes

#### Process-extension protocol 0.2.0 and Cursor SDK remote-agent bridging

Extend the process-extension protocol with subagent-dispatcher and task-executor services, bridge them in the process host, and add app-server e2e coverage for the cursor-sdk-agents TypeScript child.

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
