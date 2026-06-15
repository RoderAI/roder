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
