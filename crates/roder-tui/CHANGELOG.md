## 0.1.6 (2026-06-26)

### Fixes

#### Fix needing a second Ctrl+C to exit a live Claude Code session

Interrupt any in-flight turn during TUI teardown and exit the process cleanly
once the terminal is restored, so a single Ctrl+C fully exits even when an
in-process provider (e.g. Claude Code) has spawned a CLI subprocess whose
runtime tasks would otherwise block shutdown.

## 0.1.5 (2026-06-25)

### Features

#### Render an interactive selection dialog for `request_user_input`

The TUI previously only logged a system line when the model called the
interactive `request_user_input` survey tool, so the question and its options
were never shown and the blocked turn appeared to hang. The TUI now opens a
modal selection dialog listing each option with its description, lets you
navigate with the arrow keys (or `Ctrl+J`/`Ctrl+K`), jump with number keys
`1`-`9`, confirm with `Enter`, and skip with `Esc`. Confirming sends
`thread/resolve_user_input` with the chosen option label keyed by question id;
multi-question surveys are answered one at a time and accumulated before
resolving. The turn timer pauses while the dialog is open and resumes once it is
resolved, and a survey with no answerable options resolves immediately so the
turn never hangs.

The local mock inference provider now also drives `request_user_input` when a
user message contains `FAKE_REQUEST_USER_INPUT`, so the selection dialog can be
exercised end to end without a live provider.

## 0.1.4 (2026-06-24)

### Features

#### Manage external web-search providers from the TUI

Extend the "Web search provider" settings submenu to list the external provider
router options (firecrawl, tavily, perplexity, parallel, synthetic) alongside
the hosted modes, showing each provider's enabled and API-key-configured status
and the active selection. Selecting an external provider persists
`[web_search] mode = "external"`, the chosen `provider`, and enables that
provider's sub-section in user config (applies on restart).

The same hosted modes and external providers are also selectable from the
`Ctrl+P` command palette, and both the menu and palette fall back to reading
providers directly from user config when app-server settings are unavailable.

`settings/get` and `settings/set_web_search` now report the external-router
snapshot via `WebSearchSettings { external_enabled, external_provider, providers }`
and accept an optional `external_provider` selection. New `roder-config`
helpers (`save_web_search_external_provider`, `WebSearchConfig::provider_configured`,
`WebSearchConfig::router_snapshot`, `web_search_router_snapshot`) back the
persistence, status checks, and config-only fallback.

Synthetic web search now auto-configures from the synthetic inference provider:
because both share `SYNTHETIC_API_KEY`, pasting the synthetic provider key
(`providers/configure`) makes the synthetic search provider report as
`key configured`, enables its `[web_search.synthetic]` sub-section, and lets it
resolve the borrowed key at runtime — no separate web-search key entry needed.

## 0.1.3 (2026-06-22)

### Features

#### Show full provider error response body in the timeline detail popup

Chat Completions provider errors (e.g. Synthetic 502 Bad Gateway) previously
discarded the response body entirely and showed only a generic hint like
"provider server error; response body redacted". Now the error includes the
full response body after a separator, so clicking the error row (or pressing
Enter on it) opens the existing tool-detail popup with the complete provider
response text — the same popup used for shell output and file edits.

The auth credential (bearer token or api-key header value) is scrubbed from
the body before it enters the error message, so the popup is safe to share.
Bodies are capped at 4 KB to avoid unbounded error messages.

The TUI also makes `Error` timeline items selectable and clickable, routes
Enter on a selected error row to the detail modal, and renders the popup with
an "error details" title and "Response body" label when the source is a
provider error rather than a shell command.

### Fixes

- Stabilize Roder startup, streaming responses, and provider behavior

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

## 0.1.2 (2026-06-16)

### Fixes

- Improve context compaction across phases 2–4: prune old tool outputs before full compaction, add LLM state-snapshot summarization with verify/reject, hysteresis coalescing, `/compact` via `thread/compact`, `context.compaction_skipped` metrics, and a Grok-style loop regression fixture. Phase 1 fixes remain: compaction boundary on load, once-per-turn guard, ProviderMetadata exclusion from token estimates, and suffix retention from the last user message.

## 0.1.1 (2026-06-15)

### Features

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

#### Roadmap orchestration dashboard and multi-worker fan-out

Redesign the roadmap TUI workspace as an orchestration dashboard with progress
header, status strip, tree-style worker rows, and windowed scrolling. Add
orchestrator prompt rules in `roder-roadmap` and fan-out controls in the TUI:
`S` spawns up to eight workers across ready tasks and `s` spawns one for the
focused task.

### Fixes

- Put Models first in the Ctrl+P menu

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

#### Package-specific registry READMEs

Add package-specific README files for every Cargo crate, ensure npm and PyPI package READMEs link to roder.sh, and tighten the registry README verifier to require package-local documentation.

#### Registry README metadata and publish checklists

Ensure Cargo crates inherit the workspace README, document npm and PyPI publishing steps in package READMEs, and add a registry README verifier for future publishes.
